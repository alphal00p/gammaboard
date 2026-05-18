use std::path::{Path, PathBuf};
use std::process::Command;

use crate::core::BuildError;
use crate::resources::primary_resource_root;

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
