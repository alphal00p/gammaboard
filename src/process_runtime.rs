use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::core::BuildError;
use crate::resources::{primary_resource_root, resolve_resource_path, resource_roots};

pub(crate) fn build_process_worker_command(
    runtime_command: &[String],
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
    let workdir = resources_root.clone();
    if !workdir.is_dir() {
        return Err(BuildError::build(format!(
            "process runtime workdir is not a directory: '{}'",
            workdir.display()
        )));
    }

    let program = resolve_program(&runtime_command[0])?;
    let mut resolved_args = runtime_command[1..]
        .iter()
        .map(|arg| resolve_command_arg(arg))
        .collect::<Result<Vec<_>, _>>()?;
    let injected_apptainer_binds =
        maybe_inject_apptainer_binds(runtime_command, &mut resolved_args, &resources_root)?;

    tracing::info!(
        worker = label,
        program = %program.display(),
        args = ?resolved_args,
        workdir = %workdir.display(),
        injected_apptainer_binds = ?injected_apptainer_binds,
        "resolved process runtime command"
    );

    let mut command = Command::new(program);
    command.current_dir(&workdir);
    command.args(&resolved_args);
    command.env("GAMMABOARD_PROCESS_RUNTIME", "command");
    command.env("GAMMABOARD_PROCESS_WORKER", label);
    Ok(command)
}

fn maybe_inject_apptainer_binds(
    raw_command: &[String],
    resolved_args: &mut Vec<String>,
    resources_root: &Path,
) -> Result<Vec<String>, BuildError> {
    let Some(program) = raw_command.first() else {
        return Ok(Vec::new());
    };
    let program_name = Path::new(program)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(program);
    if program_name != "apptainer" {
        return Ok(Vec::new());
    }

    let insert_pos = raw_command
        .iter()
        .position(|arg| arg == "exec")
        .map(|index| index)
        .unwrap_or(0);
    let mut injected = Vec::new();
    let mut roots = resource_roots();
    if roots.is_empty() {
        roots.push(resources_root.to_path_buf());
    }
    for root in roots {
        let root = absolute_path(&root)?;
        if root.is_dir() {
            injected.push("--bind".to_string());
            injected.push(format!("{}:{}", root.display(), root.display()));
        }
    }
    let arg_insert_pos = insert_pos.min(resolved_args.len());
    resolved_args.splice(arg_insert_pos..arg_insert_pos, injected.clone());
    Ok(injected)
}

fn resolve_program(raw: &str) -> Result<PathBuf, BuildError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(BuildError::build(
            "process runtime program must not be empty",
        ));
    }
    let path = Path::new(trimmed);
    if path.components().count() > 1 || path.is_absolute() {
        return resolve_runtime_path(trimmed);
    }
    Ok(PathBuf::from(trimmed))
}

fn resolve_command_arg(raw: &str) -> Result<String, BuildError> {
    if Path::new(raw).is_absolute() {
        return Ok(raw.to_string());
    }
    if looks_like_resource_path(raw) {
        return Ok(resolve_runtime_path(raw)?.display().to_string());
    }
    Ok(raw.to_string())
}

fn looks_like_resource_path(raw: &str) -> bool {
    if raw.starts_with('-') || raw.contains("://") || raw.starts_with("path:") {
        return false;
    }
    raw.contains('/') || raw.starts_with("./") || raw.starts_with("../")
}

fn resolve_runtime_path(raw: &str) -> Result<PathBuf, BuildError> {
    let path = Path::new(raw);
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        resolve_resource_path(path)
            .map_err(|err| BuildError::build(format!("process runtime path '{raw}': {err}")))?
    };
    let resolved = absolute_path(&resolved)?;
    if !resolved.exists() {
        return Err(BuildError::build(format!(
            "process runtime path does not exist: '{}'",
            resolved.display()
        )));
    }
    Ok(resolved)
}

fn absolute_path(path: &Path) -> Result<PathBuf, BuildError> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        env::current_dir()
            .map(|cwd| cwd.join(path))
            .map_err(|error| BuildError::build(format!("failed to resolve current dir: {error}")))
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_command_arg;

    #[test]
    fn absolute_command_args_are_not_resource_paths() {
        let resolved = resolve_command_arg("/opt/madnis_gammaboard_api/sampler_worker.py")
            .expect("absolute container path should pass through");

        assert_eq!(resolved, "/opt/madnis_gammaboard_api/sampler_worker.py");
    }
}
