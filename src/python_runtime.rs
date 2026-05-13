use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::core::{BuildError, PythonRuntimeConfig};
use crate::resources::{primary_resource_root, resolve_resource_path, resource_roots};

pub(crate) fn build_python_worker_command(
    runtime: &PythonRuntimeConfig,
    worker_script: &Path,
    label: &str,
) -> Result<Command, BuildError> {
    match runtime {
        PythonRuntimeConfig::Flake { reference } => {
            build_flake_worker_command(reference, worker_script, label)
        }
        PythonRuntimeConfig::Apptainer {
            image,
            python,
            workdir,
            nv,
        } => build_apptainer_worker_command(image, python, workdir.as_deref(), *nv, worker_script),
    }
}

fn build_flake_worker_command(
    reference: &str,
    worker_script: &Path,
    label: &str,
) -> Result<Command, BuildError> {
    let flake_ref = normalize_flake_ref(reference);
    let output_path = build_nix_output_path(&flake_ref)?;
    let python_executable = resolve_python_executable(&output_path).ok_or_else(|| {
        BuildError::build(format!(
            "python executable not found under '{}' (expected one of: bin/python, bin/python3)",
            output_path.display()
        ))
    })?;
    let mut command = Command::new(python_executable);
    command
        .arg("-u")
        .arg(worker_script)
        .current_dir(&output_path);
    if let Some(pythonpath) = build_worker_pythonpath(&output_path) {
        command.env("PYTHONPATH", pythonpath);
    }
    command.env("GAMMABOARD_PYTHON_RUNTIME", "flake");
    command.env("GAMMABOARD_PYTHON_WORKER", label);
    Ok(command)
}

fn build_apptainer_worker_command(
    image: &str,
    python: &str,
    workdir: Option<&str>,
    nv: bool,
    worker_script: &Path,
) -> Result<Command, BuildError> {
    let image_path = resolve_resource_path(Path::new(image))
        .map_err(|err| BuildError::build(format!("apptainer runtime image: {err}")))?;
    let image_path = absolute_path(&image_path)?;
    if !image_path.is_file() {
        return Err(BuildError::build(format!(
            "apptainer runtime image not found: '{}'",
            image_path.display()
        )));
    }

    let resources_root = absolute_path(&primary_resource_root().map_err(|err| {
        BuildError::build(format!("apptainer runtime requires a resource root: {err}"))
    })?)?;
    let workdir_path = match workdir {
        Some(path) => {
            let path = Path::new(path);
            if path.is_absolute() {
                absolute_path(path)?
            } else {
                resources_root.join(path)
            }
        }
        None => resources_root.clone(),
    };
    if !workdir_path.is_dir() {
        return Err(BuildError::build(format!(
            "apptainer runtime workdir is not a directory: '{}'",
            workdir_path.display()
        )));
    }
    let runtime_project_dir = image_path.parent().map(Path::to_path_buf);

    let worker_script = absolute_path(worker_script)?;
    let worker_script_dir = worker_script
        .parent()
        .ok_or_else(|| BuildError::build("python worker script has no parent directory"))?;

    let mut command = Command::new("apptainer");
    command.arg("exec");
    let mut roots = resource_roots();
    if roots.is_empty() {
        roots.push(resources_root.clone());
    }
    for root in roots {
        let root = absolute_path(&root)?;
        if root.is_dir() {
            command
                .arg("--bind")
                .arg(format!("{}:{}", root.display(), root.display()));
        }
    }
    command.arg("--bind").arg(format!(
        "{}:{}:ro",
        worker_script_dir.display(),
        worker_script_dir.display()
    ));
    command.arg("--pwd").arg(&workdir_path);
    if nv {
        command.arg("--nv");
    }
    if let Some(pythonpath) = build_apptainer_pythonpath(runtime_project_dir.as_deref()) {
        command
            .arg("--env")
            .arg(format!("PYTHONPATH={}", pythonpath.to_string_lossy()));
    }
    command
        .arg(&image_path)
        .arg(python)
        .arg("-u")
        .arg(&worker_script);
    command.env("GAMMABOARD_PYTHON_RUNTIME", "apptainer");
    Ok(command)
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

pub(crate) fn normalize_flake_ref(reference: &str) -> String {
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

fn build_apptainer_pythonpath(runtime_project_dir: Option<&Path>) -> Option<OsString> {
    let mut entries = Vec::new();
    if let Some(project_dir) = runtime_project_dir {
        entries.push(project_dir.to_path_buf());
        entries.push(project_dir.join("src"));
    }
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

fn absolute_path(path: &Path) -> Result<PathBuf, BuildError> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        env::current_dir()
            .map(|cwd| cwd.join(path))
            .map_err(|error| BuildError::build(format!("failed to resolve current dir: {error}")))
    }
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
