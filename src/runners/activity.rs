//! In-memory activity changes are cheap; the independent lease task persists at
//! most one snapshot per heartbeat, even while a synchronous runtime is blocked.
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::sync::{Arc, Mutex};

#[derive(Clone, Serialize)]
pub struct Activity {
    pub activity: String,
    pub since: DateTime<Utc>,
    pub last_completed_batch_at: Option<DateTime<Utc>>,
    pub node_name: String,
    pub run_id: Option<i32>,
    pub task_id: Option<i64>,
}
pub type Handle = Arc<Mutex<Activity>>;
tokio::task_local! { pub static CURRENT: Handle; }

pub fn new(node_name: String) -> Handle {
    Arc::new(Mutex::new(Activity {
        activity: "waiting".into(),
        since: Utc::now(),
        last_completed_batch_at: None,
        node_name,
        run_id: None,
        task_id: None,
    }))
}
pub fn current() -> Option<Handle> {
    CURRENT.try_with(Clone::clone).ok()
}
pub fn snapshot(handle: &Handle) -> serde_json::Value {
    handle
        .lock()
        .ok()
        .and_then(|a| serde_json::to_value(&*a).ok())
        .unwrap_or_default()
}
pub fn set_on(handle: &Handle, phase: &str) {
    if let Ok(mut a) = handle.lock()
        && a.activity != phase
    {
        a.activity = phase.into();
        a.since = Utc::now();
    }
}
pub fn set(phase: &str) {
    if let Some(a) = current() {
        set_on(&a, phase);
    }
}
pub fn context(run_id: i32, task_id: i64) {
    if let Some(handle) = current()
        && let Ok(mut a) = handle.lock()
    {
        a.run_id = Some(run_id);
        a.task_id = Some(task_id);
    }
}
pub fn completed_batch() {
    if let Some(handle) = current()
        && let Ok(mut a) = handle.lock()
    {
        a.last_completed_batch_at = Some(Utc::now());
    }
}

pub struct PhaseGuard {
    previous: Option<(Handle, String, DateTime<Utc>)>,
}
impl PhaseGuard {
    pub fn enter(handle: Option<Handle>, phase: &str) -> Self {
        let previous = handle.and_then(|handle| {
            let mut activity = handle.lock().ok()?;
            let previous = (handle.clone(), activity.activity.clone(), activity.since);
            if activity.activity != phase {
                activity.activity = phase.into();
                activity.since = Utc::now();
            }
            Some(previous)
        });
        Self { previous }
    }
}
impl Drop for PhaseGuard {
    fn drop(&mut self) {
        if let Some((handle, phase, since)) = &self.previous
            && let Ok(mut activity) = handle.lock()
        {
            activity.activity = phase.clone();
            activity.since = *since;
        }
    }
}

pub fn report_progress(handle: Option<&Handle>, phase: &str) -> Result<(), String> {
    if !matches!(
        phase,
        "waiting"
            | "materializing"
            | "evaluating"
            | "updating sampler"
            | "saving checkpoint"
            | "shutdown"
    ) {
        return Err(format!("unsupported worker progress activity '{phase}'"));
    }
    if let Some(h) = handle {
        set_on(h, phase);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_activity_preserves_since_and_request_scope_restores_it() {
        let h = new("test".into());
        let before = snapshot(&h);
        set_on(&h, "waiting");
        assert_eq!(snapshot(&h)["since"], before["since"]);
        {
            let _guard = PhaseGuard::enter(Some(h.clone()), "waiting for sampler response");
            report_progress(Some(&h), "updating sampler").unwrap();
            assert_eq!(snapshot(&h)["activity"], "updating sampler");
        }
        assert_eq!(snapshot(&h)["since"], before["since"]);
        assert!(report_progress(Some(&h), "invented").is_err());
    }
}
