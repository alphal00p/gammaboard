use crate::core::StoreError;
use std::num::NonZeroUsize;
use tokio::runtime::{Builder, Handle, Runtime};

/// Background I/O for one sampler runner. Sampler/model mutation stays on the caller.
pub(super) struct SamplerIo {
    runtime: Option<Runtime>,
}

impl SamplerIo {
    pub(super) fn new(threads: NonZeroUsize) -> Result<Self, StoreError> {
        let runtime = Builder::new_multi_thread()
            .worker_threads(threads.get())
            .thread_name("sampler-io")
            .enable_all()
            .build()
            .map_err(|error| StoreError::store(format!("failed to start sampler I/O: {error}")))?;
        Ok(Self {
            runtime: Some(runtime),
        })
    }

    pub(super) fn handle(&self) -> &Handle {
        self.runtime
            .as_ref()
            .expect("sampler I/O is running")
            .handle()
    }
}

impl Drop for SamplerIo {
    fn drop(&mut self) {
        // Normal stop drains writes before dropping the runner. Also cancel any
        // remaining work on an error path, without blocking the node's runtime.
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::time::Duration;

    #[tokio::test]
    async fn synchronous_io_can_overlap_without_blocking_the_node() {
        let io = SamplerIo::new(NonZeroUsize::new(2).unwrap()).unwrap();
        assert_eq!(io.handle().metrics().num_workers(), 2);
        let (started, mut ready) = tokio::sync::mpsc::unbounded_channel();
        let mut releases = Vec::new();
        let mut tasks = Vec::new();
        for _ in 0..2 {
            let started = started.clone();
            let (release, wait) = std::sync::mpsc::channel();
            releases.push(release);
            tasks.push(io.handle().spawn(async move {
                started.send(std::thread::current().id()).unwrap();
                wait.recv_timeout(Duration::from_secs(5)).unwrap();
            }));
        }
        let mut threads = HashSet::new();
        for _ in 0..2 {
            threads.insert(
                tokio::time::timeout(Duration::from_secs(2), ready.recv())
                    .await
                    .unwrap()
                    .unwrap(),
            );
        }
        assert_eq!(threads.len(), 2);
        assert!(!threads.contains(&std::thread::current().id()));
        for release in releases {
            release.send(()).unwrap();
        }
        for task in tasks {
            task.await.unwrap();
        }
    }

    #[tokio::test]
    async fn dropping_a_role_cancels_its_io_and_allows_a_different_pool() {
        for threads in [1, 3] {
            let io = SamplerIo::new(NonZeroUsize::new(threads).unwrap()).unwrap();
            assert_eq!(io.handle().metrics().num_workers(), threads);
            let (started, ready) = tokio::sync::oneshot::channel();
            let pending = io.handle().spawn(async move {
                started.send(()).unwrap();
                std::future::pending::<()>().await;
            });
            ready.await.unwrap();
            drop(io);
            assert!(
                tokio::time::timeout(Duration::from_secs(2), pending)
                    .await
                    .unwrap()
                    .unwrap_err()
                    .is_cancelled()
            );
            assert_eq!(tokio::spawn(async { 42 }).await.unwrap(), 42);
        }
    }
}
