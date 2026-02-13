use sqlx::{Pool, Postgres, postgres::PgPoolOptions};
use dotenvy::dotenv;
use std::env;


pub async fn get_pg_pool(max_connections : u32) -> Result<Pool<Postgres>, sqlx::Error>{
    dotenv().ok();
    let db_url = env::var("DATABASE_URL").unwrap();
    PgPoolOptions::new()
    .max_connections(max_connections)
    .connect(&db_url)
    .await
}
