use anyhow::Result;
use clap::{Args, Subcommand};
use gammaboard::config::RuntimeConfig;
use gammaboard::local_db;

#[derive(Debug, Args)]
pub struct DbArgs {
    #[command(subcommand)]
    pub command: DbCommand,
}

#[derive(Debug, Subcommand)]
pub enum DbCommand {
    /// Show local Postgres cluster status
    Status,
    /// Start local Postgres and run migrations
    Start {
        /// Start PostgreSQL without creating or migrating the configured database
        #[arg(long)]
        skip_migrations: bool,
    },
    /// Stop local Postgres
    Stop,
    /// Delete the local Postgres data directory
    Delete {
        #[arg(short = 'y', long, action = clap::ArgAction::SetTrue)]
        yes: bool,
    },
    /// Dump the configured database as SQL
    DumpSql,
    /// Recreate the local Postgres cluster and run migrations
    Reset {
        #[arg(short = 'y', long, action = clap::ArgAction::SetTrue)]
        yes: bool,
    },
}

pub fn run_db_command(args: DbArgs, config: &RuntimeConfig) -> Result<()> {
    let local = &config.local_postgres;
    match args.command {
        DbCommand::Status => local_db::status_db(local, &config.database.url),
        DbCommand::Start { skip_migrations } if skip_migrations => {
            local_db::start_postgres_cluster(local, &config.database.url)
        }
        DbCommand::Start { .. } => local_db::start_db(local, &config.database.url),
        DbCommand::Stop => local_db::stop_db(local),
        DbCommand::Delete { yes } => local_db::delete_db(local, yes),
        DbCommand::DumpSql => local_db::dump_db_sql(local, &config.database.url),
        DbCommand::Reset { yes } => local_db::reset_db(local, yes, &config.database.url),
    }
}

pub(crate) use local_db::{require_commands, start_db, stop_db};
