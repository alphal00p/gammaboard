use crate::core::BuildError;
use crate::preprocess::RunAddConfig;
use crate::process_runtime::build_process_worker_command;
use crate::resources::resolve_resource_path;
use serde_json::Value;
use std::path::Path;

pub(crate) fn validate_resources(config: &RunAddConfig) -> Result<(), BuildError> {
    if let Some(evaluator) = &config.integration_params.evaluator {
        validate_value(&serde_json::to_value(evaluator).unwrap(), "evaluator")?;
    }
    for (index, task) in config.task_queue.iter().flatten().enumerate() {
        validate_value(
            &serde_json::to_value(task).unwrap(),
            &format!("task_queue[{index}]"),
        )?;
    }
    Ok(())
}

fn validate_value(value: &Value, field: &str) -> Result<(), BuildError> {
    if let Some(kind) = value.get("kind").and_then(Value::as_str) {
        if matches!(
            kind,
            "process_sampler"
                | "process_evaluator"
                | "process_materializer"
                | "process_batch_transform"
        ) {
            let argv: Vec<String> =
                serde_json::from_value(value.get("command").cloned().unwrap_or(Value::Null))
                    .map_err(|err| BuildError::build(format!("{field}.command: {err}")))?;
            let command =
                build_process_worker_command(&argv, value.get("cwd").and_then(Value::as_str), kind)
                    .map_err(|err| BuildError::build(format!("{field}: {err}")))?;
            validate_executable(
                command.get_program(),
                command.get_current_dir().unwrap_or(Path::new(".")),
            )
            .map_err(|err| BuildError::build(format!("{field}.command[0]: {err}")))?;
        }
        if kind == "gammaloop"
            && let Some(folder) = value.get("state_folder").and_then(Value::as_str)
        {
            let folder = resolve_resource_path(Path::new(folder))
                .map_err(|err| BuildError::build(format!("{field}.state_folder: {err}")))?;
            for file in ["state_manifest.toml", "model.json", "symbolica_state.bin"] {
                require_file(&folder.join(file), field)?;
            }
            // State loading visits all saved integrands, including partial output
            // left behind by an interrupted generation of a different integrand.
            let mut selected_found = value
                .get("integrand_name")
                .and_then(Value::as_str)
                .is_none();
            for family in ["cross_sections", "amplitudes"] {
                let processes = folder.join("processes").join(family);
                if !processes.is_dir() {
                    continue;
                }
                for process in std::fs::read_dir(&processes)
                    .map_err(|err| BuildError::build(format!("{}: {err}", processes.display())))?
                {
                    let process = process
                        .map_err(|err| BuildError::build(err.to_string()))?
                        .path();
                    if !process.is_dir() {
                        continue;
                    }
                    for integrand in std::fs::read_dir(&process)
                        .map_err(|err| BuildError::build(err.to_string()))?
                    {
                        let integrand = integrand
                            .map_err(|err| BuildError::build(err.to_string()))?
                            .path();
                        if !integrand.is_dir() {
                            continue;
                        }
                        let selected = value
                            .get("integrand_name")
                            .and_then(Value::as_str)
                            .is_some_and(|name| {
                                integrand.file_name().is_some_and(|file| file == name)
                            });
                        if integrand.join("integrand").exists() || selected {
                            require_file(&integrand.join("integrand/integrand.bin"), field)?;
                            require_file(
                                &integrand.join(if family == "cross_sections" {
                                    "cs.bin"
                                } else {
                                    "amp.bin"
                                }),
                                field,
                            )?;
                            selected_found |= selected;
                        }
                    }
                }
            }
            if !selected_found {
                return Err(BuildError::build(format!(
                    "{field}: integrand {} has no generated state under {}",
                    value["integrand_name"],
                    folder.display()
                )));
            }
        }
    }
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if !matches!(key.as_str(), "args" | "constants" | "replacements") {
                    validate_value(child, &format!("{field}.{key}"))?;
                }
            }
        }
        Value::Array(values) => {
            for (i, child) in values.iter().enumerate() {
                validate_value(child, &format!("{field}[{i}]"))?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn require_file(path: &Path, field: &str) -> Result<(), BuildError> {
    if !path.is_file() {
        return Err(BuildError::build(format!(
            "{field}: required state file missing: {}. Complete state generation before starting this run.",
            path.display()
        )));
    }
    Ok(())
}

fn validate_executable(program: &std::ffi::OsStr, cwd: &Path) -> Result<(), String> {
    let path = Path::new(program);
    let candidates = if path.components().count() > 1 || path.is_absolute() {
        vec![cwd.join(path)]
    } else {
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|entry| cwd.join(entry).join(path))
            .collect()
    };
    if candidates.iter().any(|path| {
        let Ok(meta) = path.metadata() else {
            return false;
        };
        if !meta.is_file() {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            meta.permissions().mode() & 0o111 != 0
        }
        #[cfg(not(unix))]
        {
            true
        }
    }) {
        return Ok(());
    }
    Err(format!(
        "executable {:?} was not found or is not executable (working directory {})",
        program,
        cwd.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_executable_reports_program_and_workdir() {
        let err = validate_executable(std::ffi::OsStr::new("./missing-worker"), Path::new("/tmp"))
            .unwrap_err();
        assert!(err.contains("missing-worker") && err.contains("/tmp"));
    }
    #[test]
    fn missing_state_reports_the_exact_artifact() {
        let temp = tempfile::tempdir().unwrap();
        let err = require_file(
            &temp.path().join("GL26/integrand/integrand.bin"),
            "evaluator",
        )
        .unwrap_err();
        assert!(err.to_string().contains("GL26/integrand/integrand.bin"));
    }
}
