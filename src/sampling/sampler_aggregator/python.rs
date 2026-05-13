use std::fs;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;
use std::{env, ffi::OsString};

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::core::{BuildError, EngineError, PythonRuntimeConfig};
use crate::evaluation::{Batch, Point};
use crate::sampling::{
    LatentBatchSpec, PdfPoint, SamplePlan, SamplerAggregator, SamplerAggregatorSnapshot,
};
use crate::utils::domain::Domain;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PythonSamplerParams {
    pub runtime: PythonRuntimeConfig,
    pub module: String,
    pub class: String,
    pub continuous_dims: usize,
    #[serde(default)]
    pub requires_training_values: bool,
    #[serde(default = "default_init_args")]
    pub init_args: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PythonSamplerParamsRaw {
    runtime: Option<PythonRuntimeConfig>,
    flake_ref: Option<String>,
    module: String,
    class: String,
    continuous_dims: usize,
    #[serde(default)]
    requires_training_values: bool,
    #[serde(default = "default_init_args")]
    init_args: Value,
}

impl<'de> Deserialize<'de> for PythonSamplerParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = PythonSamplerParamsRaw::deserialize(deserializer)?;
        if let Some(flake_ref) = raw.flake_ref {
            return Err(serde::de::Error::custom(format!(
                "python_sampler no longer accepts 'flake_ref'. Replace it with: runtime = {{ kind = \"flake\", ref = \"{flake_ref}\" }}"
            )));
        }
        let runtime = raw.runtime.ok_or_else(|| {
            serde::de::Error::custom(
                "python_sampler requires: runtime = { kind = \"flake\", ref = \"...\" }",
            )
        })?;
        Ok(Self {
            runtime,
            module: raw.module,
            class: raw.class,
            continuous_dims: raw.continuous_dims,
            requires_training_values: raw.requires_training_values,
            init_args: raw.init_args,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PythonSamplerSnapshot {
    params: PythonSamplerParams,
    sampler_state: Value,
}

pub struct PythonSampler {
    params: PythonSamplerParams,
    discrete_cardinalities: Vec<usize>,
    worker: Mutex<PythonSamplerWorker>,
}

impl PythonSampler {
    pub(crate) fn from_params_and_domain(
        params: PythonSamplerParams,
        domain: &Domain,
    ) -> Result<Self, BuildError> {
        let discrete_cardinalities = validate_homogeneous_domain(domain, params.continuous_dims)?;
        let worker = PythonSamplerWorker::spawn(&params, &discrete_cardinalities, None)?;
        Ok(Self {
            params,
            discrete_cardinalities,
            worker: Mutex::new(worker),
        })
    }

    pub(crate) fn from_snapshot(
        snapshot: PythonSamplerSnapshot,
        domain: &Domain,
    ) -> Result<Self, BuildError> {
        let discrete_cardinalities =
            validate_homogeneous_domain(domain, snapshot.params.continuous_dims)?;
        let worker = PythonSamplerWorker::spawn(
            &snapshot.params,
            &discrete_cardinalities,
            Some(snapshot.sampler_state),
        )?;
        Ok(Self {
            params: snapshot.params,
            discrete_cardinalities,
            worker: Mutex::new(worker),
        })
    }
}

impl SamplerAggregator for PythonSampler {
    fn validate_domain(&self, domain: &Domain) -> Result<(), BuildError> {
        let discrete_cardinalities =
            validate_homogeneous_domain(domain, self.params.continuous_dims)?;
        if discrete_cardinalities != self.discrete_cardinalities {
            return Err(BuildError::build(format!(
                "python_sampler expects discrete_cardinalities={:?}, got {:?} from domain",
                self.discrete_cardinalities, discrete_cardinalities
            )));
        }
        Ok(())
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
                "python_sampler requires nr_samples > 0",
            ));
        }
        let sample_batch = self.worker_mut()?.produce_latent_batch(nr_samples)?;
        let mut points = Vec::with_capacity(nr_samples);
        let discrete_dims = self.discrete_cardinalities.len();
        for sample_idx in 0..nr_samples {
            let discrete_start = sample_idx * discrete_dims;
            let discrete_end = discrete_start + discrete_dims;
            let continuous_start = sample_idx * self.params.continuous_dims;
            let continuous_end = continuous_start + self.params.continuous_dims;
            points.push(Point::new(
                sample_batch.xs_continuous_row_major[continuous_start..continuous_end].to_vec(),
                sample_batch.xs_discrete_row_major[discrete_start..discrete_end].to_vec(),
                sample_batch.weights[sample_idx],
            ));
        }
        let batch = Batch::new(points).map_err(|err| EngineError::engine(err.to_string()))?;
        Ok(LatentBatchSpec::from_batch(&batch))
    }

    fn ingest_training_values(&mut self, training_values: &[f64]) -> Result<(), EngineError> {
        self.worker_mut()?.ingest_training_values(training_values)
    }

    fn pdf_batch(&mut self, points: &[PdfPoint]) -> Result<Vec<Option<f64>>, EngineError> {
        self.worker_mut()?.pdf_batch(points)
    }

    fn snapshot(&mut self) -> Result<SamplerAggregatorSnapshot, EngineError> {
        let sampler_state = self.worker_mut()?.snapshot()?;
        let raw = serde_json::to_value(PythonSamplerSnapshot {
            params: self.params.clone(),
            sampler_state,
        })
        .map_err(EngineError::from)?;
        Ok(SamplerAggregatorSnapshot::PythonSampler { raw })
    }

    fn get_diagnostics(&mut self) -> Value {
        self.worker_mut()
            .and_then(|mut worker| worker.get_diagnostics())
            .unwrap_or_else(|_| serde_json::json!({}))
    }
}

