use std::fs;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::{env, ffi::OsString};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::{AccumulatorConfig, BuildError, EvalError};
use crate::evaluation::{
    AccumulatorState, Batch, BatchResult, EvalBatchOptions, Evaluator, IngestScalar,
    ScalarBatchEvaluator,
};
use crate::utils::domain::Domain;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PythonScalarParams {
    pub flake_ref: String,
    pub module: String,
    pub class: String,
    pub continuous_dims: usize,
    #[serde(default)]
    pub discrete_cardinalities: Vec<usize>,
    #[serde(default = "default_init_args")]
    pub init_args: Value,
}

pub struct ScalarPythonEvaluator {
    continuous_dims: usize,
    discrete_cardinalities: Vec<usize>,
    worker: PythonWorker,
}

impl ScalarPythonEvaluator {
    pub fn from_params(params: PythonScalarParams) -> Result<Self, BuildError> {
        if !params.init_args.is_object() {
            return Err(BuildError::build(
                "python_scalar init_args must be a TOML table / JSON object",
            ));
        }
        if params
            .discrete_cardinalities
            .iter()
            .any(|value| *value == 0)
        {
            return Err(BuildError::build(
                "python_scalar discrete_cardinalities must contain only positive integers",
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
        let worker = PythonWorker::spawn(
            &python_executable,
            &output_path,
            &params.module,
            &params.class,
            &params.discrete_cardinalities,
            params.continuous_dims,
            params.init_args,
        )?;
        Ok(Self {
            continuous_dims: params.continuous_dims,
            discrete_cardinalities: params.discrete_cardinalities,
            worker,
        })
    }

    fn scalar_ingestor<'a>(
        state: &'a mut AccumulatorState,
    ) -> Result<&'a mut dyn IngestScalar, EvalError> {
        match state {
            AccumulatorState::Empty(accumulator) => Ok(accumulator),
            AccumulatorState::Scalar(accumulator) => Ok(accumulator),
            AccumulatorState::FullScalar(accumulator) => Ok(accumulator),
            other => Err(EvalError::eval(format!(
                "python_scalar evaluator does not support accumulator kind {}",
                other.kind_str()
            ))),
        }
    }
}

impl ScalarBatchEvaluator for ScalarPythonEvaluator {
    fn discrete_dims(&self) -> usize {
        self.discrete_cardinalities.len()
    }

    fn continuous_dims(&self) -> usize {
        self.continuous_dims
    }

    fn eval_scalar_rectangular_batch(
        &mut self,
        xs_discrete_row_major: &[i64],
        xs_continuous_row_major: &[f64],
        nr_samples: usize,
    ) -> Result<Vec<f64>, EvalError> {
        self.worker
            .eval_scalar(xs_discrete_row_major, xs_continuous_row_major, nr_samples)
    }
}

impl Evaluator for ScalarPythonEvaluator {
    fn get_domain(&self) -> Domain {
        Domain::rectangular_with_cardinalities(
            self.continuous_dims,
            self.discrete_cardinalities.clone(),
        )
    }

