use serde::Serialize;
use std::{collections::HashMap, future::Future, sync::Arc, time::Duration};
use tokio::{sync::Mutex, time::Instant};

#[derive(Clone, Default)]
pub(super) struct RunRemovals(Arc<Mutex<HashMap<String, Entry>>>);

struct Entry {
    operation: RunRemoval,
    completed_at: Option<Instant>,
}

#[derive(Clone, Serialize)]
pub(super) struct RunRemoval {
    pub operation_id: String,
    pub run_id: i32,
    pub run_name: String,
    pub status: RemovalStatus,
    pub error: Option<String>,
    pub poll_after_ms: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RemovalStatus {
    Running,
    Completed,
    Failed,
}

impl RunRemovals {
    // Deletion stays transactional in PgStore. HTTP only submits and polls the
    // operation, so proxy timeouts or disconnected browsers cannot cancel it.
    pub async fn start(
        &self,
        run_id: i32,
        run_name: String,
        remove: impl Future<Output = Result<(), String>> + Send + 'static,
    ) -> RunRemoval {
        let mut entries = self.0.lock().await;
        entries.retain(|_, entry| {
            entry
                .completed_at
                .is_none_or(|at| at.elapsed() < Duration::from_secs(3600))
        });
        if let Some(entry) = entries.values().find(|entry| {
            entry.operation.run_id == run_id && entry.operation.status == RemovalStatus::Running
        }) {
            return entry.operation.clone();
        }
        let operation = RunRemoval {
            operation_id: uuid::Uuid::new_v4().to_string(),
            run_id,
            run_name,
            status: RemovalStatus::Running,
            error: None,
            poll_after_ms: 1000,
        };
        entries.insert(
            operation.operation_id.clone(),
            Entry {
                operation: operation.clone(),
                completed_at: None,
            },
        );
        let operations = self.clone();
        let operation_id = operation.operation_id.clone();
        tokio::spawn(async move {
            // Observe panics as failures rather than leaving a permanently busy job.
            let result = tokio::spawn(remove)
                .await
                .unwrap_or_else(|error| Err(format!("run removal task stopped: {error}")));
            let mut entries = operations.0.lock().await;
            if let Some(entry) = entries.get_mut(&operation_id) {
                entry.operation.status = if result.is_ok() {
                    RemovalStatus::Completed
                } else {
                    RemovalStatus::Failed
                };
                entry.operation.error = result.err();
                entry.completed_at = Some(Instant::now());
            }
        });
        operation
    }

    pub async fn get(&self, operation_id: &str) -> Option<RunRemoval> {
        self.0
            .lock()
            .await
            .get(operation_id)
            .map(|entry| entry.operation.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn finished(operations: &RunRemovals, id: &str) -> RunRemoval {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let operation = operations.get(id).await.unwrap();
                if operation.status != RemovalStatus::Running {
                    break operation;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn slow_removal_returns_immediately_and_duplicate_requests_share_the_operation() {
        let operations = RunRemovals::default();
        let (release, blocked) = tokio::sync::oneshot::channel();
        let operation = operations
            .start(1, "campaign".into(), async move {
                blocked.await.unwrap();
                Ok(())
            })
            .await;
        assert_eq!(operation.status, RemovalStatus::Running);
        let duplicate = operations
            .start(1, "campaign".into(), async {
                panic!("duplicate deletion must not execute")
            })
            .await;
        assert_eq!(operation.operation_id, duplicate.operation_id);
        release.send(()).unwrap();
        assert_eq!(
            finished(&operations, &operation.operation_id).await.status,
            RemovalStatus::Completed
        );
    }

    #[tokio::test]
    async fn failed_removal_exposes_the_error_and_allows_an_explicit_retry() {
        let operations = RunRemovals::default();
        let operation = operations
            .start(1, "campaign".into(), async {
                Err("workers are still draining; run retained, retry removal".into())
            })
            .await;
        let failed = finished(&operations, &operation.operation_id).await;
        assert_eq!(failed.status, RemovalStatus::Failed);
        assert!(failed.error.unwrap().contains("workers are still draining"));
        let retry = operations
            .start(1, "campaign".into(), async { Ok(()) })
            .await;
        assert_ne!(retry.operation_id, operation.operation_id);
        assert_eq!(
            finished(&operations, &retry.operation_id).await.status,
            RemovalStatus::Completed
        );
    }
}
