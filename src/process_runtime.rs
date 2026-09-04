use std::path::{Path, PathBuf};
use std::process::Command;

use crate::core::{BuildError, EngineError};
use crate::resources::primary_resource_root;
use serde_json::Value;

pub(crate) fn default_process_args() -> Value {
    Value::Object(serde_json::Map::new())
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
) -> Result<Command, BuildError> {
    if runtime_command.is_empty() {
        return Err(BuildError::build(
            "process runtime command must not be empty",
        ));
    }
    let resources_root = absolute_path(&primary_resource_root().map_err(|err| {
        BuildError::build(format!("process runtime requires a resource root: {err}"))
    })?)?;
    let workdir = expand_runtime_path(runtime_cwd.unwrap_or("$resources"), &resources_root)?;
    if !workdir.is_dir() {
        return Err(BuildError::build(format!(
            "process runtime workdir is not a directory: '{}'",
            workdir.display()
        )));
    }

    let program = expand_runtime_value(&runtime_command[0], &resources_root);
    if program.trim().is_empty() {
        return Err(BuildError::build(
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
        args = ?resolved_args,
        workdir = %workdir.display(),
        "resolved process runtime command"
    );

    let mut command = Command::new(program);
    command.current_dir(&workdir);
    command.args(&resolved_args);
    command.env("GAMMABOARD_PROCESS_RUNTIME", "command");
    command.env("GAMMABOARD_PROCESS_WORKER", label);
    Ok(command)
}

fn expand_runtime_path(raw: &str, resources_root: &Path) -> Result<PathBuf, BuildError> {
    let expanded = expand_runtime_value(raw, resources_root);
    let path = PathBuf::from(expanded);
    absolute_path(&path)
}

fn expand_runtime_value(raw: &str, resources_root: &Path) -> String {
    raw.replace("$resources", &resources_root.display().to_string())
}

fn absolute_path(path: &Path) -> Result<PathBuf, BuildError> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .map_err(|error| BuildError::build(format!("failed to resolve current dir: {error}")))
    }
}

#[cfg(test)]
mod tests {
    use super::expand_runtime_value;
    use std::path::Path;

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
}
