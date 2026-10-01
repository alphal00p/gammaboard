//! Saturated database paths, with computation replaced by prepared payloads.
//! Publication, fenced claims, result submission and decoding use WorkQueueStore.
//! These measure storage capacity, independently of runner ticks and checkpointing.
use super::IoRuntime;
use crate::{
    api::runs,
    core::{
        AccumulatorConfig, ControlPlaneStore, RunTaskStore, WorkQueueStore, WorkerBusyMetrics,
        WorkerRole,
    },
    evaluation::BatchResult,
    runners::busy_time::BusyTime,
    sampling::{LatentBatch, LatentBatchPayload},
    stores::{PgStore, get_pg_pool},
};
use anyhow::{Result, ensure};
use rand::{Rng, SeedableRng};
use rand_xoshiro::Xoshiro256PlusPlus;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering::SeqCst},
    },
    time::{Duration, Instant},
};
use tokio::{
    sync::{Semaphore, mpsc},
    task::JoinSet,
};

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub role: String,
    pub batch_size: usize,
    pub feedback: bool,
    pub duration_seconds: f64,
    pub warmup_seconds: f64,
    pub io_threads: usize,
    pub consumers: usize,
    pub memory_mib: usize,
    #[serde(default = "default_insert_concurrency")]
    pub insert_concurrency: usize,
    #[serde(default = "default_queue_batches")]
    pub queue_batches: usize,
    #[serde(default)]
    pub profile: bool,
    pub sampler_cpus: Vec<usize>,
    pub evaluator_cpus: Vec<usize>,
}
fn default_insert_concurrency() -> usize {
    4
}
fn default_queue_batches() -> usize {
    64
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            matches!(self.role.as_str(), "sampler" | "evaluator"),
            "unknown I/O role"
        );
        ensure!(
            (16..=1_048_576).contains(&self.batch_size),
            "batch size must be 16..1048576"
        );
        ensure!(
            (1..=32).contains(&self.io_threads) && (1..=64).contains(&self.consumers),
            "invalid concurrency"
        );
        ensure!(
            (1..=16).contains(&self.insert_concurrency) && (8..=512).contains(&self.queue_batches),
            "invalid insert concurrency or queue capacity"
        );
        ensure!(
            self.duration_seconds.is_finite() && (0.5..=60.).contains(&self.duration_seconds),
            "duration must be 0.5..60s"
        );
        ensure!(
            self.warmup_seconds.is_finite() && (0.1..=10.).contains(&self.warmup_seconds),
            "warmup must be 0.1..10s"
        );
        ensure!(
            (64..=16384).contains(&self.memory_mib),
            "memory budget must be 64..16384 MiB"
        );
        ensure!(
            !self.sampler_cpus.is_empty() && !self.evaluator_cpus.is_empty(),
            "assign CPUs to both roles"
        );
        ensure!(
            self.sampler_cpus
                .iter()
                .all(|c| !self.evaluator_cpus.contains(c)),
            "role CPU allocations overlap"
        );
        ensure!(
            self.role != "evaluator" || self.consumers == 1,
            "evaluator I/O measures one worker"
        );
        ensure!(
            self.batch_size * 160 * 8 <= self.memory_mib * 1024 * 1024,
            "memory budget cannot hold eight batches; increase --memory-mib or lower batch size"
        );
        Ok(())
    }
}
/// Include decoded inputs, feedback and transient copies in estimated residency.
fn queue_shape(
    batch_size: usize,
    memory_mib: usize,
    role: &str,
    queue_batches: usize,
) -> (usize, usize) {
    let slots = (memory_mib * 1024 * 1024 / (batch_size * 160)).clamp(
        8,
        if role == "sampler" {
            queue_batches
        } else {
            512
        },
    );
    let bundle = if role == "sampler" {
        (slots / 8).clamp(1, 5)
    } else {
        5.min(slots)
    };
    (slots, bundle)
}
struct Prepared {
    batches: Vec<LatentBatch>,
    result: BatchResult,
    input_bytes: usize,
    feedback_bytes: usize,
}
fn prepare(size: usize, feedback: bool, bundle: usize) -> Result<Prepared> {
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(1234);
    let continuous: Vec<f64> = (0..size * 6).map(|_| rng.random()).collect();
    let values: Vec<f64> = continuous.chunks_exact(6).map(|p| p[0]).collect();
    let evaluator: crate::core::EvaluatorConfig = serde_json::from_value(json!({
        "kind":"unit", "continuous_dims":6, "value_coordinate":0}))?;
    let latent = LatentBatch {
        nr_samples: size,
        accumulator: AccumulatorConfig::scalar(),
        payload: LatentBatchPayload::IndexedBatch {
            discrete_signatures: vec![vec![]],
            discrete_map: vec![0; size],
            continuous_layouts: vec![6; size],
            continuous_values: continuous,
            weights: vec![1.; size],
        },
    };
    let result = evaluator.build()?.eval_batch(
        &latent.payload.clone().into_batch()?,
        &AccumulatorConfig::scalar(),
        crate::EvalBatchOptions {
            require_training_values: feedback,
        },
    )?;
    ensure!(
        !feedback || result.values.as_ref() == Some(&values),
        "prepared feedback differs from f(x)=x[0]"
    );
    let input_bytes = latent.to_bytes()?.len();
    let feedback_bytes = result.values_to_bytes()?.map_or(0, |v| v.len());
    Ok(Prepared {
        batches: vec![latent; bundle],
        result,
        input_bytes,
        feedback_bytes,
    })
}
const STAGES: [&str; 7] = [
    "serialize",
    "metadata_insert",
    "input_copy",
    "commit",
    "insert_total",
    "fetch",
    "cleanup",
];
#[derive(Default)]
struct Progress {
    published: AtomicU64,
    submitted: AtomicU64,
    collected: AtomicU64,
    empty_claims: AtomicU64,
    empty_fetches: AtomicU64,
    stop: AtomicBool,
    writers_done: AtomicBool,
    measuring: AtomicBool,
    stages: Mutex<[f64; 7]>,
    database_activity: Mutex<BTreeMap<String, u64>>,
}
/// A later insert can commit first. Collection must not skip unpublished IDs.
#[derive(Default)]
struct Published {
    through: u64,
    ranges: BTreeMap<u64, u64>,
}
impl Published {
    fn complete(&mut self, first: u64, end: u64) {
        self.ranges.insert(first, end);
        while let Some(end) = self.ranges.remove(&(self.through + 1)) {
            self.through = end;
        }
    }
}
#[derive(Serialize)]
struct Window {
    elapsed_seconds: f64,
    published_batches: u64,
    submitted_batches: u64,
    collected_batches: u64,
    empty_claims: u64,
    empty_fetches: u64,
    sampler_io_seconds: f64,
    sampler_elapsed_seconds: f64,
    evaluator_io_seconds: f64,
    evaluator_elapsed_seconds: f64,
    sampler_stage_seconds: BTreeMap<&'static str, f64>,
}
struct Snapshot {
    at: Instant,
    published: u64,
    submitted: u64,
    collected: u64,
    empty_claims: u64,
    empty_fetches: u64,
    sampler: WorkerBusyMetrics,
    evaluators: Vec<WorkerBusyMetrics>,
    stages: [f64; 7],
}
fn snapshot(p: &Progress, sampler: &BusyTime, evaluators: &[BusyTime]) -> Snapshot {
    Snapshot {
        at: Instant::now(),
        stages: *p.stages.lock().unwrap(),
        published: p.published.load(SeqCst),
        submitted: p.submitted.load(SeqCst),
        collected: p.collected.load(SeqCst),
        empty_claims: p.empty_claims.load(SeqCst),
        empty_fetches: p.empty_fetches.load(SeqCst),
        sampler: sampler.snapshot(),
        evaluators: evaluators.iter().map(BusyTime::snapshot).collect(),
    }
}
fn window(a: Snapshot, b: Snapshot) -> Window {
    Window {
        sampler_stage_seconds: STAGES
            .into_iter()
            .enumerate()
            .map(|(i, name)| (name, b.stages[i] - a.stages[i]))
            .collect(),
        elapsed_seconds: b.at.duration_since(a.at).as_secs_f64(),
        published_batches: b.published - a.published,
        submitted_batches: b.submitted - a.submitted,
        collected_batches: b.collected - a.collected,
        empty_claims: b.empty_claims - a.empty_claims,
        empty_fetches: b.empty_fetches - a.empty_fetches,
        sampler_io_seconds: b.sampler.io_seconds - a.sampler.io_seconds,
        sampler_elapsed_seconds: b.sampler.elapsed_seconds - a.sampler.elapsed_seconds,
        evaluator_io_seconds: b
            .evaluators
            .iter()
            .zip(&a.evaluators)
            .map(|(b, a)| b.io_seconds - a.io_seconds)
            .sum(),
        evaluator_elapsed_seconds: b
            .evaluators
            .iter()
            .zip(&a.evaluators)
            .map(|(b, a)| b.elapsed_seconds - a.elapsed_seconds)
            .sum(),
    }
}

