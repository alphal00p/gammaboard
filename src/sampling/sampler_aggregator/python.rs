use std::fs;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;
use std::{env, ffi::OsString};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::{BuildError, EngineError};
use crate::evaluation::{Batch, Point};
use crate::sampling::{LatentBatchSpec, SamplePlan, SamplerAggregator, SamplerAggregatorSnapshot};
use crate::utils::domain::Domain;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PythonHomogeneousMonteCarloSamplerParams {
    pub flake_ref: String,
    pub module: String,
    pub class: String,
    pub input_dim: usize,
    #[serde(default = "default_init_args")]
    pub init_args: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PythonHomogeneousMonteCarloSnapshot {
    params: PythonHomogeneousMonteCarloSamplerParams,
    sampler_state: Value,
}

pub struct PythonHomogeneousMonteCarloSampler {
    params: PythonHomogeneousMonteCarloSamplerParams,
    worker: Mutex<PythonSamplerWorker>,
}

impl PythonHomogeneousMonteCarloSampler {
    pub(crate) fn from_params_and_domain(
        params: PythonHomogeneousMonteCarloSamplerParams,
        domain: &Domain,
    ) -> Result<Self, BuildError> {
        validate_homogeneous_domain(domain, params.input_dim)?;
        let worker = PythonSamplerWorker::spawn(&params, None)?;
        Ok(Self {
            params,
            worker: Mutex::new(worker),
        })
    }

    pub(crate) fn from_snapshot(
        snapshot: PythonHomogeneousMonteCarloSnapshot,
        domain: &Domain,
    ) -> Result<Self, BuildError> {
        validate_homogeneous_domain(domain, snapshot.params.input_dim)?;
        let worker = PythonSamplerWorker::spawn(&snapshot.params, Some(snapshot.sampler_state))?;
        Ok(Self {
            params: snapshot.params,
            worker: Mutex::new(worker),
        })
    }
}

impl SamplerAggregator for PythonHomogeneousMonteCarloSampler {
    fn validate_domain(&self, domain: &Domain) -> Result<(), BuildError> {
        validate_homogeneous_domain(domain, self.params.input_dim)
    }

    fn training_samples_remaining(&self) -> Option<usize> {
        let mut worker = match self.worker.lock() {
            Ok(guard) => guard,
            Err(_) => return None,
        };
        worker.training_samples_remaining().ok().flatten()
    }

    fn sample_plan(&mut self) -> Result<SamplePlan, EngineError> {
        self.worker_mut()?.sample_plan()
    }

    fn produce_latent_batch(&mut self, nr_samples: usize) -> Result<LatentBatchSpec, EngineError> {
        if nr_samples == 0 {
            return Err(EngineError::engine(
                "python_homogeneous_monte_carlo sampler requires nr_samples > 0",
            ));
        }
        let xs_row_major = self.worker_mut()?.produce_latent_batch(nr_samples)?;
        let mut points = Vec::with_capacity(nr_samples);
        for sample_idx in 0..nr_samples {
            let start = sample_idx * self.params.input_dim;
            let end = start + self.params.input_dim;
            points.push(Point::new(
                xs_row_major[start..end].to_vec(),
                Vec::new(),
                1.0,
            ));
        }
        let batch = Batch::new(points).map_err(|err| EngineError::engine(err.to_string()))?;
        Ok(LatentBatchSpec::from_batch(&batch))
    }

    fn ingest_training_weights(&mut self, training_weights: &[f64]) -> Result<(), EngineError> {
        self.worker_mut()?.ingest_training_weights(training_weights)
    }

    fn snapshot(&mut self) -> Result<SamplerAggregatorSnapshot, EngineError> {
        let sampler_state = self.worker_mut()?.snapshot()?;
        let raw = serde_json::to_value(PythonHomogeneousMonteCarloSnapshot {
            params: self.params.clone(),
            sampler_state,
        })
        .map_err(EngineError::from)?;
        Ok(SamplerAggregatorSnapshot::PythonHomogeneousMonteCarlo { raw })
    }

    fn get_diagnostics(&mut self) -> Value {
        self.worker_mut()
            .and_then(|mut worker| worker.get_diagnostics())
            .unwrap_or_else(|_| serde_json::json!({}))
    }
}

impl PythonHomogeneousMonteCarloSampler {
    fn worker_mut(&self) -> Result<std::sync::MutexGuard<'_, PythonSamplerWorker>, EngineError> {
        self.worker
            .lock()
            .map_err(|_| EngineError::engine("python sampler worker mutex poisoned"))
    }
}

