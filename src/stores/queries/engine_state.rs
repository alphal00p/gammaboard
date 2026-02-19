use serde_json::Value as JsonValue;
use sqlx::PgPool;

pub(crate) async fn load_engine_state(
    pool: &PgPool,
    run_id: i32,
) -> Result<Option<JsonValue>, sqlx::Error> {
    let state = sqlx::query_scalar::<_, JsonValue>(
        r#"
        SELECT state
        FROM sampler_states
        WHERE run_id = $1
        ORDER BY version DESC
        LIMIT 1
        "#,
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await?;

    Ok(state)
}

pub(crate) async fn save_engine_state(
    pool: &PgPool,
    run_id: i32,
    state_payload: &JsonValue,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO sampler_states (
            run_id,
            version,
            state,
            nr_samples_trained,
            training_error
        )
        VALUES (
            $1,
            COALESCE((SELECT MAX(version) + 1 FROM sampler_states WHERE run_id = $1), 1),
            $2,
            NULL,
            NULL
        )
        "#,
    )
    .bind(run_id)
    .bind(state_payload)
    .execute(pool)
    .await?;
    Ok(())
}