impl PythonSampler {
    fn worker_mut(&self) -> Result<std::sync::MutexGuard<'_, PythonSamplerWorker>, EngineError> {
        self.worker
            .lock()
            .map_err(|_| EngineError::engine("python sampler worker mutex poisoned"))
    }
}

fn validate_homogeneous_domain(
    domain: &Domain,
    expected_continuous_dims: usize,
) -> Result<Vec<usize>, BuildError> {
    let (actual_continuous_dims, actual_discrete_dims) = domain
        .fixed_rectangular_dims()
        .ok_or_else(|| BuildError::build("python_sampler requires a fixed rectangular domain"))?;
    if actual_continuous_dims != expected_continuous_dims {
        return Err(BuildError::build(format!(
            "python_sampler expects continuous_dims={}, got {} from domain",
            expected_continuous_dims, actual_continuous_dims
        )));
    }
    let discrete_cardinalities = domain
        .fixed_discrete_cardinalities()
        .ok_or_else(|| BuildError::build("python_sampler requires fixed discrete cardinalities"))?;
    if discrete_cardinalities.len() != actual_discrete_dims {
        return Err(BuildError::build(format!(
            "python_sampler domain cardinality mismatch: depth={}, cardinalities={:?}",
            actual_discrete_dims, discrete_cardinalities
        )));
    }
    Ok(discrete_cardinalities)
}

struct PythonSampleBatch {
    xs_discrete_row_major: Vec<i64>,
    xs_continuous_row_major: Vec<f64>,
    weights: Vec<f64>,
}

struct PythonSamplerWorker {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    stderr: ChildStderr,
    next_id: u64,
    discrete_cardinalities: Vec<usize>,
    continuous_dims: usize,
}

