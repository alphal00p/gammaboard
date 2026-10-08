use super::*;

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_symbolica_havana_pdf_two_bumps_e2e() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;

    let config = temp_config(
        r#"
name = "symbolica-havana-pdf-1d2d-e2e"

[evaluator]
kind = "symbolica"
expr = "1/((x-1/4)^2+(y-1/4)^2+1/40) + 1/((x-3/4)^2+(y-3/4)^2+1/40) + z"
args = ["x", "y", "z"]

[evaluator_runner_params]
performance_snapshot_interval_ms = 2000

[sampler_aggregator_runner_params]
performance_snapshot_interval_ms = 2000
min_tick_time_ms = 50
frontend_sync_interval_ms = 2000
db_pool_size = 2

[sampler_aggregator_runner_params.queue]

target_batch_eval_ms = 500.0

max_batch_size = 100000

max_batches_per_tick = 100
max_insert_bundle_size = 5
max_concurrent_insert_tasks = 8
completed_batch_fetch_limit = 100
max_batch_retries = 3

[[task_queue]]
name = "accumulator"
kind = "set_accumulator"
accumulator = "scalar"

[[task_queue]]
name = "havana-train"
kind = "sample"
[task_queue.stop_condition]
max_samples = 200000
[task_queue.sampler_aggregator.config]
kind = "havana_training"
seed = 0
bins = 64
samples_for_update = 16384
initial_training_rate = 0.1
final_training_rate = 0.001

[[task_queue]]
name = "pdf-2d"
kind = "pdf_adaptation_image"
batch_transforms = []

[task_queue.geometry]
offset = [0.0, 0.0, 0.0]
u_vector = [1.0, 0.0, 0.0]
v_vector = [0.0, 1.0, 0.0]
discrete = []

[task_queue.geometry.u_linspace]
start = 0.0
stop = 1.0
count = 128

[task_queue.geometry.v_linspace]
start = 0.0
stop = 1.0
count = 128
"#,
    );

    harness.add_run(&config);
    let run_id = harness.run_id("symbolica-havana-pdf-1d2d-e2e").await?;

    harness.start_nodes(&["w-1", "w-2"]).await?;
    harness.assign_node("w-1", "sampler_aggregator", "symbolica-havana-pdf-1d2d-e2e");
    harness.assign_node("w-2", "evaluator", "symbolica-havana-pdf-1d2d-e2e");

    let pdf_task_id: i64 = sqlx::query_scalar(
        "SELECT id FROM run_tasks WHERE run_id = $1 AND name = 'pdf-2d' LIMIT 1",
    )
    .bind(run_id)
    .fetch_one(&harness.pool)
    .await?;

    let expected_width = 128usize;
    let expected_height = 128usize;
    let expected_points = expected_width * expected_height;
    let mut seen_batch_ids = HashSet::<i64>::new();
    let mut seen_grid_points = HashSet::<(usize, usize)>::new();
    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    let mut min_z = f64::INFINITY;
    let mut max_z = f64::NEG_INFINITY;
    let mut observed_points = 0usize;
    let mut last_seen_batch_id = 0_i64;
    let sampling_deadline = Instant::now() + Duration::from_secs(180);
    while seen_grid_points.len() < expected_points {
        let rows: Vec<(i64, Vec<u8>)> = sqlx::query_as(
            r#"
            SELECT b.id, bi.latent_batch
            FROM batch_inputs bi
            JOIN batches b ON b.id = bi.batch_id
            WHERE b.run_id = $1 AND b.task_id = $2 AND b.id > $3
            ORDER BY b.id ASC
            "#,
        )
        .bind(run_id)
        .bind(pdf_task_id)
        .bind(last_seen_batch_id)
        .fetch_all(&harness.pool)
        .await?;

        for (batch_id, payload) in rows {
            last_seen_batch_id = last_seen_batch_id.max(batch_id);
            if !seen_batch_ids.insert(batch_id) {
                continue;
            }
            let latent = LatentBatch::from_bytes(&payload)?;
            let batch = latent.payload.as_batch()?;
            for point in batch.points() {
                assert_eq!(
                    point.continuous.len(),
                    3,
                    "expected 3 continuous dimensions for symbolica args x,y,z"
                );
                let x = point.continuous[0];
                let y = point.continuous[1];
                let z = point.continuous[2];
                assert!(x.is_finite() && y.is_finite() && z.is_finite());
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
                min_z = min_z.min(z);
                max_z = max_z.max(z);
                observed_points += 1;

                let u = (x * (expected_width - 1) as f64).round();
                let v = (y * (expected_height - 1) as f64).round();
                assert!(
                    (u - x * (expected_width - 1) as f64).abs() <= 1e-9,
                    "x={x} is off the 128-point linspace grid"
                );
                assert!(
                    (v - y * (expected_height - 1) as f64).abs() <= 1e-9,
                    "y={y} is off the 128-point linspace grid"
                );
                let u = u as usize;
                let v = v as usize;
                assert!(u < expected_width && v < expected_height);
                seen_grid_points.insert((u, v));
            }
        }

        if Instant::now() >= sampling_deadline {
            anyhow::bail!(
                "timed out while validating evaluator input points: observed_unique={} expected={} observed_points={} observed_batches={}",
                seen_grid_points.len(),
                expected_points,
                observed_points,
                seen_batch_ids.len()
            );
        }
        sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(
        seen_grid_points.len(),
        expected_points,
        "did not observe every raster point"
    );
    assert!(observed_points > 0, "expected at least one evaluated point");
    assert!(
        min_x >= -1e-12 && max_x <= 1.0 + 1e-12,
        "x outside [0,1]: min={min_x}, max={max_x}"
    );
    assert!(
        min_y >= -1e-12 && max_y <= 1.0 + 1e-12,
        "y outside [0,1]: min={min_y}, max={max_y}"
    );
    assert!(
        min_z >= -1e-12 && max_z <= 1e-12,
        "z should be fixed at 0: min={min_z}, max={max_z}"
    );

    harness
        .wait_for("all tasks complete", Duration::from_secs(180), || {
            let pool = harness.pool.clone();
            async move {
                let completed: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM run_tasks WHERE run_id = $1 AND state = 'completed'",
                )
                .bind(run_id)
                .fetch_one(&pool)
                .await?;
                Ok(completed == 3)
            }
        })
        .await?;

    let persisted = harness
        .latest_task_persisted_observable(run_id, "pdf-2d")
        .await?;
    let output: PdfAdaptationImagePersistedOutput = serde_json::from_value(persisted)?;
    assert!(
        output.global_pdf_norm.is_finite() && output.global_pdf_norm > 0.0,
        "expected positive global pdf norm, got {}",
        output.global_pdf_norm
    );
    assert!(
        output
            .global_abs_integrand_norm
            .is_some_and(|value| value.is_finite() && value > 0.0),
        "expected positive global integrand norm in persisted output"
    );

    let width = expected_width;
    let height = expected_height;
    assert_eq!(output.integrand_values.len(), width * height);

    let value_at = |u: usize, v: usize| -> f64 {
        output.integrand_values[v * width + u]
            .map(f64::abs)
            .filter(|value| value.is_finite())
            .unwrap_or(f64::NEG_INFINITY)
    };

    let mut local_maxima = Vec::<(usize, usize, f64)>::new();
    for v in 0..height {
        for u in 0..width {
            let center = value_at(u, v);
            if !center.is_finite() {
                continue;
            }
            let mut is_local_max = true;
            for dv in -1_i32..=1 {
                for du in -1_i32..=1 {
                    if du == 0 && dv == 0 {
                        continue;
                    }
                    let nu = u as i32 + du;
                    let nv = v as i32 + dv;
                    if nu < 0 || nv < 0 || nu >= width as i32 || nv >= height as i32 {
                        continue;
                    }
                    if value_at(nu as usize, nv as usize) > center {
                        is_local_max = false;
                        break;
                    }
                }
                if !is_local_max {
                    break;
                }
            }
            if is_local_max {
                local_maxima.push((u, v, center));
            }
        }
    }

    local_maxima.sort_by(|a, b| b.2.partial_cmp(&a.2).expect("finite maxima values"));
    assert!(
        local_maxima.len() >= 2,
        "expected at least two local maxima, got {}",
        local_maxima.len()
    );

    let first = local_maxima[0];
    let second = local_maxima
        .iter()
        .copied()
        .find(|(u, v, _)| {
            let du = (*u as isize - first.0 as isize).unsigned_abs();
            let dv = (*v as isize - first.1 as isize).unsigned_abs();
            du + dv >= 16
        })
        .ok_or_else(|| anyhow::anyhow!("failed to find a second distinct peak"))?;

    let to_param = |index: usize| -> f64 { index as f64 / (width - 1) as f64 };
    let (t1, s1) = (to_param(first.0), to_param(first.1));
    let (t2, s2) = (to_param(second.0), to_param(second.1));

    let near = |a: f64, b: f64| (a - b).abs() <= 0.08;
    let first_match = near(t1, 0.25) && near(s1, 0.25);
    let second_match = near(t2, 0.75) && near(s2, 0.75);
    let swapped_match = near(t1, 0.75) && near(s1, 0.75) && near(t2, 0.25) && near(s2, 0.25);

    assert!(
        (first_match && second_match) || swapped_match,
        "peak positions mismatch: first=({t1:.4},{s1:.4}) second=({t2:.4},{s2:.4})"
    );

    harness.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege, scalar-sin .venv, nix, and python+numpy"]
