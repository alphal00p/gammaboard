//! Cumulative occupied wall time, shared by the runner and its I/O tasks.
use crate::core::models::WorkerBusyMetrics;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone)]
pub(crate) struct BusyTime(Arc<Mutex<State>>);

struct State {
    started: Instant,
    accounted: Instant,
    active: [usize; 2],
    occupied: [Duration; 2],
}

impl State {
    fn account(&mut self, now: Instant) {
        let elapsed = now.duration_since(self.accounted);
        for (active, occupied) in self.active.iter().zip(&mut self.occupied) {
            if *active > 0 {
                *occupied += elapsed;
            }
        }
        self.accounted = now;
    }
}

impl Default for BusyTime {
    fn default() -> Self {
        let now = Instant::now();
        Self(Arc::new(Mutex::new(State {
            started: now,
            accounted: now,
            active: [0; 2],
            occupied: [Duration::ZERO; 2],
        })))
    }
}

impl BusyTime {
    pub(crate) fn compute(&self) -> BusyGuard {
        self.start(0)
    }

    /// Start inside the operation, not when scheduling it or collecting its result.
    pub(crate) fn io(&self) -> BusyGuard {
        self.start(1)
    }

    fn start(&self, lane: usize) -> BusyGuard {
        let mut state = self.0.lock().unwrap();
        state.account(Instant::now());
        state.active[lane] += 1;
        BusyGuard {
            clock: self.clone(),
            lane,
        }
    }

    pub(crate) fn snapshot(&self) -> WorkerBusyMetrics {
        let mut state = self.0.lock().unwrap();
        let now = Instant::now();
        state.account(now);
        WorkerBusyMetrics {
            elapsed_seconds: now.duration_since(state.started).as_secs_f64(),
            compute_seconds: state.occupied[0].as_secs_f64(),
            io_seconds: state.occupied[1].as_secs_f64(),
        }
    }
}

/// Ends on success, error, unwind, or cancellation, before a result is collected.
pub(crate) struct BusyGuard {
    clock: BusyTime,
    lane: usize,
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        let mut state = self.clock.0.lock().unwrap();
        state.account(Instant::now());
        state.active[self.lane] -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_operations_count_once_and_snapshots_split_open_intervals() {
        let start = Instant::now();
        let mut state = State {
            started: start,
            accounted: start,
            active: [0, 1],
            occupied: [Duration::ZERO; 2],
        };
        state.account(start + Duration::from_secs(2));
        state.active = [1, 2];
        state.account(start + Duration::from_secs(5));
        assert_eq!(
            state.occupied,
            [Duration::from_secs(3), Duration::from_secs(5)]
        );
        state.active[1] -= 1;
        state.account(start + Duration::from_secs(7));
        state.active = [0, 0];
        state.account(start + Duration::from_secs(10));
        assert_eq!(
            state.occupied,
            [Duration::from_secs(5), Duration::from_secs(7)]
        );
    }

    #[tokio::test]
    async fn finished_uncollected_failed_and_cancelled_tasks_stop_counting() {
        let clock = BusyTime::default();
        let worker = clock.clone();
        let finished = tokio::spawn(async move {
            let _io = worker.io();
            Err::<(), _>("failed operation")
        });
        while !finished.is_finished() {
            tokio::task::yield_now().await;
        }
        let before = clock.snapshot();
        tokio::task::yield_now().await;
        assert_eq!(clock.snapshot().io_seconds, before.io_seconds);
        assert!(finished.await.unwrap().is_err());

        let worker = clock.clone();
        let (started, ready) = tokio::sync::oneshot::channel();
        let pending = tokio::spawn(async move {
            let _io = worker.io();
            started.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        ready.await.unwrap();
        assert!(clock.snapshot().io_seconds >= before.io_seconds);
        pending.abort();
        assert!(pending.await.unwrap_err().is_cancelled());
        let after = clock.snapshot();
        tokio::task::yield_now().await;
        assert_eq!(clock.snapshot().io_seconds, after.io_seconds);
        assert_eq!(clock.0.lock().unwrap().active, [0, 0]);
    }
}
