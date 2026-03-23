use crate::api::ApiError;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct TemplateFile {
    pub name: String,
    pub toml: String,
}

pub fn list_templates(dir: &Path) -> Result<Vec<String>, ApiError> {
    let mut items = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(items);
    };
    for entry in entries {
        let entry = entry.map_err(|err| {
            ApiError::Internal(format!(
                "failed reading template dir {}: {err}",
                dir.display()
            ))
        })?;
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("toml") {
            continue;
        }
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        items.push(name.to_string());
    }
    items.sort();
    Ok(items)
}

pub fn load_template(dir: &Path, name: &str) -> Result<TemplateFile, ApiError> {
    let path = resolve_template_path(dir, name)?;
    let toml = fs::read_to_string(&path)
        .map_err(|err| ApiError::Internal(format!("failed reading {}: {err}", path.display())))?;
    Ok(TemplateFile {
        name: name.to_string(),
        toml,
    })
}

fn resolve_template_path(dir: &Path, name: &str) -> Result<PathBuf, ApiError> {
    if name.is_empty()
        || !name.ends_with(".toml")
        || name.contains('/')
        || name.contains('\\')
        || Path::new(name).file_name().and_then(|value| value.to_str()) != Some(name)
    {
        return Err(ApiError::BadRequest("invalid template name".to_string()));
    }
    let path = dir.join(name);
    if !path.is_file() {
        return Err(ApiError::NotFound(format!("template {name} not found")));
    }
    Ok(path)
}