fn validate_homogeneous_domain(domain: &Domain, input_dim: usize) -> Result<(), BuildError> {
    let (continuous_dims, discrete_dims) = domain.fixed_rectangular_dims().ok_or_else(|| {
        BuildError::build(
            "python_homogeneous_monte_carlo sampler requires a fixed rectangular domain",
        )
    })?;
    if discrete_dims != 0 {
        return Err(BuildError::build(format!(
            "python_homogeneous_monte_carlo sampler requires discrete_dims=0, got {}",
            discrete_dims
        )));
    }
    if continuous_dims != input_dim {
        return Err(BuildError::build(format!(
            "python_homogeneous_monte_carlo sampler expects input_dim={}, got {} from domain",
            input_dim, continuous_dims
        )));
    }
    Ok(())
}

struct PythonSamplerWorker {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    stderr: ChildStderr,
    next_id: u64,
    input_dim: usize,
}

impl PythonSamplerWorker {
    fn spawn(
        params: &PythonHomogeneousMonteCarloSamplerParams,
        snapshot: Option<Value>,
    ) -> Result<Self, BuildError> {
        if !params.init_args.is_object() {
            return Err(BuildError::build(
                "python_homogeneous_monte_carlo init_args must be a TOML table / JSON object",
            ));
        }
        let flake_ref = normalize_flake_ref(&params.flake_ref);
        let output_path = build_nix_output_path(&flake_ref)?;
        let python_executable = resolve_python_executable(&output_path).ok_or_else(|| {
            BuildError::build(format!(
                "python executable not found under '{}' (expected one of: bin/python, bin/python3)",
                output_path.display()
            ))
        })?;

        let mut command = Command::new(&python_executable);
        let worker_script = sampler_worker_script_path()?;
        command
            .arg("-u")
            .arg(worker_script)
            .current_dir(&output_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(pythonpath) = build_worker_pythonpath(&output_path) {
            command.env("PYTHONPATH", pythonpath);
        }

        let mut child = command.spawn().map_err(|error| {
            BuildError::build(format!(
                "failed to start python sampler worker '{}': {error}",
                python_executable.display()
            ))
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| BuildError::build("python sampler worker stdin not available"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| BuildError::build("python sampler worker stdout not available"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| BuildError::build("python sampler worker stderr not available"))?;

        let mut worker = Self {
            child,
            stdin: BufWriter::new(stdin),
            stdout: BufReader::new(stdout),
            stderr,
            next_id: 1,
            input_dim: params.input_dim,
        };
        worker.send_init(
            &params.module,
            &params.class,
            params.input_dim,
            params.init_args.clone(),
            snapshot,
        )?;
        Ok(worker)
    }

    fn send_init(
        &mut self,
        module: &str,
        class: &str,
        input_dim: usize,
        init_args: Value,
        snapshot: Option<Value>,
    ) -> Result<(), BuildError> {
        let request = serde_json::json!({
            "id": self.allocate_request_id(),
            "op": "init",
            "module": module,
            "class": class,
            "input_dim": input_dim,
            "init_args": init_args,
            "snapshot": snapshot,
        });
        let response = self.send_request(&request).map_err(BuildError::build)?;
        Self::expect_ok(response).map_err(BuildError::build)
    }

    fn training_samples_remaining(&mut self) -> Result<Option<usize>, EngineError> {
        let request = serde_json::json!({
            "id": self.allocate_request_id(),
            "op": "training_samples_remaining",
        });
        let response = self.send_request(&request).map_err(EngineError::engine)?;
        Self::expect_ok(response.clone()).map_err(EngineError::engine)?;
        let remaining = match response.get("remaining") {
            Some(Value::Null) | None => None,
            Some(value) => Some(value.as_u64().ok_or_else(|| {
                EngineError::engine("python sampler response field 'remaining' must be u64 or null")
            })? as usize),
        };
        Ok(remaining)
    }

    fn sample_plan(&mut self) -> Result<SamplePlan, EngineError> {
        let request = serde_json::json!({
            "id": self.allocate_request_id(),
            "op": "sample_plan",
        });
        let response = self.send_request(&request).map_err(EngineError::engine)?;
        Self::expect_ok(response.clone()).map_err(EngineError::engine)?;
        let plan_value = response
            .get("plan")
            .cloned()
            .ok_or_else(|| EngineError::engine("python sampler response missing 'plan'"))?;
        serde_json::from_value(plan_value).map_err(|err| {
            EngineError::engine(format!("invalid python sample_plan payload: {err}"))
        })
    }

    fn produce_latent_batch(&mut self, nr_samples: usize) -> Result<Vec<f64>, EngineError> {
        let request = serde_json::json!({
            "id": self.allocate_request_id(),
            "op": "produce_latent_batch",
            "nr_samples": nr_samples,
        });
        let response = self.send_request(&request).map_err(EngineError::engine)?;
        Self::expect_ok(response.clone()).map_err(EngineError::engine)?;
        let xs = response
            .get("xs_row_major")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                EngineError::engine("python sampler response missing 'xs_row_major' array")
            })?;
        let expected_len = nr_samples.saturating_mul(self.input_dim);
        if xs.len() != expected_len {
            return Err(EngineError::engine(format!(
                "python sampler output size mismatch: expected {} values, got {}",
                expected_len,
                xs.len()
            )));
        }
        xs.iter()
            .enumerate()
            .map(|(index, value)| {
                value.as_f64().ok_or_else(|| {
                    EngineError::engine(format!(
                        "python sampler returned non-f64 value at xs_row_major[{index}]"
                    ))
                })
            })
            .collect()
    }

    fn ingest_training_weights(&mut self, training_weights: &[f64]) -> Result<(), EngineError> {
        let request = serde_json::json!({
            "id": self.allocate_request_id(),
            "op": "ingest_training_weights",
            "training_weights": training_weights,
        });
        let response = self.send_request(&request).map_err(EngineError::engine)?;
        Self::expect_ok(response).map_err(EngineError::engine)
    }

    fn snapshot(&mut self) -> Result<Value, EngineError> {
        let request = serde_json::json!({
            "id": self.allocate_request_id(),
            "op": "snapshot",
        });
        let response = self.send_request(&request).map_err(EngineError::engine)?;
        Self::expect_ok(response.clone()).map_err(EngineError::engine)?;
        response
            .get("snapshot")
            .cloned()
            .ok_or_else(|| EngineError::engine("python sampler response missing 'snapshot'"))
    }

    fn get_diagnostics(&mut self) -> Result<Value, EngineError> {
        let request = serde_json::json!({
            "id": self.allocate_request_id(),
            "op": "get_diagnostics",
        });
        let response = self.send_request(&request).map_err(EngineError::engine)?;
        Self::expect_ok(response.clone()).map_err(EngineError::engine)?;
        Ok(response
            .get("diagnostics")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({})))
    }

    fn send_request(&mut self, request: &Value) -> Result<Value, String> {
        let line = serde_json::to_string(request)
            .map_err(|error| format!("failed to serialize python sampler request: {error}"))?;
        self.stdin
            .write_all(line.as_bytes())
            .map_err(|error| format!("failed writing request to python sampler worker: {error}"))?;
        self.stdin
            .write_all(b"\n")
            .map_err(|error| format!("failed writing newline to python sampler worker: {error}"))?;
        self.stdin.flush().map_err(|error| {
            format!("failed flushing request to python sampler worker: {error}")
        })?;

        let mut response_line = String::new();
        let read = self
            .stdout
            .read_line(&mut response_line)
            .map_err(|error| format!("failed reading python sampler worker response: {error}"))?;
        if read == 0 {
            return Err(self
                .worker_terminated_message("python sampler worker terminated before responding"));
        }
        let response = serde_json::from_str::<Value>(response_line.trim()).map_err(|error| {
            format!(
                "failed to parse python sampler response as JSON: {error}; response='{}'",
                response_line.trim()
            )
        })?;
        let expected_id = request
            .get("id")
            .and_then(Value::as_u64)
            .ok_or_else(|| "python sampler request missing numeric 'id'".to_string())?;
        let actual_id = response
            .get("id")
            .and_then(Value::as_u64)
            .ok_or_else(|| "python sampler response missing numeric 'id'".to_string())?;
        if actual_id != expected_id {
            return Err(format!(
                "python sampler worker returned mismatched response id: expected {expected_id}, got {actual_id}"
            ));
        }
        Ok(response)
    }

    fn expect_ok(response: Value) -> Result<(), String> {
        let ok = response
            .get("ok")
            .and_then(Value::as_bool)
            .ok_or_else(|| "python sampler response missing boolean 'ok'".to_string())?;
        if ok {
            return Ok(());
        }
        let error = response
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("unknown python sampler worker error");
        let traceback = response
            .get("traceback")
            .and_then(Value::as_str)
            .unwrap_or("");
        if traceback.is_empty() {
            Err(format!("python sampler worker error: {error}"))
        } else {
            Err(format!("python sampler worker error: {error}\n{traceback}"))
        }
    }

    fn allocate_request_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    fn worker_terminated_message(&mut self, context: &str) -> String {
        let mut stderr = String::new();
        let _ = self.stderr.read_to_string(&mut stderr);
        let status = self
            .child
            .try_wait()
            .ok()
            .flatten()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "running".to_string());
        if stderr.trim().is_empty() {
            format!("{context}; status={status}; stderr=<empty>")
        } else {
            format!("{context}; status={status}; stderr='{}'", stderr.trim())
        }
    }
}