    fn eval_batch(
        &mut self,
        batch: &Batch,
        accumulator: &AccumulatorConfig,
        options: EvalBatchOptions,
    ) -> Result<BatchResult, EvalError> {
        let mut observable_state = AccumulatorState::from_config(accumulator);
        let weighted_values = self.eval_scalar_batch_into(
            batch,
            Self::scalar_ingestor(&mut observable_state)?,
            options.require_training_values,
        )?;
        Ok(BatchResult::new(weighted_values, observable_state))
    }
}

struct PythonWorker {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    stderr: ChildStderr,
    next_id: u64,
    discrete_cardinalities: Vec<usize>,
    continuous_dims: usize,
}

impl PythonWorker {
    fn spawn(
        python_executable: &Path,
        runtime_root: &Path,
        module: &str,
        class: &str,
        discrete_cardinalities: &[usize],
        continuous_dims: usize,
        init_args: Value,
    ) -> Result<Self, BuildError> {
        let mut command = Command::new(python_executable);
        let worker_script = evaluator_worker_script_path()?;
        command
            .arg("-u")
            .arg(worker_script)
            .current_dir(runtime_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(pythonpath) = build_worker_pythonpath(runtime_root) {
            command.env("PYTHONPATH", pythonpath);
        }

        let mut child = command.spawn().map_err(|error| {
            BuildError::build(format!(
                "failed to start python worker '{}': {error}",
                python_executable.display()
            ))
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| BuildError::build("python worker stdin not available"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| BuildError::build("python worker stdout not available"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| BuildError::build("python worker stderr not available"))?;

        let mut worker = Self {
            child,
            stdin: BufWriter::new(stdin),
            stdout: BufReader::new(stdout),
            stderr,
            next_id: 1,
            discrete_cardinalities: discrete_cardinalities.to_vec(),
            continuous_dims,
        };
        worker.send_init(
            module,
            class,
            discrete_cardinalities,
            continuous_dims,
            init_args,
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
    ) -> Result<(), BuildError> {
        let request = serde_json::json!({
            "id": self.allocate_request_id(),
            "op": "init",
            "module": module,
            "class": class,
            "discrete_cardinalities": discrete_cardinalities,
            "continuous_dims": continuous_dims,
            "init_args": init_args,
        });
        let response = self.send_request(&request).map_err(BuildError::build)?;
        Self::expect_ok(response).map_err(BuildError::build)
    }

    fn eval_scalar(
        &mut self,
        xs_discrete_row_major: &[i64],
        xs_continuous_row_major: &[f64],
        nr_samples: usize,
    ) -> Result<Vec<f64>, EvalError> {
        let request = serde_json::json!({
            "id": self.allocate_request_id(),
            "op": "eval_scalar",
            "nr_samples": nr_samples,
            "discrete_cardinalities": self.discrete_cardinalities,
            "continuous_dims": self.continuous_dims,
            "xs_discrete_row_major": xs_discrete_row_major,
            "xs_continuous_row_major": xs_continuous_row_major,
        });
        let response = self.send_request(&request).map_err(EvalError::eval)?;
        Self::expect_ok(response.clone()).map_err(EvalError::eval)?;
        let values = response
            .get("values")
            .and_then(Value::as_array)
            .ok_or_else(|| EvalError::eval("python worker response missing 'values'"))?;
        if values.len() != nr_samples {
            return Err(EvalError::eval(format!(
                "python evaluator output size mismatch: expected {}, got {}",
                nr_samples,
                values.len()
            )));
        }
        values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                value.as_f64().ok_or_else(|| {
                    EvalError::eval(format!(
                        "python worker returned non-f64 at output index {index}"
                    ))
                })
            })
            .collect()
    }

    fn send_request(&mut self, request: &Value) -> Result<Value, String> {
        let line = serde_json::to_string(request)
            .map_err(|error| format!("failed to serialize python request: {error}"))?;
        self.stdin
            .write_all(line.as_bytes())
            .map_err(|error| format!("failed writing request to python worker: {error}"))?;
        self.stdin
            .write_all(b"\n")
            .map_err(|error| format!("failed writing newline to python worker: {error}"))?;
        self.stdin
            .flush()
            .map_err(|error| format!("failed flushing request to python worker: {error}"))?;

        let mut response_line = String::new();
        let read = self
            .stdout
            .read_line(&mut response_line)
            .map_err(|error| format!("failed reading python worker response: {error}"))?;
        if read == 0 {
            return Err(
                self.worker_terminated_message("python worker terminated before responding")
            );
        }
        let response = serde_json::from_str::<Value>(response_line.trim()).map_err(|error| {
            format!(
                "failed to parse python worker response as JSON: {error}; response='{}'",
                response_line.trim()
            )
        })?;
        let expected_id = request
            .get("id")
            .and_then(Value::as_u64)
            .ok_or_else(|| "python request missing numeric 'id'".to_string())?;
        let actual_id = response
            .get("id")
            .and_then(Value::as_u64)
            .ok_or_else(|| "python response missing numeric 'id'".to_string())?;
        if actual_id != expected_id {
            return Err(format!(
                "python worker returned mismatched response id: expected {expected_id}, got {actual_id}"
            ));
        }
        Ok(response)
    }

    fn expect_ok(response: Value) -> Result<(), String> {
        let ok = response
            .get("ok")
            .and_then(Value::as_bool)
            .ok_or_else(|| "python worker response missing boolean 'ok'".to_string())?;
        if ok {
            return Ok(());
        }
        let error = response
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("unknown python worker error");
        let traceback = response
            .get("traceback")
            .and_then(Value::as_str)
            .unwrap_or("");
        if traceback.is_empty() {
            Err(format!("python worker error: {error}"))
        } else {
            Err(format!("python worker error: {error}\n{traceback}"))
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

impl Drop for PythonWorker {
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

fn default_init_args() -> Value {
    Value::Object(serde_json::Map::new())
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

fn evaluator_worker_script_path() -> Result<PathBuf, BuildError> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("python_api")
        .join("python_workers")
        .join("evaluator_worker.py");
    if !path.is_file() {
        return Err(BuildError::build(format!(
            "python evaluator worker script not found at '{}'",
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

#[cfg(test)]
mod tests {
    use super::normalize_flake_ref;

    #[test]
    fn normalize_flake_ref_prefixes_local_paths() {
        assert_eq!(
            normalize_flake_ref("./myflake#runtime"),
            "path:./myflake#runtime"
        );
        assert_eq!(
            normalize_flake_ref("../myflake#runtime"),
            "path:../myflake#runtime"
        );
        assert_eq!(
            normalize_flake_ref("/abs/path/to/flake#runtime"),
            "path:/abs/path/to/flake#runtime"
        );
    }

    #[test]
    fn normalize_flake_ref_keeps_remote_refs() {
        assert_eq!(
            normalize_flake_ref("github:owner/repo#runtime"),
            "github:owner/repo#runtime"
        );
        assert_eq!(
            normalize_flake_ref("path:./myflake#runtime"),
            "path:./myflake#runtime"
        );
    }
}