async fn full_stack_cli_python_scalar_venv_e2e() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;
    harness.start_nodes(&["w-1", "w-2"]).await?;

    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let evaluator_dir = manifest_dir.join("process_api/examples/python_scalar_sin");
    let evaluator_python = evaluator_dir.join(".venv/bin/python");
    let process_api_python = manifest_dir.join("process_api/python/src");
    let sampler_src = manifest_dir.join("process_api/examples/python_sampler_symbolica_havana/src");
    let sampler_pythonpath = format!("{}:{}", process_api_python.display(), sampler_src.display());
    let sampler_flake_ref = format!(
        "path:{}#runtime",
        manifest_dir
            .join("process_api/examples/python_sampler_symbolica_havana")
            .display()
    );
    let config = temp_config(&format!(
        r#"
name = "python-scalar-venv-e2e"

[evaluator]
kind = "process_evaluator"
command = ["env", "PYTHONPATH={}", "{}", "-u", "-m", "run_evaluator"]
cwd = "{}"
domain = {{ rectangular = {{ discrete_cardinalities = [2, 3], continuous_dims = 2 }} }}
args = {{ scale = 1.0, bias = 0.0, freq_u = 2.0, freq_v = 1.25 }}

[[task_queue]]
name = "accumulator"
kind = "set_accumulator"

[task_queue.accumulator]
kind = "vector"
components = ["value"]
training_projection = {{ kind = "component", name = "value" }}

[task_queue.accumulator.discrete_projections]
[[task_queue.accumulator.discrete_projections.items]]
name = "spin"
dims = [0]
fixed_dims = {{}}

[[task_queue.accumulator.discrete_projections.items]]
name = "channel_for_spin_0"
dims = [1]
fixed_dims = {{ "0" = 0 }}

[[task_queue]]
name = "sample-a"
kind = "sample"
stop_condition = {{ max_samples = 64 }}
sampler_aggregator = {{ config = {{ kind = "process_sampler", command = ["nix", "shell", "{sampler_flake_ref}", "-c", "env", "PYTHONPATH={sampler_pythonpath}", "python", "-u", "-m", "run_sampler"], requires_training_values = true, args = {{ seed = 0, bins = 8, samples_for_update = 8, stop_training_after_n_samples = 64, initial_training_rate = 0.1, final_training_rate = 0.01 }} }} }}
"#,
        process_api_python.display(),
        evaluator_python.display(),
        evaluator_dir.join("src").display(),
    ));

    harness.add_run(&config);
    let run_id = harness.run_id("python-scalar-venv-e2e").await?;

    harness.assign_node("w-1", "sampler_aggregator", "python-scalar-venv-e2e");
    harness.assign_node("w-2", "evaluator", "python-scalar-venv-e2e");

    harness
        .wait_for(
            "python scalar venv task completes",
            Duration::from_secs(120),
            || async {
                let (state, failure_reason): (String, Option<String>) = sqlx::query_as(
                    "SELECT state, failure_reason FROM run_tasks WHERE run_id = $1 AND name = 'sample-a'",
                )
                .bind(run_id)
                .fetch_one(&harness.pool)
                .await?;
                anyhow::ensure!(
                    state != "failed",
                    "sample-a failed: {}",
                    failure_reason.unwrap_or_else(|| "no failure_reason".to_string())
                );
                Ok(state == "completed")
            },
        )
        .await?;

    harness
        .wait_for(
            "python symbolica havana sampler checkpoint is persisted",
            Duration::from_secs(30),
            || async { Ok(harness.run_sampler_checkpoint(run_id).await?.is_some()) },
        )
        .await?;

    let completed_samples: i64 =
        sqlx::query_scalar("SELECT nr_completed_samples FROM runs WHERE id = $1")
            .bind(run_id)
            .fetch_one(&harness.pool)
            .await?;
    assert!(completed_samples >= 64);

    harness.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres, a local MadNIS Python runtime, and the bundled GammaLoop state"]
