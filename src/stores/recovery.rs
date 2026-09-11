use super::PgStore;
use crate::core::{SamplerAggregatorCheckpoint, StoreError};

impl PgStore {
    /// Roll back live progress and speculative work to the same boundary as the sampler.
    /// Retained results within that boundary can be ingested again, exactly once.
    pub async fn restore_sampler_checkpoint(
        &self,
        run_id: i32,
        checkpoint: &SamplerAggregatorCheckpoint,
    ) -> Result<(), StoreError> {
        let produced = checkpoint.produced_samples();
        let last_produced = checkpoint.queue.last_produced_batch_id;
        if produced > 0 && last_produced.is_none() {
            return Err(StoreError::store(
                "checkpoint has no production boundary; cannot safely recover queued work",
            ));
        }
        let payload =
            serde_json::to_value(checkpoint).map_err(|e| StoreError::store(e.to_string()))?;
        let observable = checkpoint
            .observable_state
            .to_json()
            .map_err(|e| StoreError::store(e.to_string()))?;
        let mut tx = self.pool().begin().await?;
        let saved: serde_json::Value = sqlx::query_scalar(
            "SELECT sampler_checkpoint FROM run_sampler_checkpoints WHERE run_id=$1 FOR UPDATE",
        )
        .bind(run_id)
        .fetch_one(&mut *tx)
        .await?;
        if saved != payload {
            return Err(StoreError::store(
                "recovery checkpoint changed during runtime initialization; retry activation",
            ));
        }
        let retained: i64 = sqlx::query_scalar(
            "SELECT COALESCE(sum(batch_size),0)::bigint FROM batches WHERE run_id=$1 AND task_id=$2 AND id>$3 AND id<=$4"
        ).bind(run_id).bind(checkpoint.task_id)
            .bind(checkpoint.queue.last_completed_batch_id.unwrap_or(0))
            .bind(last_produced.unwrap_or(0)).fetch_one(&mut *tx).await?;
        let expected = produced - checkpoint.completed_samples;
        if retained != expected {
            return Err(StoreError::store(format!(
                "checkpoint needs {expected} samples of queued work but only {retained} remain; refusing inconsistent recovery"
            )));
        }
        // Result submission checks ownership on the batch row in its transaction. Deleting
        // later batches also rejects results arriving from evaluators still computing them.
        sqlx::query("DELETE FROM batches WHERE run_id=$1 AND task_id=$2 AND id>$3")
            .bind(run_id)
            .bind(checkpoint.task_id)
            .bind(last_produced.unwrap_or(0))
            .execute(&mut *tx)
            .await?;
        let task = sqlx::query("UPDATE run_tasks SET nr_produced_samples=$2,nr_completed_samples=$3 WHERE id=$1 AND run_id=$4 AND state='active'")
            .bind(checkpoint.task_id).bind(produced).bind(checkpoint.completed_samples).bind(run_id)
            .execute(&mut *tx).await?;
        if task.rows_affected() != 1 {
            return Err(StoreError::store("checkpoint task is no longer active"));
        }
        sqlx::query(
            "DELETE FROM persisted_observable_snapshots WHERE run_id=$1 AND task_id=$2 AND id>$3",
        )
        .bind(run_id)
        .bind(checkpoint.task_id)
        .bind(checkpoint.output_snapshot_id.unwrap_or(0))
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE runs SET batches_completed=$3,nr_produced_samples=(SELECT COALESCE(sum(nr_produced_samples),0) FROM run_tasks WHERE run_id=$1),nr_completed_samples=(SELECT COALESCE(sum(nr_completed_samples),0) FROM run_tasks WHERE run_id=$1),current_observable=$2 WHERE id=$1")
            .bind(run_id).bind(observable).bind(checkpoint.batches_completed).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
}