impl Drop for PythonSamplerWorker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn build_nix_output_path(flake_ref: &str) -> Result<PathBuf, BuildError> {
    let output = Command::new("nix")
        .arg("build")
        .arg(flake_ref)
        .arg("--no-link")
        .arg("--print-out-paths")
        .output()
        .map_err(|error| BuildError::build(format!("failed to start nix build: {error}")))?;
    if !output.status.success() {
        return Err(BuildError::build(format!(
            "nix build failed for '{}': exit={} stderr='{}' stdout='{}'",
            flake_ref,
            output.status,
            String::from_utf8_lossy(&output.stderr).trim(),
            String::from_utf8_lossy(&output.stdout).trim()
        )));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let output_path = stdout
        .lines()
        .find(|line| !line.trim().is_empty())
        .ok_or_else(|| {
            BuildError::build(format!(
                "nix build did not return an output path for '{}'",
                flake_ref
            ))
        })?;
    Ok(PathBuf::from(output_path.trim()))
}

fn normalize_flake_ref(reference: &str) -> String {
    if reference.contains("://")
        || reference.starts_with("flake:")
        || reference.starts_with("path:")
        || reference.starts_with("github:")
        || reference.starts_with("git+")
        || reference.starts_with("tarball+")
    {
        return reference.to_string();
    }

    let maybe_path = Path::new(reference);
    if maybe_path.is_absolute()
        || reference.starts_with("./")
        || reference.starts_with("../")
        || reference.starts_with('~')
    {
        return format!("path:{reference}");
    }

    if reference.contains('#') {
        let mut parts = reference.splitn(2, '#');
        let base = parts.next().unwrap_or_default();
        let fragment = parts.next().unwrap_or_default();
        let base_path = Path::new(base);
        if base_path.is_absolute() || base.starts_with("./") || base.starts_with("../") {
            return format!("path:{base}#{fragment}");
        }
    }

    reference.to_string()
}

fn resolve_python_executable(output_path: &Path) -> Option<PathBuf> {
    let candidates = [
        output_path.join("bin/python"),
        output_path.join("bin/python3"),
        output_path.join("python"),
    ];
    candidates.into_iter().find(|path| is_executable(path))
}

fn build_worker_pythonpath(runtime_root: &Path) -> Option<OsString> {
    let mut entries: Vec<PathBuf> = vec![runtime_root.to_path_buf(), runtime_root.join("src")];
    entries.retain(|path| path.is_dir());
    if let Some(existing) = env::var_os("PYTHONPATH") {
        entries.extend(env::split_paths(&existing));
    }
    if entries.is_empty() {
        None
    } else {
        env::join_paths(entries).ok()
    }
}

fn sampler_worker_script_path() -> Result<PathBuf, BuildError> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("integrand_api")
        .join("python_workers")
        .join("sampler_worker.py");
    if !path.is_file() {
        return Err(BuildError::build(format!(
            "python sampler worker script not found at '{}'",
            path.display()
        )));
    }
    Ok(path)
}

fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = fs::metadata(path) {
            return metadata.permissions().mode() & 0o111 != 0;
        }
    }
    #[cfg(not(unix))]
    {
        return true;
    }
    false
}

fn default_init_args() -> Value {
    Value::Object(serde_json::Map::new())
}