impl PythonSamplerWorker {
    fn spawn(
        params: &PythonSamplerParams,
        discrete_cardinalities: &[usize],
        snapshot: Option<Value>,
    ) -> Result<Self, BuildError> {
        if !params.init_args.is_object() {
            return Err(BuildError::build(
                "python_sampler init_args must be a TOML table / JSON object",
            ));
        }
        let PythonRuntimeConfig::Flake { reference } = &params.runtime;
        let flake_ref = normalize_flake_ref(reference);
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
            discrete_cardinalities: discrete_cardinalities.to_vec(),
            continuous_dims: params.continuous_dims,
        };
        worker.send_init(
            &params.module,
            &params.class,
            discrete_cardinalities,
            params.continuous_dims,
            params.init_args.clone(),
            snapshot,
        )?;
        Ok(worker)
    }

    fn send_init(
        &mut self,
        module: &str,
        class: &str,
        discrete_cardinalities: &[usize],
        continuous_dims: usize,
        init_args: Value,
        snapshot: Option<Value>,
    ) -> Result<(), BuildError> {
        let request = serde_json::json!({
            "id": self.allocate_request_id(),
            "op": "init",
            "module": module,
            "class": class,
            "discrete_cardinalities": discrete_cardinalities,
            "continuous_dims": continuous_dims,
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

    fn produce_latent_batch(
        &mut self,
        nr_samples: usize,
    ) -> Result<PythonSampleBatch, EngineError> {
        let request = serde_json::json!({
            "id": self.allocate_request_id(),
            "op": "produce_latent_batch",
            "nr_samples": nr_samples,
        });
        let response = self.send_request(&request).map_err(EngineError::engine)?;
        Self::expect_ok(response.clone()).map_err(EngineError::engine)?;
        let xs_discrete = response
            .get("xs_discrete_row_major")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                EngineError::engine("python sampler response missing 'xs_discrete_row_major' array")
            })?;
        let discrete_dims = self.discrete_cardinalities.len();
        let expected_discrete_len = nr_samples.saturating_mul(discrete_dims);
        if xs_discrete.len() != expected_discrete_len {
            return Err(EngineError::engine(format!(
                "python sampler discrete output size mismatch: expected {} values, got {}",
                expected_discrete_len,
                xs_discrete.len()
            )));
        }
        let xs_continuous = response
            .get("xs_continuous_row_major")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                EngineError::engine(
                    "python sampler response missing 'xs_continuous_row_major' array",
                )
            })?;
        let expected_continuous_len = nr_samples.saturating_mul(self.continuous_dims);
        if xs_continuous.len() != expected_continuous_len {
            return Err(EngineError::engine(format!(
                "python sampler continuous output size mismatch: expected {} values, got {}",
                expected_continuous_len,
                xs_continuous.len()
            )));
        }
        let xs_discrete_row_major = xs_discrete
            .iter()
            .enumerate()
            .map(|(index, value)| {
                value.as_i64().ok_or_else(|| {
                    EngineError::engine(format!(
                        "python sampler returned non-i64 value at xs_discrete_row_major[{index}]"
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let xs_continuous_row_major = xs_continuous
            .iter()
            .enumerate()
            .map(|(index, value)| {
                value.as_f64().ok_or_else(|| {
                    EngineError::engine(format!(
                        "python sampler returned non-f64 value at xs_continuous_row_major[{index}]"
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let weights = response
            .get("weights")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                EngineError::engine("python sampler response missing 'weights' array")
            })?;
        if weights.len() != nr_samples {
            return Err(EngineError::engine(format!(
                "python sampler weights size mismatch: expected {} values, got {}",
                nr_samples,
                weights.len()
            )));
        }
        let weights = weights
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let weight = value.as_f64().ok_or_else(|| {
                    EngineError::engine(format!(
                        "python sampler returned non-f64 value at weights[{index}]"
                    ))
                })?;
                if !weight.is_finite() || weight <= 0.0 {
                    return Err(EngineError::engine(format!(
                        "python sampler returned non-positive or non-finite value at weights[{index}]"
                    )));
                }
                Ok(weight)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(PythonSampleBatch {
            xs_discrete_row_major,
            xs_continuous_row_major,
            weights,
        })
    }

    fn ingest_training_values(&mut self, training_values: &[f64]) -> Result<(), EngineError> {
        let request = serde_json::json!({
            "id": self.allocate_request_id(),
            "op": "ingest_training_values",
            "training_values": training_values,
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

    fn pdf_batch(&mut self, points: &[PdfPoint]) -> Result<Vec<Option<f64>>, EngineError> {
        let discrete_dims = self.discrete_cardinalities.len();
        let continuous_dims = self.continuous_dims;
        let mut xs_discrete_row_major = Vec::with_capacity(points.len() * discrete_dims);
        let mut xs_continuous_row_major = Vec::with_capacity(points.len() * continuous_dims);
        for (index, point) in points.iter().enumerate() {
            if point.0.len() != discrete_dims {
                return Err(EngineError::engine(format!(
                    "python sampler pdf discrete dimension mismatch at point {index}: expected {}, got {}",
                    discrete_dims,
                    point.0.len()
                )));
            }
            if point.1.len() != continuous_dims {
                return Err(EngineError::engine(format!(
                    "python sampler pdf continuous dimension mismatch at point {index}: expected {}, got {}",
                    continuous_dims,
                    point.1.len()
                )));
            }
            xs_discrete_row_major.extend_from_slice(&point.0);
            xs_continuous_row_major.extend_from_slice(&point.1);
        }
        let request = serde_json::json!({
            "id": self.allocate_request_id(),
            "op": "pdf",
            "nr_samples": points.len(),
            "xs_discrete_row_major": xs_discrete_row_major,
            "xs_continuous_row_major": xs_continuous_row_major,
        });
        let response = self.send_request(&request).map_err(EngineError::engine)?;
        Self::expect_ok(response.clone()).map_err(EngineError::engine)?;
        match response.get("values") {
            Some(Value::Null) | None => Ok(vec![None; points.len()]),
            Some(Value::Array(values)) => {
                if values.len() != points.len() {
                    return Err(EngineError::engine(format!(
                        "python sampler pdf output size mismatch: expected {}, got {}",
                        points.len(),
                        values.len()
                    )));
                }
                values
                    .iter()
                    .enumerate()
                    .map(|(index, value)| {
                        if value.is_null() {
                            return Ok(None);
                        }
                        value.as_f64().map(Some).ok_or_else(|| {
                            EngineError::engine(format!(
                                "python sampler response field 'values[{index}]' must be f64 or null"
                            ))
                        })
                    })
                    .collect()
            }
            Some(_) => Err(EngineError::engine(
                "python sampler response field 'values' must be an array or null",
            )),
        }
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
        .join("python_api")
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

#[cfg(test)]
mod tests {
    use super::{PythonSamplerParams, validate_homogeneous_domain};
    use crate::utils::domain::Domain;

    #[test]
    fn python_sampler_rejects_legacy_flake_ref_with_migration_hint() {
        let err = toml::from_str::<PythonSamplerParams>(
            r#"
flake_ref = "path:./old#runtime"
module = "my_module"
class = "MySampler"
continuous_dims = 2
"#,
        )
        .expect_err("legacy flake_ref should fail")
        .to_string();

        assert!(err.contains("python_sampler no longer accepts 'flake_ref'"));
        assert!(
            err.contains("runtime = { kind = \"flake\", ref = \"path:./old#runtime\" }"),
            "{err}"
        );
    }

    #[test]
    fn python_sampler_domain_accepts_fixed_rectangular_mixed_dims() {
        assert_eq!(
            validate_homogeneous_domain(&Domain::rectangular(3, 2), 3).expect("valid domain"),
            vec![1, 1]
        );
    }

    #[test]
    fn python_sampler_domain_rejects_continuous_dim_mismatch() {
        let err = validate_homogeneous_domain(&Domain::rectangular(3, 1), 2)
            .expect_err("expected mismatch");
        assert!(err.to_string().contains("continuous_dims=2"));
    }
}