async fn madnis_metadata_and_batch_boundaries() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;

    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let process_api_python = manifest_dir.join("process_api/python/src");
    let madnis_src = manifest_dir.join("integrations/madnis/src");
    let gammaloop_state = std::env::var_os("GAMMABOARD_MADNIS_STATE_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest_dir.join("resources/states/epem_a_ttxh/LO/state"));
    let integrand_name =
        std::env::var("GAMMABOARD_MADNIS_INTEGRAND").unwrap_or_else(|_| "LO".into());
    let madnis_pythonpath = format!("{}:{}", process_api_python.display(), madnis_src.display());
    let default_madnis_python = manifest_dir.join("integrations/madnis/.venv/bin/python");
    let madnis_python = std::env::var("GAMMABOARD_MADNIS_PYTHON").unwrap_or_else(|_| {
        if default_madnis_python.is_file() {
            default_madnis_python.display().to_string()
        } else {
            "python".to_string()
        }
    });
    let run_name = format!("gammaloop-madnis-metadata-fuzz-{}", unique_suffix());
    let cases = [(1_usize, 1_usize, 2_usize), (2, 1, 3), (3, 2, 4)];

    let madnis_runtime_status = std::process::Command::new(&madnis_python)
        .args(["-c", "import run_sampler"])
        .env("PYTHONPATH", &madnis_pythonpath)
        .current_dir(manifest_dir.join("resources"))
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;
    anyhow::ensure!(
        madnis_runtime_status.success(),
        "failed to preflight MadNIS runtime"
    );

    harness.start_nodes(&["w-1", "w-2"]).await?;

    let mut task_toml = String::new();
    for (case_idx, (queue_max_batch_size, madnis_max_batch_size, samples_for_update)) in
        cases.iter().copied().enumerate()
    {
        task_toml.push_str(&format!(
            r#"

[[task_queue]]
name = "madnis-case-{case_idx}"
kind = "sample"
stop_condition = {{ max_samples = {samples_for_update} }}
accumulator = "latest"
sampler_aggregator = {{ config = {{ kind = "process_sampler", command = ["env", "PYTHONPATH={madnis_pythonpath}", "{madnis_python}", "-u", "-m", "run_sampler"], requires_training_values = true, args = {{ seed = {case_idx}, training_steps = 1, training_batch_size = {samples_for_update}, max_batch_size = {madnis_max_batch_size}, use_gpu = false, learning_rate = 0.001, use_scheduler = false, discrete_model = "made", discrete_dims_position = "first", flow_config = {{ layers = 1, units = 8, bins = 4 }}, made_config = {{ layers = 1, nodes_per_feature = 8 }} }} }} }}

[task_queue.queue_tuning]
max_batch_size = {queue_max_batch_size}
target_batch_eval_ms = 1.0
"#
        ));
    }

    let config = temp_config(&format!(
        r#"
name = "{run_name}"

[evaluator]
kind = "gammaloop"
state_folder = "{}"
integrand_name = "{integrand_name}"
training_projection = "abs"

[evaluator.preprocessing]
read_only = true
commands = [
  # Keep a rectangular graph axis while retaining native graph-aware maps.
  # Orientation and channel counts may differ between graphs, so sum those axes.
  "set process string '\n[sampling]\ngraphs = \"monte_carlo\"\norientations = \"summed\"\nsampling_multichanneling = true\nsampling_channels = \"summed\"\n'",
  "set model MT=173.0",
  "set model WT=0.0",
  "set model ymt=173.0",
  "set model aS=0.118",
  "set model aEWM1=132.507",
  "set model Gf=1.166390e-05",
  "set model MZ=91.188",
]

[sampler_aggregator_runner_params]
performance_snapshot_interval_ms = 100
min_tick_time_ms = 10
frontend_sync_interval_ms = 100
db_pool_size = 2

[evaluator_runner_params]
performance_snapshot_interval_ms = 100

[[task_queue]]
name = "accumulator"
kind = "set_accumulator"

[task_queue.accumulator]
kind = "vector"
components = ["real", "imag"]
training_projection = {{ kind = "component", name = "real" }}

{task_toml}
"#,
        gammaloop_state.display()
    ));

    harness.add_run(&config);
    let run_id = harness.run_id(&run_name).await?;

    harness.assign_node("w-1", "sampler_aggregator", &run_name);
    harness.assign_node("w-2", "evaluator", &run_name);

    for case_idx in 0..cases.len() {
        let task_name = format!("madnis-case-{case_idx}");
        harness
            .wait_for(
                format!("GammaLoop/MadNIS task {task_name} completes"),
                Duration::from_secs(300),
                || {
                    let pool = harness.pool.clone();
                    let task_name = task_name.clone();
                    async move {
                        let (state, failure_reason): (String, Option<String>) = sqlx::query_as(
                            "SELECT state, failure_reason FROM run_tasks WHERE run_id = $1 AND name = $2",
                        )
                        .bind(run_id)
                        .bind(&task_name)
                        .fetch_one(&pool)
                        .await?;
                        anyhow::ensure!(
                            state != "failed",
                            "{task_name} failed: {}",
                            failure_reason.unwrap_or_else(|| "no failure_reason".to_string())
                        );
                        Ok(state == "completed")
                    }
                },
            )
            .await?;
    }

    harness
        .wait_for(
            "MadNIS diagnostics include GammaLoop evaluator metadata",
            Duration::from_secs(30),
            || {
                let pool = harness.pool.clone();
                let gammaloop_state = gammaloop_state.clone();
                let integrand_name = integrand_name.clone();
                async move {
                    let diagnostics: Option<JsonValue> = sqlx::query_scalar(
                        r#"
                        SELECT engine_diagnostics
                        FROM sampler_aggregator_performance_latest
                        WHERE run_id = $1 AND worker_id = 'w-1'
                        "#,
                    )
                    .bind(run_id)
                    .fetch_optional(&pool)
                    .await?;
                    let Some(diagnostics) = diagnostics else {
                        return Ok(false);
                    };
                    let metadata = &diagnostics["gammaloop_metadata"];
                    Ok(metadata["state_folder"].as_str().is_some_and(|path| {
                        std::path::Path::new(path) == gammaloop_state.as_path()
                    }) && metadata["process_id"].as_u64().is_some()
                        && metadata["integrand_name"].as_str() == Some(integrand_name.as_str())
                        && metadata["coordinate_space"].as_str() == Some("x_space")
                        && diagnostics["produced_batches"].as_u64().unwrap_or(0) > 0)
                }
            },
        )
        .await?;

    let completed_samples: i64 =
        sqlx::query_scalar("SELECT nr_completed_samples FROM runs WHERE id = $1")
            .bind(run_id)
            .fetch_one(&harness.pool)
            .await?;
    let expected_samples: i64 = cases
        .iter()
        .map(|(_, _, samples_for_update)| *samples_for_update as i64)
        .sum();
    assert!(completed_samples >= expected_samples);

    harness.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege and python+numpy"]
async fn full_stack_cli_python_gammaloop_observable_process_api_e2e() -> anyhow::Result<()> {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let example_dir = manifest_dir.join("process_api/examples/python_gammaloop_observable");
    let default_python = example_dir.join(".venv/bin/python");
    let python = std::env::var("GAMMABOARD_GAMMALOOP_EXAMPLE_PYTHON").unwrap_or_else(|_| {
        if default_python.is_file() {
            default_python.display().to_string()
        } else {
            "python".to_string()
        }
    });
    let process_api_python = manifest_dir.join("process_api/python/src");
    let pythonpath = format!(
        "{}:{}",
        process_api_python.display(),
        example_dir.join("src").display()
    );

    let preflight_output = tokio::time::timeout(
        Duration::from_secs(30),
        TokioCommand::new(&python)
            .args(["-c", "import run_evaluator"])
            .env("PYTHONPATH", &pythonpath)
            .current_dir(example_dir.join("src"))
            .output(),
    )
    .await
    .map_err(|_| {
        anyhow::anyhow!(
            "timed out preflighting python_gammaloop_observable runtime with {} using PYTHONPATH={}",
            python,
            pythonpath
        )
    })??;
    anyhow::ensure!(
        preflight_output.status.success(),
        "failed to preflight python_gammaloop_observable runtime with {} using PYTHONPATH={}\nstdout:\n{}\nstderr:\n{}",
        python,
        pythonpath,
        String::from_utf8_lossy(&preflight_output.stdout),
        String::from_utf8_lossy(&preflight_output.stderr)
    );

    let mut harness = FullStackHarness::new().await?;
    harness.start_nodes(&["w-1", "w-2"]).await?;

    let run_name = format!("python-gammaloop-observable-e2e-{}", unique_suffix());
    let config = temp_config(&format!(
        r#"
name = "{run_name}"

[evaluator]
kind = "process_evaluator"
command = ["env", "PYTHONPATH={pythonpath}", "{python}", "-u", "-m", "run_evaluator"]
cwd = "{}"
domain = {{ continuous = {{ dims = 1 }} }}
accumulator = "gammaloop"

[[task_queue]]
name = "accumulator"
kind = "set_accumulator"

[task_queue.accumulator]
kind = "gammaloop"

[[task_queue]]
name = "sample"
kind = "sample"
stop_condition = {{ max_samples = 8 }}
accumulator = "latest"
sampler_aggregator = {{ config = {{ kind = "naive_monte_carlo", seed = 0 }} }}

[task_queue.queue_tuning]
max_batch_size = 8
target_batch_eval_ms = 1.0
"#,
        example_dir.join("src").display()
    ));

    harness.add_run(&config);
    let run_id = harness.run_id(&run_name).await?;
    harness.assign_node("w-1", "sampler_aggregator", &run_name);
    harness.assign_node("w-2", "evaluator", &run_name);

    sleep(Duration::from_secs(3)).await;
    let (startup_state, startup_failure): (String, Option<String>) = sqlx::query_as(
        "SELECT state, failure_reason FROM run_tasks WHERE run_id = $1 AND name = 'sample'",
    )
    .bind(run_id)
    .fetch_one(&harness.pool)
    .await?;
    anyhow::ensure!(
        startup_state != "failed",
        "sample failed during startup: {}",
        startup_failure.unwrap_or_else(|| "no failure_reason".to_string())
    );

    harness
        .wait_for("python gammaloop observable sample completes", Duration::from_secs(15), || {
            let pool = harness.pool.clone();
            async move {
                let (state, failure_reason): (String, Option<String>) = sqlx::query_as(
                    "SELECT state, failure_reason FROM run_tasks WHERE run_id = $1 AND name = 'sample'",
                )
                .bind(run_id)
                .fetch_one(&pool)
                .await?;
                anyhow::ensure!(
                    state != "failed",
                    "sample failed: {}",
                    failure_reason.unwrap_or_else(|| "no failure_reason".to_string())
                );
                Ok(state == "completed")
            }
        })
        .await?;

    let observable = harness
        .run_current_accumulator(run_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing current GammaLoop observable"))?;
    let real_count = observable
        .pointer("/estimate/components/0/state/count")
        .and_then(JsonValue::as_i64)
        .unwrap_or(0);
    let histogram_sample_count = observable
        .pointer("/bundle/histograms/x/sample_count")
        .and_then(JsonValue::as_i64)
        .unwrap_or(0);
    assert!(
        real_count >= 8,
        "expected real estimate count >= 8, got {real_count}; observable={observable}"
    );
    assert!(
        histogram_sample_count >= 8,
        "expected x histogram sample_count >= 8, got {histogram_sample_count}; observable={observable}"
    );

    let persisted = harness
        .latest_task_persisted_observable(run_id, "sample")
        .await?;
    assert_eq!(
        persisted
            .pointer("/bundle/histograms/x/title")
            .and_then(JsonValue::as_str),
        Some("x")
    );

    harness.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege, apptainer, and a working unprivileged Apptainer build setup"]
async fn full_stack_cli_rust_apptainer_process_evaluator_e2e() -> anyhow::Result<()> {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let example_dir = manifest_dir.join("process_api/examples/rust_breit_wigner_evaluator");
    let image_path = example_dir.join("runtime.sif");
    let _ = std::fs::remove_file(&image_path);
    let _image_cleanup = RemoveFileOnDrop(image_path.clone());

    let build_output = std::process::Command::new("apptainer")
        .arg("build")
        .arg("--force")
        .arg(&image_path)
        .arg("apptainer.def")
        .current_dir(&example_dir)
        .output()?;
    anyhow::ensure!(
        build_output.status.success(),
        "apptainer build failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&build_output.stdout),
        String::from_utf8_lossy(&build_output.stderr)
    );

    let mut harness = FullStackHarness::new().await?;
    harness.start_nodes(&["w-1", "w-2"]).await?;

    let config = temp_config(&format!(
        r#"
name = "rust-apptainer-process-evaluator-e2e"

[evaluator]
kind = "process_evaluator"
command = ["apptainer", "exec", "{}", "breit-wigner-worker"]
domain = {{ discrete = {{ axis_label = "d0", branches = [
  {{ index = 0, domain = {{ continuous = {{ dims = 3 }} }} }},
  {{ index = 1, domain = {{ discrete = {{ axis_label = "d1", branches = [
    {{ index = 0, domain = {{ continuous = {{ dims = 1 }} }} }},
    {{ index = 1, domain = {{ rectangular = {{ discrete_cardinalities = [5], continuous_dims = 5 }} }} }},
  ] }} }} }},
] }} }}
args = {{ masses = [0.25, 0.50, 0.75], widths = [0.04, 0.06, 0.05], channel_weights = [1.0, 0.7, 1.3] }}

[[task_queue]]
name = "accumulator"
kind = "set_accumulator"

[task_queue.accumulator]
kind = "vector"
components = ["value"]
training_projection = {{ kind = "component", name = "value" }}

[[task_queue]]
name = "sample-a"
kind = "sample"
stop_condition = {{ max_samples = 64 }}
sampler_aggregator = {{ config = {{ kind = "naive_monte_carlo" }} }}
"#,
        "../process_api/examples/rust_breit_wigner_evaluator/runtime.sif"
    ));

    harness.add_run(&config);
    let run_id = harness
        .run_id("rust-apptainer-process-evaluator-e2e")
        .await?;

    harness.assign_node(
        "w-1",
        "sampler_aggregator",
        "rust-apptainer-process-evaluator-e2e",
    );
    harness.assign_node("w-2", "evaluator", "rust-apptainer-process-evaluator-e2e");

    harness
        .wait_for(
            "rust apptainer process evaluator task completes",
            Duration::from_secs(120),
            || async {
                let (state, failure_reason): (String, Option<String>) = sqlx::query_as(
                    "SELECT state, failure_reason FROM run_tasks WHERE run_id = $1 AND name = 'sample-a'",
                )
                .bind(run_id)
                .fetch_one(&harness.pool)
                .await?;
                anyhow::ensure!(
                    state != "failed",
                    "sample-a failed: {}",
                    failure_reason.unwrap_or_else(|| "no failure_reason".to_string())
                );
                Ok(state == "completed")
            },
        )
        .await?;

    let completed_samples: i64 =
        sqlx::query_scalar("SELECT nr_completed_samples FROM runs WHERE id = $1")
            .bind(run_id)
            .fetch_one(&harness.pool)
            .await?;
    assert!(completed_samples >= 64);

    harness.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege and nginx"]
async fn full_stack_deploy_can_run_two_port_isolated_instances() -> anyhow::Result<()> {
    anyhow::ensure!(nginx_available(), "deployment tests require nginx");

    let mut harness = FullStackHarness::new().await?;
    let second_db = TestDatabase::create().await?;
    let frontend_build = temp_frontend_build();
    let password_hash = hash_password_for_tests("test-password");
    let server_config = temp_server_config(
        "127.0.0.1",
        4000,
        "http://localhost:8080",
        false,
        true,
        (&password_hash, "test-session-secret"),
    );
    let frontend_port_a = unused_local_port()?;
    let frontend_port_b = unused_local_port()?;
    let deploy_config_a =
        temp_deploy_server_config(frontend_build.path(), server_config.path(), frontend_port_a);
    let deploy_config_b =
        temp_deploy_server_config(frontend_build.path(), server_config.path(), frontend_port_b);
    let api_port_a = unused_local_port()?;
    let api_port_b = unused_local_port()?;

    for (label, database_url, frontend_port, api_port, deploy_config_path) in [
        (
            "deploy-a",
            harness.db.database_url.as_str(),
            frontend_port_a,
            api_port_a,
            deploy_config_a.path(),
        ),
        (
            "deploy-b",
            second_db.database_url.as_str(),
            frontend_port_b,
            api_port_b,
            deploy_config_b.path(),
        ),
    ] {
        let mut child = TokioCommand::new(&harness.bin_path);
        child
            .arg("--runtime-config")
            .arg(&harness.runtime_config_path)
            .arg("--database-url")
            .arg(database_url)
            .arg("deploy")
            .arg("--resume-workers")
            .arg("--server-config")
            .arg(deploy_config_path)
            .arg("--api-port")
            .arg(api_port.to_string())
            .arg("--allowed-origin")
            .arg(format!("http://localhost:{frontend_port}"))
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        let child = child.spawn()?;
        harness.children.push(ManagedChild {
            label: label.to_string(),
            child,
        });
    }

    let base_a = format!("http://127.0.0.1:{frontend_port_a}");
    let base_b = format!("http://127.0.0.1:{frontend_port_b}");
    harness
        .wait_for("first deploy API health", Duration::from_secs(20), || {
            let base = base_a.clone();
            async move {
                Ok(http_get(&base, "/api/health")
                    .await
                    .is_ok_and(|response| response.contains("\"status\":\"ok\"")))
            }
        })
        .await?;
    harness
        .wait_for("second deploy API health", Duration::from_secs(20), || {
            let base = base_b.clone();
            async move {
                Ok(http_get(&base, "/api/health")
                    .await
                    .is_ok_and(|response| response.contains("\"status\":\"ok\"")))
            }
        })
        .await?;

    assert!(http_get(&base_a, "/").await?.contains("gammaboard e2e"));
    assert!(http_get(&base_b, "/").await?.contains("gammaboard e2e"));

    harness.terminate_child("deploy-a").await?;
    harness.terminate_child("deploy-b").await?;
    harness.cleanup().await?;
    second_db.cleanup().await?;
    Ok(())
}

#[cfg(feature = "gammaloop")]
#[tokio::test]
#[ignore = "requires postgres and GAMMABOARD_TEST_REFERENCE_STATE from the acceptance fixture"]
async fn full_stack_gammaloop_reference_training_and_inference() -> anyhow::Result<()> {
    let state_folder = std::env::var_os("GAMMABOARD_TEST_REFERENCE_STATE").ok_or_else(|| {
        anyhow::anyhow!(
            "physics profile requires GAMMABOARD_TEST_REFERENCE_STATE from the acceptance fixture"
        )
    })?;
    let state_folder =
        toml::Value::String(state_folder.into_string().map_err(|_| {
            anyhow::anyhow!("GAMMABOARD_TEST_REFERENCE_STATE must be a UTF-8 path")
        })?)
        .to_string();
    let mut harness = FullStackHarness::new().await?;
    let config = temp_config(&format!(
        r#"
name = "gammaloop-reference-e2e"
[evaluator]
kind = "gammaloop"
state_folder = {state_folder}
integrand_name = "default"
reference_gaussian = {{ width = 1.5, center = [0.2, -0.3, 0.1] }}
[evaluator.preprocessing]
commands = ["""set process string '
[sampling]
graphs = "monte_carlo"
sampling_channels = "monte_carlo"
'"""]
[[task_queue]]
name = "train"
kind = "sample"
publish_result = false
stop_condition = {{ max_samples = 512 }}
accumulator = {{ config = "gammaloop" }}
sampler_aggregator = {{ config = {{ kind = "havana_training", seed = 21, bins = 16, samples_for_update = 128 }} }}
[[task_queue]]
name = "infer"
kind = "sample"
stop_condition = {{ max_samples = 2048 }}
accumulator = {{ config = "gammaloop" }}
sampler_aggregator = {{ config = {{ kind = "havana_inference", source = "latest_training_sampler_aggregator" }} }}
[sampler_aggregator_runner_params.queue]
max_batch_size = 64
fixed_batch_size = 64

"#
    ));
    harness.add_run(&config);
    let run_id = harness.run_id("gammaloop-reference-e2e").await?;
    harness
        .start_nodes(&["reference-s", "reference-e1", "reference-e2"])
        .await?;
    harness.assign_node(
        "reference-s",
        "sampler_aggregator",
        "gammaloop-reference-e2e",
    );
    harness.assign_node("reference-e1", "evaluator", "gammaloop-reference-e2e");
    harness.assign_node("reference-e2", "evaluator", "gammaloop-reference-e2e");
    harness
        .wait_for(
            "reference training and inference complete",
            Duration::from_secs(120),
            || async {
                let rows: Vec<(String, Option<String>)> =
                    sqlx::query_as("SELECT state, failure_reason FROM run_tasks WHERE run_id=$1")
                        .bind(run_id)
                        .fetch_all(&harness.pool)
                        .await?;
                for (state, reason) in &rows {
                    anyhow::ensure!(state != "failed", "reference task failed: {reason:?}");
                }
                Ok(rows.len() == 2 && rows.iter().all(|(state, _)| state == "completed"))
            },
        )
        .await?;
    let state: gammaboard::evaluation::GammaLoopAccumulatorState = serde_json::from_value(
        harness
            .run_current_accumulator(run_id)
            .await?
            .expect("reference accumulator"),
    )?;
    assert_eq!(state.diagnostics.count_total, 2048);
    assert_eq!(state.diagnostics.count_nan_or_unstable, 0);
    for (label, mean, stderr) in [
        ("normalization", state.real_mean(), state.real_stderr()),
        ("moment", state.imag_mean(), state.imag_stderr()),
    ] {
        eprintln!("reference inference {label}: {mean:.6} +/- {stderr:.6}");
        assert!(stderr.is_finite() && stderr < 0.15);
        assert!(
            (mean - 1.0).abs() < 6.0 * stderr,
            "{label}: {mean} +/- {stderr}"
        );
    }
    harness.cleanup().await
}
