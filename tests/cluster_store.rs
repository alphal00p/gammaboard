//! Durable external submission and direct-database build fencing contracts.
use gammaboard::{PgStore, core::ControlPlaneStore};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};

struct Database {
    pool: PgPool,
    admin: PgPool,
    name: String,
    store: PgStore,
}
impl Database {
    async fn new() -> anyhow::Result<Self> {
        let mut url = url::Url::parse(&std::env::var("GAMMABOARD_TEST_DATABASE_URL")?)?;
        url.set_path("/postgres");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(url.as_str())
            .await?;
        let name = format!("gammaboard_cluster_{}", uuid::Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE DATABASE {name}"))
            .execute(&admin)
            .await?;
        url.set_path(&name);
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect(url.as_str())
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        let store = PgStore::new(pool.clone());
        Ok(Self {
            pool,
            admin,
            name,
            store,
        })
    }
    async fn finish(self) -> anyhow::Result<()> {
        self.pool.close().await;
        sqlx::query(&format!("DROP DATABASE {} WITH (FORCE)", self.name))
            .execute(&self.admin)
            .await?;
        self.admin.close().await;
        Ok(())
    }
    async fn request(&self, count: i32) -> anyhow::Result<i64> {
        Ok(self.store.reserve_worker_launch("external",vec![json!({"count":count,"name_prefix":"worker","config":{},"capabilities":{},"max_start_failures":3})]).await?)
    }
}
fn token(claim: &Value) -> &str {
    claim["lease_token"].as_str().unwrap()
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL with CREATE DATABASE permission"]
async fn submission_intent_survives_lease_takeover_and_stale_writers_are_fenced()
-> anyhow::Result<()> {
    let db = Database::new().await?;
    let id = db.request(2).await?;
    let first = db
        .store
        .claim_external_launch("first", None)
        .await?
        .unwrap();
    assert!(
        db.store
            .claim_external_launch("second", None)
            .await?
            .is_none()
    );
    db.store
        .update_external_worker(id, token(&first), "worker-1", "submitting", None, None)
        .await?;
    sqlx::query("UPDATE node_launch_requests SET launcher_lease_expires_at=now()-interval '1 second' WHERE id=$1").bind(id).execute(&db.pool).await?;
    let second = db
        .store
        .claim_external_launch("second", Some(id))
        .await?
        .unwrap();
    assert_ne!(token(&first), token(&second));
    assert_eq!(
        first["workers"][0]["submission_key"],
        second["workers"][0]["submission_key"]
    );
    assert_eq!(second["workers"][0]["state"], "submitting");
    assert!(
        db.store
            .update_external_worker(
                id,
                token(&first),
                "worker-1",
                "submitted",
                Some("101"),
                None
            )
            .await
            .is_err()
    );
    assert!(
        db.store
            .heartbeat_external_launch(id, token(&first), false)
            .await
            .is_err()
    );
    db.store
        .update_external_worker(
            id,
            token(&second),
            "worker-1",
            "submitted",
            Some("101"),
            None,
        )
        .await?;
    assert!(
        db.store
            .update_external_worker(
                id,
                token(&second),
                "worker-1",
                "submitted",
                Some("999"),
                None
            )
            .await
            .is_err()
    );
    assert!(
        db.store
            .update_external_worker(id, token(&second), "worker-1", "connected", None, None)
            .await
            .is_err()
    );
    db.store
        .announce_node("worker-1", "uuid-1", &Default::default())
        .await?;
    db.store
        .update_external_worker(
            id,
            token(&second),
            "worker-2",
            "failed",
            None,
            Some("scheduler rejected worker"),
        )
        .await?;
    let state: String = sqlx::query_scalar("SELECT state FROM node_launch_requests WHERE id=$1")
        .bind(id)
        .fetch_one(&db.pool)
        .await?;
    assert_eq!(state, "failed");
    assert!(
        db.store
            .claim_external_launch("third", None)
            .await?
            .is_none()
    );
    db.finish().await
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL with CREATE DATABASE permission"]
async fn registration_finishes_a_lost_submission_receipt_and_accepts_its_late_job_id()
-> anyhow::Result<()> {
    let db = Database::new().await?;
    let id = db.request(1).await?;
    let claim = db
        .store
        .claim_external_launch("launcher", None)
        .await?
        .unwrap();
    db.store
        .update_external_worker(id, token(&claim), "worker-1", "submitting", None, None)
        .await?;
    db.store
        .announce_node("worker-1", "uuid", &Default::default())
        .await?;
    let state: String = sqlx::query_scalar("SELECT state FROM node_launch_requests WHERE id=$1")
        .bind(id)
        .fetch_one(&db.pool)
        .await?;
    assert_eq!(state, "fulfilled");
    db.store
        .update_external_worker(
            id,
            token(&claim),
            "worker-1",
            "submitted",
            Some("101"),
            None,
        )
        .await?;
    let row: (String, Option<String>) =
        sqlx::query_as("SELECT state,job_id FROM external_worker_launches WHERE request_id=$1")
            .bind(id)
            .fetch_one(&db.pool)
            .await?;
    assert_eq!(row, ("connected".into(), Some("101".into())));
    db.finish().await
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL with CREATE DATABASE permission"]
async fn failed_worker_releases_capacity_while_other_workers_are_still_pending()
-> anyhow::Result<()> {
    let db = Database::new().await?;
    let id = db.request(2).await?;
    let claim = db
        .store
        .claim_external_launch("launcher", None)
        .await?
        .unwrap();
    db.store
        .update_external_worker(
            id,
            token(&claim),
            "worker-1",
            "failed",
            None,
            Some("invalid scheduler resources"),
        )
        .await?;
    let state: String = sqlx::query_scalar("SELECT state FROM node_launch_requests WHERE id=$1")
        .bind(id)
        .fetch_one(&db.pool)
        .await?;
    assert_eq!(state, "starting");
    let allowance:i32=sqlx::query_scalar("SELECT (current_setting('max_connections')::int-current_setting('superuser_reserved_connections')::int-COALESCE(current_setting('reserved_connections',true)::int,0)-16)/4")
        .fetch_one(&db.pool).await?;
    // Only worker-2 still holds an unregistered reservation.
    db.request(allowance - 1).await?;
    assert!(db.request(1).await.is_err());
    db.finish().await
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL with CREATE DATABASE permission"]
async fn database_rejects_old_or_wrong_worker_builds_and_live_deployment_replacement()
-> anyhow::Result<()> {
    let db = Database::new().await?;
    db.store.bind_deployment(Some("deployment-a")).await?;
    assert!(db.store.bind_deployment(None).await.is_err());
    assert!(sqlx::query("INSERT INTO nodes(name,uuid,lease_expires_at) VALUES('old','old',now()+interval '1 hour')").execute(&db.pool).await.is_err());
    assert!(sqlx::query("INSERT INTO nodes(name,uuid,lease_expires_at,deployment_id,build_id) VALUES('wrong','wrong',now()+interval '1 hour','deployment-a','old-build')").execute(&db.pool).await.is_err());
    sqlx::query("INSERT INTO nodes(name,uuid,lease_expires_at,deployment_id,build_id) VALUES('right','right',now()+interval '1 hour','deployment-a',$1)")
        .bind(gammaboard::provenance::build_identity()).execute(&db.pool).await?;
    assert!(
        db.store
            .bind_deployment(Some("deployment-b"))
            .await
            .is_err()
    );
    let request = db.request(1).await?;
    let claim = db.store.claim_external_launch("a", None).await?.unwrap();
    sqlx::query("UPDATE nodes SET lease_expires_at=now()-interval '1 second' WHERE name='right'")
        .execute(&db.pool)
        .await?;
    db.store.bind_deployment(Some("deployment-b")).await?;
    assert!(
        sqlx::query("UPDATE nodes SET lease_expires_at=now()+interval '1 hour' WHERE name='right'")
            .execute(&db.pool)
            .await
            .is_err()
    );
    assert!(
        db.store
            .update_external_worker(request, token(&claim), "worker-1", "submitting", None, None)
            .await
            .is_err()
    );
    let state: String = sqlx::query_scalar("SELECT state FROM node_launch_requests WHERE id=$1")
        .bind(request)
        .fetch_one(&db.pool)
        .await?;
    assert_eq!(state, "canceled");
    db.finish().await
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL with CREATE DATABASE permission"]
async fn registration_and_deployment_replacement_are_serialized() -> anyhow::Result<()> {
    let db = Database::new().await?;
    db.store.bind_deployment(Some("a")).await?;
    let mut registration = db.pool.begin().await?;
    sqlx::query("INSERT INTO nodes(name,uuid,lease_expires_at,deployment_id,build_id) VALUES('race','race',now()+interval '1 hour','a',$1)")
        .bind(gammaboard::provenance::build_identity()).execute(&mut *registration).await?;
    let store = PgStore::new(db.pool.clone());
    let mut replacement = tokio::spawn(async move { store.bind_deployment(Some("b")).await });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut replacement)
            .await
            .is_err()
    );
    registration.commit().await?;
    assert!(replacement.await?.is_err());
    sqlx::query("DELETE FROM nodes").execute(&db.pool).await?;
    let mut replacement = db.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(71809242)")
        .execute(&mut *replacement)
        .await?;
    sqlx::query("UPDATE gammaboard_deployment SET deployment_id='b'")
        .execute(&mut *replacement)
        .await?;
    let pool = db.pool.clone();
    let mut old_worker = tokio::spawn(async move {
        sqlx::query("INSERT INTO nodes(name,uuid,lease_expires_at,deployment_id,build_id) VALUES('old','old',now()+interval '1 hour','a',$1)")
            .bind(gammaboard::provenance::build_identity()).execute(&pool).await
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut old_worker)
            .await
            .is_err()
    );
    replacement.commit().await?;
    assert!(old_worker.await?.is_err());
    db.finish().await
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL with CREATE DATABASE permission"]
async fn late_registration_finishes_a_partially_failed_request_without_a_watcher()
-> anyhow::Result<()> {
    let db = Database::new().await?;
    let id = db.request(2).await?;
    let claim = db
        .store
        .claim_external_launch("launcher", None)
        .await?
        .unwrap();
    db.store
        .update_external_worker(
            id,
            token(&claim),
            "worker-1",
            "failed",
            None,
            Some("rejected"),
        )
        .await?;
    db.store
        .update_external_worker(id, token(&claim), "worker-2", "submitting", None, None)
        .await?;
    db.store
        .announce_node("worker-2", "registered", &Default::default())
        .await?;
    let state: String = sqlx::query_scalar("SELECT state FROM node_launch_requests WHERE id=$1")
        .bind(id)
        .fetch_one(&db.pool)
        .await?;
    assert_eq!(state, "failed");
    db.store
        .update_external_worker(
            id,
            token(&claim),
            "worker-2",
            "submitted",
            Some("102"),
            None,
        )
        .await?;
    let worker: (String, Option<String>) = sqlx::query_as("SELECT state,job_id FROM external_worker_launches WHERE request_id=$1 AND node_name='worker-2'").bind(id).fetch_one(&db.pool).await?;
    assert_eq!(worker, ("connected".into(), Some("102".into())));
    assert!(
        db.store
            .claim_external_launch("next", None)
            .await?
            .is_none()
    );
    db.finish().await
}
