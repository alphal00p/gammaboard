#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::core::EngineError;
use crate::evaluation::{Batch, Point};
use crate::resources::primary_resource_root;
use crate::utils::domain::Domain;
use serde_json::Value;

/// Shared launch configuration for all process-backed roles.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProcessConfig {
    pub command: Vec<String>,
    pub cwd: Option<String>,
    #[serde(default = "default_process_args")]
    pub args: Value,
    #[serde(default = "crate::process_worker::default_process_shutdown_grace_seconds")]
    pub shutdown_grace_seconds: u64,
}

impl ProcessConfig {
    pub fn validate(&self) -> Result<(), EngineError> {
        if self.command.is_empty() {
            return Err(EngineError::build("process command must not be empty"));
        }
        if !self.args.is_object() {
            return Err(EngineError::build(
                "process args must be a TOML table / JSON object",
            ));
        }
        Ok(())
    }
}

pub(crate) fn default_process_args() -> Value {
    Value::Object(serde_json::Map::new())
}

pub(crate) fn decode_process_batch(
    response: &Value,
    nr_samples: usize,
    domain: &Domain,
    adapter: &str,
) -> Result<Batch, EngineError> {
    let xs_discrete = response
        .get("xs_discrete_row_major")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            EngineError::engine(format!(
                "process {adapter} response missing 'xs_discrete_row_major' array"
            ))
        })?;
    let xs_discrete_offsets = parse_process_offsets(
        response,
        "xs_discrete_offsets",
        nr_samples,
        domain.fixed_discrete_depth().unwrap_or(0),
        xs_discrete.len(),
        adapter,
    )?;
    let xs_continuous = response
        .get("xs_continuous_row_major")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            EngineError::engine(format!(
                "process {adapter} response missing 'xs_continuous_row_major' array"
            ))
        })?;
    let xs_continuous_offsets = parse_process_offsets(
        response,
        "xs_continuous_offsets",
        nr_samples,
        domain.fixed_continuous_dims().unwrap_or(0),
        xs_continuous.len(),
        adapter,
    )?;
    let xs_discrete_row_major = xs_discrete
        .iter()
        .enumerate()
        .map(|(index, value)| {
            value.as_i64().ok_or_else(|| {
                EngineError::engine(format!(
                    "process {adapter} returned non-i64 value at xs_discrete_row_major[{index}]"
                ))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let xs_continuous_row_major = xs_continuous
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let value = value.as_f64().ok_or_else(|| {
                EngineError::engine(format!(
                    "process {adapter} returned non-f64 value at xs_continuous_row_major[{index}]"
                ))
            })?;
            if !value.is_finite() {
                return Err(EngineError::engine(format!(
                    "process {adapter} returned non-finite value at xs_continuous_row_major[{index}]"
                )));
            }
            Ok(value)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let weights = response
        .get("weights")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            EngineError::engine(format!(
                "process {adapter} response missing 'weights' array"
            ))
        })?;
    if weights.len() != nr_samples {
        return Err(EngineError::engine(format!(
            "process {adapter} weights size mismatch: expected {} values, got {}",
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
                    "process {adapter} returned non-f64 value at weights[{index}]"
                ))
            })?;
            if !weight.is_finite() || weight <= 0.0 {
                return Err(EngineError::engine(format!(
                    "process {adapter} returned non-positive or non-finite value at weights[{index}]"
                )));
            }
            Ok(weight)
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut points = Vec::with_capacity(nr_samples);
    for sample_idx in 0..nr_samples {
        let discrete_start = xs_discrete_offsets[sample_idx];
        let discrete_end = xs_discrete_offsets[sample_idx + 1];
        let continuous_start = xs_continuous_offsets[sample_idx];
        let continuous_end = xs_continuous_offsets[sample_idx + 1];
        points.push(Point::new(
            xs_continuous_row_major[continuous_start..continuous_end].to_vec(),
            xs_discrete_row_major[discrete_start..discrete_end].to_vec(),
            weights[sample_idx],
        ));
    }
    Ok(Batch::new(points))
}

pub(crate) fn expect_process_ack(response: Value, adapter: &str) -> Result<(), String> {
    if response.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        return Ok(());
    }
    Err(format!(
        "process {adapter} initialize result missing ok=true"
    ))
}

