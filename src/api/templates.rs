use crate::api::ApiError;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct TemplateFile {
    pub name: String,
    pub toml: String,
}

/// Lists `.toml` templates in a directory by file name.
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

/// Loads a named template file from a template directory.
pub fn load_template(dir: &Path, name: &str) -> Result<TemplateFile, ApiError> {
    let name = normalize_template_name(name)?;
    let path = resolve_template_path(dir, &name)?;
    let toml = fs::read_to_string(&path)
        .map_err(|err| ApiError::Internal(format!("failed reading {}: {err}", path.display())))?;
    Ok(TemplateFile {
        name,
        toml,
    })
}

/// Saves a named template file in a template directory.
pub fn save_template(dir: &Path, name: &str, toml: &str) -> Result<TemplateFile, ApiError> {
    let name = normalize_template_name(name)?;
    fs::create_dir_all(dir).map_err(|err| {
        ApiError::Internal(format!(
            "failed creating template directory {}: {err}",
            dir.display()
        ))
    })?;
    let path = dir.join(&name);
    fs::write(&path, toml)
        .map_err(|err| ApiError::Internal(format!("failed writing {}: {err}", path.display())))?;
    Ok(TemplateFile {
        name,
        toml: toml.to_string(),
    })
}

/// Deletes a named template file from a template directory.
pub fn delete_template(dir: &Path, name: &str) -> Result<(), ApiError> {
    let name = normalize_template_name(name)?;
    let path = resolve_template_path(dir, &name)?;
    fs::remove_file(&path)
        .map_err(|err| ApiError::Internal(format!("failed deleting {}: {err}", path.display())))?;
    Ok(())
}

fn normalize_template_name(name: &str) -> Result<String, ApiError> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.contains('/') || trimmed.contains('\\') {
        return Err(ApiError::BadRequest("invalid template name".to_string()));
    }
    let file_name = Path::new(trimmed)
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| ApiError::BadRequest("invalid template name".to_string()))?;
    if file_name != trimmed {
        return Err(ApiError::BadRequest("invalid template name".to_string()));
    }
    let normalized = if trimmed.ends_with(".toml") {
        trimmed.to_string()
    } else {
        format!("{trimmed}.toml")
    };
    Ok(normalized)
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