async fn consumer(
    store: PgStore,
    run: i32,
    uuid: String,
    prepared: Arc<Prepared>,
    busy: BusyTime,
    progress: Arc<Progress>,
    limit: Option<u64>,
) -> Result<()> {
    // One batch of lookahead. Waiting for channel space and completed-but-uncollected
    // results never count as busy; only actual claim/decode and encode/submit do.
    let (tx, mut rx) = mpsc::channel(1);
    let (fetch_store, fetch_uuid, fetch_busy, fetch_progress) =
        (store.clone(), uuid.clone(), busy.clone(), progress.clone());
    let mut fetches = JoinSet::new();
    fetches.spawn(async move {
        let mut claimed = 0;
        while limit.is_none_or(|n| claimed < n) && !fetch_progress.stop.load(SeqCst) {
            let Ok(permit) = tx.reserve().await else {
                break;
            };
            let token = uuid::Uuid::new_v4().to_string();
            let batch = {
                let _io = fetch_busy.io();
                fetch_store.claim_batch(run, &fetch_uuid, &token).await?
            };
            if let Some(batch) = batch {
                claimed += 1;
                permit.send(batch);
            } else {
                fetch_progress.empty_claims.fetch_add(1, SeqCst);
                drop(permit);
                if limit.is_some() {
                    anyhow::bail!("prefilled evaluator queue starved");
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        }
        Ok::<_, anyhow::Error>(())
    });
    while let Some(batch) = rx.recv().await {
        ensure!(
            batch.latent_batch.nr_samples == prepared.batches[0].nr_samples
                && batch.requires_training_values == prepared.result.values.is_some(),
            "unexpected input batch"
        );
        {
            let _io = busy.io();
            store
                .submit_batch_results(
                    batch.batch_id,
                    &uuid,
                    &batch.claim_token,
                    &prepared.result,
                    0.,
                )
                .await?;
        }
        progress.submitted.fetch_add(1, SeqCst);
    }
    fetches.join_next().await.unwrap()??;
    Ok(())
}

pub async fn measure(database_url: &str, config: Config) -> Result<Value> {
    config.validate()?;
    let control = PgStore::new(get_pg_pool(database_url, 2).await?);
    let name = format!("io-benchmark-{}", uuid::Uuid::new_v4());
    let definition = runs::parse_run_add_config_toml(&format!(
        r#"
name = "{name}"
[evaluator]
kind = "unit"
continuous_dims = 6
[[task_queue]]
name = "measure"
kind = "sample"
stop_condition = {{ max_samples = 1000000000000 }}
accumulator = {{ config = "scalar" }}
sampler_aggregator = {{ config = {{ kind = "naive_monte_carlo" }} }}
"#
    ))?;
    let run = runs::create_run(&control, definition).await?.run_id;
    let task = control.list_run_tasks(run).await?[0].id;
    // All benchmark work lies after this empty recovery boundary. Production
    // cleanup can discard it without retaining batches for replay.
    sqlx::query(
        "INSERT INTO run_sampler_checkpoints (run_id,task_id,sampler_checkpoint) VALUES ($1,$2,$3)",
    )
    .bind(run)
    .bind(task)
    .bind(json!({"queue":{"last_completed_batch_id":0,"last_produced_batch_id":0}}))
    .execute(control.pool())
    .await?;
    let uuids: Vec<_> = (0..config.consumers)
        .map(|_| uuid::Uuid::new_v4().to_string())
        .collect();
    let outcome = run_case(database_url, &control, run, task, &uuids, &config).await;
    for id in &uuids {
        control.clear_current_assignment(id).await?;
        control.expire_node_lease(id).await?;
    }
    control.remove_run(run).await?;
    outcome
}

async fn run_case(
    url: &str,
    control: &PgStore,
    run: i32,
    task: i64,
    uuids: &[String],
    c: &Config,
) -> Result<Value> {
    let sampler_runtime = IoRuntime::new(c.io_threads, c.sampler_cpus.clone())?;
    let evaluator_runtime = IoRuntime::new(c.evaluator_cpus.len(), c.evaluator_cpus.clone())?;
    let sampler = PgStore::new(get_pg_pool(url, (c.insert_concurrency + 2) as u32).await?);
    let mut stores = Vec::new();
    for uuid in uuids {
        control
            .announce_node(uuid, uuid, &Default::default())
            .await?;
        control
            .set_current_assignment(uuid, WorkerRole::Evaluator, run)
            .await?;
        stores.push(PgStore::new(get_pg_pool(url, 2).await?));
    }
    let mut tasks = JoinSet::<Result<()>>::new();
    let p = Arc::new(Progress::default());
    let heartbeat_store = control.clone();
    let heartbeat_ids = uuids.to_vec();
    let mut heartbeats = JoinSet::new();
    if c.profile {
        let profile_store = control.clone();
        let profile_progress = p.clone();
        heartbeats.spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(100)).await;
                if !profile_progress.measuring.load(SeqCst) { continue; }
                let rows: Vec<(String, i64)> = sqlx::query_as(
                    "SELECT COALESCE(wait_event_type || ':' || wait_event, 'CPU_or_runnable'), count(*) FROM pg_stat_activity WHERE datname=current_database() AND state='active' AND pid<>pg_backend_pid() GROUP BY 1")
                    .fetch_all(profile_store.pool()).await?;
                if profile_progress.measuring.load(SeqCst) {
                    let mut counts = profile_progress.database_activity.lock().unwrap();
                    for (state,count) in rows { *counts.entry(state).or_default() += count as u64; }
                }
            }
            #[allow(unreachable_code)]
            Ok::<(),anyhow::Error>(())
        });
    }
    heartbeats.spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            for id in &heartbeat_ids {
                heartbeat_store
                    .announce_node(id, id, &Default::default())
                    .await?;
            }
        }
        #[allow(unreachable_code)]
        Ok::<(), anyhow::Error>(())
    });
    let (slots, bundle) = queue_shape(c.batch_size, c.memory_mib, &c.role, c.queue_batches);
    let prepared = Arc::new(prepare(c.batch_size, c.feedback, bundle)?);
    let sampler_busy = BusyTime::default();
    let evaluator_busy: Vec<_> = uuids.iter().map(|_| BusyTime::default()).collect();
    let mut windows = Vec::new();
    if c.role == "sampler" {
        let credits = Arc::new(Semaphore::new(slots));
        let next = Arc::new(AtomicU64::new(1));
        let published = Arc::new(Mutex::new(Published::default()));
        let mut writers = JoinSet::new();
        for _ in 0..c.insert_concurrency {
            let (store, data, p, next, credits, published, busy) = (
                sampler.clone(),
                prepared.clone(),
                p.clone(),
                next.clone(),
                credits.clone(),
                published.clone(),
                sampler_busy.clone(),
            );
            writers.spawn_on(
                async move {
                    while !p.writers_done.load(SeqCst) {
                        let permit = credits.acquire_many(bundle as u32).await?;
                        if p.writers_done.load(SeqCst) {
                            break;
                        }
                        let first = next.fetch_add(bundle as u64, SeqCst);
                        let ids: Vec<_> =
                            (first..first + bundle as u64).map(|v| v as i64).collect();
                        {
                            let _io = busy.io();
                            let result = store
                                .insert_batches(
                                    run,
                                    task,
                                    data.result.values.is_some(),
                                    &ids,
                                    &data.batches,
                                )
                                .await?;
                            let m = result.metrics;
                            let mut stages = p.stages.lock().unwrap();
                            for (i, value) in [
                                m.serialize_ms,
                                m.insert_batches_exec_ms,
                                m.insert_inputs_exec_ms,
                                m.commit_ms,
                                m.end_to_end_ms,
                            ]
                            .into_iter()
                            .enumerate()
                            {
                                stages[i] += value / 1000.;
                            }
                        }
                        published
                            .lock()
                            .unwrap()
                            .complete(first, first + bundle as u64 - 1);
                        p.published.fetch_add(bundle as u64, SeqCst);
                        permit.forget();
                    }
                    Ok::<_, anyhow::Error>(())
                },
                sampler_runtime.handle(),
            );
        }
        for ((store, uuid), busy) in stores.iter().zip(uuids).zip(&evaluator_busy) {
            tasks.spawn_on(
                consumer(
                    store.clone(),
                    run,
                    uuid.clone(),
                    prepared.clone(),
                    busy.clone(),
                    p.clone(),
                    None,
                ),
                evaluator_runtime.handle(),
            );
        }
        let (store, busy, pc, credits_c) = (
            sampler.clone(),
            sampler_busy.clone(),
            p.clone(),
            credits.clone(),
        );
        tasks.spawn_on(
            async move {
                let mut last = 0;
                while !pc.stop.load(SeqCst) {
                    let through = published.lock().unwrap().through;
                    let completed = {
                        let _io = busy.io();
                        let started = Instant::now();
                        let result = store
                            .fetch_completed_batches(run, task, 100, true, Some(last))
                            .await?;
                        pc.stages.lock().unwrap()[5] += started.elapsed().as_secs_f64();
                        result
                    };
                    let completed: Vec<_> = completed
                        .into_iter()
                        .take_while(|b| b.batch_id <= through as i64)
                        .collect();
                    if completed.is_empty() {
                        pc.empty_fetches.fetch_add(1, SeqCst);
                        tokio::time::sleep(Duration::from_millis(1)).await;
                        continue;
                    }
                    last = completed.last().unwrap().batch_id;
                    let count = completed.len();
                    ensure!(
                        completed
                            .iter()
                            .all(|b| b.result.values.is_some() == b.requires_training_values),
                        "missing feedback"
                    );
                    {
                        let _io = busy.io();
                        let started = Instant::now();
                        ensure!(
                            store
                                .cleanup_consumed_completed_batches(run, last, count)
                                .await?
                                == count as u64,
                            "cleanup did not remove the consumed prefix"
                        );
                        pc.stages.lock().unwrap()[6] += started.elapsed().as_secs_f64();
                    }
                    pc.collected.fetch_add(count as u64, SeqCst);
                    credits_c.add_permits(count);
                }
                Ok(())
            },
            sampler_runtime.handle(),
        );
        let warmup_started = Instant::now();
        while warmup_started.elapsed().as_secs_f64() < c.warmup_seconds
            || p.collected.load(SeqCst) < 4
        {
            if let Some(result) = tasks.try_join_next() {
                result??;
                anyhow::bail!("I/O actor exited during warmup");
            }
            ensure!(
                warmup_started.elapsed() < Duration::from_secs(30),
                "sampler warmup made insufficient progress"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        p.measuring.store(true, SeqCst);
        let first = snapshot(&p, &sampler_busy, &evaluator_busy);
        while first.at.elapsed().as_secs_f64() < c.duration_seconds
            || p.collected.load(SeqCst) - first.collected < 16
        {
            if let Some(result) = tasks.try_join_next() {
                result??;
                anyhow::bail!("I/O actor exited during measurement");
            }
            if first.at.elapsed().as_secs_f64() >= c.duration_seconds.max(30.) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        windows.push(window(first, snapshot(&p, &sampler_busy, &evaluator_busy)));
        p.measuring.store(false, SeqCst);
        p.writers_done.store(true, SeqCst);
        let drain = async {
            while !writers.is_empty() {
                tokio::select! {
                    result = writers.join_next() => { result.unwrap()??; }
                    result = tasks.join_next() => { result.unwrap()??; anyhow::bail!("I/O actor exited before drain"); }
                }
            }
            while p.collected.load(SeqCst) < p.published.load(SeqCst) {
                if let Some(result) = tasks.try_join_next() {
                    result??;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            Ok::<_, anyhow::Error>(())
        };
        tokio::time::timeout(Duration::from_secs(30), drain).await??;
        p.stop.store(true, SeqCst);
        while let Some(result) = tasks.join_next().await {
            result??;
        }
    } else {
        // Bounded prefilled passes; retain two reserve batches. Setup and cleanup
        // are excluded, and every timed pass is retained rather than selecting the best.
        let count = slots - 2;
        let (mut elapsed, mut warmup, mut next) = (0., 0., 1_i64);
        while elapsed < c.duration_seconds
            || windows.iter().map(|w| w.submitted_batches).sum::<u64>() < 16
        {
            let end = next + slots as i64;
            while next < end {
                let size = bundle.min((end - next) as usize);
                let ids: Vec<_> = (next..next + size as i64).collect();
                sampler
                    .insert_batches(run, task, c.feedback, &ids, &prepared.batches[..size])
                    .await?;
                next += size as i64;
            }
            p.measuring.store(warmup >= c.warmup_seconds, SeqCst);
            let first = snapshot(&p, &sampler_busy, &evaluator_busy);
            tasks.spawn_on(
                consumer(
                    stores[0].clone(),
                    run,
                    uuids[0].clone(),
                    prepared.clone(),
                    evaluator_busy[0].clone(),
                    p.clone(),
                    Some(count as u64),
                ),
                evaluator_runtime.handle(),
            );
            tasks.join_next().await.unwrap()??;
            let measured = window(first, snapshot(&p, &sampler_busy, &evaluator_busy));
            p.measuring.store(false, SeqCst);
            if warmup >= c.warmup_seconds {
                elapsed += measured.elapsed_seconds;
                windows.push(measured);
            } else {
                warmup += measured.elapsed_seconds;
            }
            sqlx::query("DELETE FROM batches WHERE run_id=$1")
                .bind(run)
                .execute(sampler.pool())
                .await?;
        }
    }
    if let Some(result) = heartbeats.try_join_next() {
        result??;
    }
    let seconds: f64 = windows.iter().map(|w| w.elapsed_seconds).sum();
    let batches: u64 = windows
        .iter()
        .map(|w| {
            if c.role == "sampler" {
                w.collected_batches
            } else {
                w.submitted_batches
            }
        })
        .sum();
    let sampler_io = windows.iter().map(|w| w.sampler_io_seconds).sum::<f64>()
        / windows
            .iter()
            .map(|w| w.sampler_elapsed_seconds)
            .sum::<f64>();
    let evaluator_io = windows.iter().map(|w| w.evaluator_io_seconds).sum::<f64>()
        / windows
            .iter()
            .map(|w| w.evaluator_elapsed_seconds)
            .sum::<f64>();
    let measured_io = if c.role == "sampler" {
        sampler_io
    } else {
        evaluator_io
    };
    Ok(
        json!({"schema_version":1,"role":c.role,"batch_size":c.batch_size,"feedback":c.feedback,
        "elapsed_seconds":seconds,"batches":batches,"samples_per_second":batches as f64*c.batch_size as f64/seconds,
        "batches_per_second":batches as f64/seconds,"sampler_io_busy":sampler_io,"evaluator_io_busy":evaluator_io,
        "measured_io_busy":measured_io,"input_bytes_per_batch":prepared.input_bytes,"feedback_bytes_per_batch":prepared.feedback_bytes,
        "io_threads":c.io_threads,"consumers":c.consumers,"queue_slots":slots,"insert_bundle":bundle,
        "insert_concurrency":c.insert_concurrency,"sampler_pool_size":c.insert_concurrency+2,
        "sampler_stage_seconds":STAGES.into_iter().map(|name|(name,windows.iter().map(|w|w.sampler_stage_seconds[name]).sum::<f64>())).collect::<BTreeMap<_,_>>(),
        "database_activity_samples":*p.database_activity.lock().unwrap(),
        "windows":windows,
        "scope":"Production store operations; prepared 6D inputs and f(x)=x[0] results; excludes model compute, process IPC, runner ticks and checkpointing. Busy is occupied I/O wall time, including database waits, with overlap counted once."}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_sweep_keeps_queue_depth_and_bundling_constant() {
        for size in [256, 4096, 16384, 65536, 131072] {
            assert_eq!(queue_shape(size, 2048, "sampler", 64), (64, 5));
        }
        // Overrides retain the memory bound and expose the changed comparison.
        assert_eq!(queue_shape(1_048_576, 2048, "sampler", 64), (12, 1));
        assert_eq!(queue_shape(1_048_576, 16384, "sampler", 64), (64, 5));
    }
    #[test]
    fn collection_cannot_skip_late_insert_commits() {
        let mut p = Published::default();
        p.complete(6, 10);
        assert_eq!(p.through, 0);
        p.complete(1, 5);
        assert_eq!(p.through, 10);
        p.complete(16, 20);
        assert_eq!(p.through, 10);
        p.complete(11, 15);
        assert_eq!(p.through, 20);
    }
}
