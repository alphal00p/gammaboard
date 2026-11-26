//! Offline database maintenance, executed by the installed service binary.
use super::{
    db::{DbArgs, DbCommand},
    instance::Instance,
    service as local, shared,
};
use anyhow::{Context, Result, ensure};
use sqlx::Connection;
use std::{
    fs,
    io::{IsTerminal, Write},
    process::Command,
};
pub fn confirm(yes: bool, message: &str) -> Result<()> {
    if yes {
        return Ok(());
    }
    ensure!(
        std::io::stdin().is_terminal(),
        "{message} Pass --yes in non-interactive mode."
    );
    eprint!("{message} [y/N] ");
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    ensure!(
        matches!(line.trim().to_lowercase().as_str(), "y" | "yes"),
        "aborted"
    );
    Ok(())
}
fn command(command: &mut Command, label: &str) -> Result<()> {
    ensure!(
        command
            .status()
            .with_context(|| format!("cannot run {label}"))?
            .success(),
        "{label} failed"
    );
    Ok(())
}
pub async fn database(instance: &Instance, args: DbArgs) -> Result<()> {
    let url = &instance.runtime.database.url;
    match args.command {
        DbCommand::Stop => local::stop_database(instance),
        DbCommand::Backup { output } => {
            let output = super::instance::resolve(&std::env::current_dir()?, &output)?;
            ensure!(
                !output.exists(),
                "backup already exists: {}",
                output.display()
            );
            fs::create_dir_all(output.parent().context("backup needs parent")?)?;
            command(
                Command::new("pg_dump")
                    .args(["--format=custom", "--file"])
                    .arg(&output)
                    .arg(url),
                "backup",
            )?;
            shared::print_json(&serde_json::json!({"output":output}));
            Ok(())
        }
        DbCommand::Status => {
            let connection = tokio::time::timeout(
                std::time::Duration::from_secs(3),
                sqlx::PgConnection::connect(url),
            )
            .await;
            let mut value =
                serde_json::json!({"managed":instance.managed_database,"reachable":false});
            if let Ok(Ok(mut conn)) = connection {
                value["reachable"] = true.into();
                value["model_version"] = serde_json::to_value(
                    sqlx::query_scalar::<_, i32>(
                        "SELECT model_version FROM gammaboard_compatibility WHERE singleton=true",
                    )
                    .fetch_optional(&mut conn)
                    .await
                    .ok()
                    .flatten(),
                )?;
            }
            if instance.managed_database {
                value["data_dir"] = instance.runtime.local_postgres.data_dir.clone().into();
            }
            shared::print_json(&value);
            Ok(())
        }
        operation => {
            fs::create_dir_all(&instance.paths.state_dir)?;
            let _guard = local::lock(&instance.paths.state_dir.join("service.lock"))
                .context("stop the instance before database maintenance")?;
            let db_guard = if instance.managed_database {
                let data = std::path::Path::new(&instance.runtime.local_postgres.data_dir);
                fs::create_dir_all(data.parent().context("database directory has no parent")?)?;
                Some(local::lock(&data.with_extension("gammaboard.lock"))?)
            } else {
                None
            };
            let _db_guard = db_guard;
            if let DbCommand::Purge { yes } = operation {
                ensure!(
                    instance.managed_database,
                    "cannot purge an external PostgreSQL cluster"
                );
                let local = &instance.runtime.local_postgres;
                let data = std::path::Path::new(&local.data_dir);
                ensure!(
                    !data.exists()
                        || data.join("PG_VERSION").is_file()
                        || fs::read_dir(data)?.next().is_none(),
                    "refusing to purge a non-PostgreSQL directory: {}",
                    data.display()
                );
                confirm(
                    yes,
                    &format!("Delete the complete PostgreSQL cluster {}?", data.display()),
                )?;
                if gammaboard::local_db::is_db_running(local)? {
                    let mut connection = sqlx::PgConnection::connect(url).await?;
                    let active:i64=sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE backend_type='client backend' AND pid<>pg_backend_pid()").fetch_one(&mut connection).await?;
                    ensure!(
                        active == 0,
                        "cluster has active connections; stop servers and workers first"
                    );
                    connection.close().await?;
                }
                gammaboard::local_db::stop_instance_database(local)?;
                if data.exists() {
                    fs::remove_dir_all(data)?;
                }
                // PostgreSQL removes its sockets. Never recursively remove a potentially shared socket directory.
                let socket = std::path::Path::new(&local.socket_dir);
                if socket.is_dir() && fs::read_dir(socket)?.next().is_none() {
                    fs::remove_dir(socket)?;
                }
                archive_export_state(instance)?;
                shared::print_json(
                    &serde_json::json!({"purged":instance.runtime.local_postgres.data_dir}),
                );
                return Ok(());
            }
            let mut target = url::Url::parse(url)?;
            let name = target.path().trim_start_matches('/').to_string();
            ensure!(
                !name.is_empty()
                    && !matches!(name.as_str(), "postgres" | "template0" | "template1"),
                "refusing maintenance of a PostgreSQL system database"
            );
            let yes = match &operation {
                DbCommand::Delete { yes }
                | DbCommand::Reset { yes }
                | DbCommand::Restore { yes, .. } => Some(*yes),
                _ => None,
            };
            if let Some(yes) = yes {
                confirm(
                    yes,
                    &format!(
                        "Replace/delete database '{name}' for instance '{}' at {}:{}?",
                        instance.name,
                        target.host_str().unwrap_or("local"),
                        target.port().unwrap_or(5432)
                    ),
                )?;
            }
            if let DbCommand::Restore { backup, .. } = &operation {
                ensure!(backup.is_file(), "backup not found: {}", backup.display());
            }
            let was_running = if instance.managed_database {
                gammaboard::local_db::is_db_running(&instance.runtime.local_postgres)?
            } else {
                true
            };
            if instance.managed_database {
                gammaboard::local_db::start_postgres_cluster(
                    &instance.runtime.local_postgres,
                    url,
                )?;
            }
            let replaces_data = matches!(
                &operation,
                DbCommand::Delete { .. } | DbCommand::Reset { .. } | DbCommand::Restore { .. }
            );
            let result: Result<()> = async {
                target.set_path("postgres");
                let mut admin=sqlx::PgConnection::connect(target.as_str()).await?;
                let active:i64=sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE datname=$1 AND pid<>pg_backend_pid()").bind(&name).fetch_one(&mut admin).await?;
                ensure!(active==0,"database has {active} active connections; stop servers and workers first");
                let quoted=format!("\"{}\"",name.replace('"',"\"\""));
                let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname=$1)").bind(&name).fetch_one(&mut admin).await?;
                match &operation {
                    DbCommand::Delete {..}|DbCommand::Reset {..} if exists => { sqlx::query(&format!("DROP DATABASE {quoted}")).execute(&mut admin).await?; }
                    _ => {}
                }
                if matches!(operation, DbCommand::Create|DbCommand::Reset {..}) && (!exists || matches!(operation,DbCommand::Reset {..})) { sqlx::query(&format!("CREATE DATABASE {quoted}")).execute(&mut admin).await?; }
                match operation {
                    DbCommand::Create|DbCommand::Reset {..}|DbCommand::Migrate => gammaboard::local_db::migrate_instance(url).await?,
                    DbCommand::Restore {backup,..} => {
                        ensure!(exists,"database does not exist; run db create first");
                        let mut conn=sqlx::PgConnection::connect(url).await?;
                        sqlx::raw_sql("DROP SCHEMA public CASCADE; CREATE SCHEMA public;").execute(&mut conn).await?; conn.close().await?;
                        command(Command::new("pg_restore").args(["--no-owner","--exit-on-error","--dbname",url]).arg(backup),"restore")?; gammaboard::local_db::migrate_instance(url).await?;
                    }
                    DbCommand::Delete {..} => {},
                    _=>unreachable!(),
                }
                Ok(())
            }.await;
            if instance.managed_database && !was_running {
                gammaboard::local_db::stop_instance_database(&instance.runtime.local_postgres)?;
            }
            result?;
            if replaces_data {
                archive_export_state(instance)?;
            }
            shared::print_json(&serde_json::json!({"success":true,"database":name}));
            Ok(())
        }
    }
}

fn archive_export_state(instance: &Instance) -> Result<()> {
    let state = instance.paths.state_dir.join("state.json");
    if state.exists() {
        fs::rename(
            &state,
            instance
                .paths
                .state_dir
                .join(format!("state-before-reset-{}.json", uuid::Uuid::new_v4())),
        )?;
    }
    Ok(())
}
