use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static RESOURCE_ROOTS: OnceLock<Vec<PathBuf>> = OnceLock::new();

pub fn initialize_resource_roots(roots: &[String]) {
    let parsed: Vec<PathBuf> = roots
        .iter()
        .map(|root| root.trim())
        .filter(|root| !root.is_empty())
        .map(PathBuf::from)
        .collect();
    let _ = RESOURCE_ROOTS.set(parsed);
}

pub fn resolve_resource_path(path: &Path) -> Result<PathBuf, ResourceResolutionError> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let Some(roots) = RESOURCE_ROOTS.get() else {
        return Ok(path.to_path_buf());
    };
    if roots.is_empty() {
        return Ok(path.to_path_buf());
    }

    let attempted: Vec<PathBuf> = roots.iter().map(|root| root.join(path)).collect();
    if let Some(found) = attempted.iter().find(|candidate| candidate.exists()) {
        return Ok(found.to_path_buf());
    }

    Err(ResourceResolutionError {
        relative_path: path.to_path_buf(),
        attempted,
    })
}

#[derive(Debug)]
pub struct ResourceResolutionError {
    pub relative_path: PathBuf,
    pub attempted: Vec<PathBuf>,
}

impl std::fmt::Display for ResourceResolutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let attempted = self
            .attempted
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        write!(
            f,
            "failed to resolve resource path '{}'; attempted: [{}]",
            self.relative_path.display(),
            attempted
        )
    }
}

impl std::error::Error for ResourceResolutionError {}