pub(crate) fn parse_process_offsets(
    response: &Value,
    field: &str,
    nr_samples: usize,
    fixed_width: usize,
    values_len: usize,
    adapter: &str,
) -> Result<Vec<usize>, EngineError> {
    let Some(raw) = response.get(field) else {
        let expected_len = nr_samples.saturating_mul(fixed_width);
        if values_len != expected_len {
            return Err(EngineError::engine(format!(
                "process {adapter} response missing '{field}' for ragged output with {values_len} values; fixed-width fallback expected {expected_len}"
            )));
        }
        return Ok((0..=nr_samples).map(|index| index * fixed_width).collect());
    };
    let offsets = raw.as_array().ok_or_else(|| {
        EngineError::engine(format!(
            "process {adapter} response field '{field}' must be an array"
        ))
    })?;
    if offsets.len() != nr_samples + 1 {
        return Err(EngineError::engine(format!(
            "process {adapter} response field '{field}' must contain nr_samples + 1 offsets"
        )));
    }
    let mut parsed = Vec::with_capacity(offsets.len());
    let mut previous = 0;
    for (index, value) in offsets.iter().enumerate() {
        let offset = value.as_u64().ok_or_else(|| {
            EngineError::engine(format!(
                "process {adapter} response field '{field}[{index}]' must be a non-negative integer"
            ))
        })? as usize;
        if index == 0 && offset != 0 {
            return Err(EngineError::engine(format!(
                "process {adapter} response field '{field}' must start at 0"
            )));
        }
        if offset < previous {
            return Err(EngineError::engine(format!(
                "process {adapter} response field '{field}' must be non-decreasing"
            )));
        }
        if offset > values_len {
            return Err(EngineError::engine(format!(
                "process {adapter} response field '{field}[{index}]' exceeds row-major value length {values_len}"
            )));
        }
        previous = offset;
        parsed.push(offset);
    }
    if parsed.last().copied() != Some(values_len) {
        return Err(EngineError::engine(format!(
            "process {adapter} response field '{field}' must end at row-major value length {values_len}"
        )));
    }
    Ok(parsed)
}

pub(crate) fn build_process_worker_command(
    runtime_command: &[String],
    runtime_cwd: Option<&str>,
    label: &str,
) -> Result<Command, EngineError> {
    if runtime_command.is_empty() {
        return Err(EngineError::build(
            "process runtime command must not be empty",
        ));
    }
    let resources_root = absolute_path(&primary_resource_root().map_err(|err| {
        EngineError::build(format!("process runtime requires a resource root: {err}"))
    })?)?;
    let workdir = expand_runtime_path(runtime_cwd.unwrap_or("$resources"), &resources_root)?;
    if !workdir.is_dir() {
        return Err(EngineError::build(format!(
            "process runtime workdir is not a directory: '{}'",
            workdir.display()
        )));
    }

    let program = expand_runtime_value(&runtime_command[0], &resources_root);
    if program.trim().is_empty() {
        return Err(EngineError::build(
            "process runtime program must not be empty",
        ));
    }
    let resolved_args = runtime_command[1..]
        .iter()
        .map(|arg| expand_runtime_value(arg, &resources_root))
        .collect::<Vec<_>>();

    tracing::info!(
        worker = label,
        program = %program,
        workdir = %workdir.display(),
        "resolved process runtime command"
    );

    let mut command = Command::new(program);
    command.current_dir(&workdir);
    command.args(&resolved_args);
    // Process workers must not receive terminal signals intended for GammaBoard.
    // GammaBoard owns their lifecycle and shuts down the isolated process group.
    isolate_process_command(&mut command);
    command.env("GAMMABOARD_PROCESS_RUNTIME", "command");
    command.env("GAMMABOARD_PROCESS_WORKER", label);
    Ok(command)
}

fn isolate_process_command(command: &mut Command) {
    #[cfg(unix)]
    command.process_group(0);
}

fn expand_runtime_path(raw: &str, resources_root: &Path) -> Result<PathBuf, EngineError> {
    let expanded = expand_runtime_value(raw, resources_root);
    let path = PathBuf::from(expanded);
    absolute_path(&path)
}

fn expand_runtime_value(raw: &str, resources_root: &Path) -> String {
    raw.replace("$resources", &resources_root.display().to_string())
}

fn absolute_path(path: &Path) -> Result<PathBuf, EngineError> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .map_err(|error| EngineError::build(format!("failed to resolve current dir: {error}")))
    }
}

#[cfg(test)]
mod tests {
    use super::{expand_runtime_value, isolate_process_command};
    use std::path::Path;
    #[cfg(unix)]
    use std::process::Command;

    #[test]
    fn process_config_deserializes_command_and_args() {
        let params: super::ProcessConfig = toml::from_str(
            "command = ['python', '-u', 'worker.py']\ncwd = '$resources'\nargs = { scale = 2.0 }",
        )
        .unwrap();
        assert_eq!(params.command, ["python", "-u", "worker.py"]);
        assert_eq!(params.cwd.as_deref(), Some("$resources"));
        assert_eq!(params.args, serde_json::json!({"scale": 2.0}));
        params.validate().unwrap();
    }

    #[test]
    fn expands_resources_placeholder_only() {
        let resources = Path::new("/tmp/gammaboard/resources");

        assert_eq!(
            expand_runtime_value("$resources/runtime.sif:$resources", resources),
            "/tmp/gammaboard/resources/runtime.sif:/tmp/gammaboard/resources"
        );
        assert_eq!(
            expand_runtime_value("relative/path", resources),
            "relative/path"
        );
        assert_eq!(
            expand_runtime_value("/opt/worker.py", resources),
            "/opt/worker.py"
        );
    }

    #[cfg(unix)]
    #[test]
    fn process_commands_start_in_an_isolated_process_group() {
        let mut command = Command::new("sh");
        command.arg("-c").arg("sleep 10");
        isolate_process_command(&mut command);
        let mut child = command.spawn().expect("spawn isolated worker");
        let pid = child.id() as libc::pid_t;

        assert_eq!(unsafe { libc::getpgid(pid) }, pid);

        let _ = unsafe { libc::kill(-pid, libc::SIGKILL) };
        let _ = child.wait();
    }
}
