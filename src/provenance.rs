use serde::Serialize;
use serde_json::Value as JsonValue;

pub fn build_identity() -> String {
    format!(
        "{}:{}:{}",
        env!("CARGO_PKG_VERSION"),
        option_env!("GAMMABOARD_GIT_REVISION").unwrap_or("unknown"),
        enabled_features().join(",")
    )
}

pub fn build_info() -> JsonValue {
    serde_json::json!({"build_id":build_identity(),"version":env!("CARGO_PKG_VERSION"),
        "git_revision":option_env!("GAMMABOARD_GIT_REVISION"),"features":enabled_features(),
        "model_version":crate::compatibility::DATABASE_MODEL_VERSION})
}

#[derive(Debug, Clone, Serialize)]
pub struct RunProvenance {
    pub gammaboard_version: String,
    pub git_revision: Option<String>,
    pub enabled_features: Vec<String>,
    pub submitted_toml: Option<String>,
    pub effective_toml: String,
    pub external_versions: JsonValue,
}

impl RunProvenance {
    pub fn capture(submitted_toml: Option<String>, effective_toml: String) -> Self {
        Self {
            gammaboard_version: env!("CARGO_PKG_VERSION").to_string(),
            git_revision: option_env!("GAMMABOARD_GIT_REVISION").map(str::to_string),
            enabled_features: enabled_features(),
            submitted_toml,
            effective_toml,
            external_versions: external_versions(),
        }
    }
}

fn enabled_features() -> Vec<String> {
    #[cfg(feature = "gammaloop")]
    return vec!["gammaloop".to_string()];
    #[cfg(not(feature = "gammaloop"))]
    Vec::new()
}

fn external_versions() -> JsonValue {
    std::env::var("GAMMABOARD_EXTERNAL_VERSIONS")
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .filter(JsonValue::is_object)
        .unwrap_or_else(|| JsonValue::Object(Default::default()))
}
