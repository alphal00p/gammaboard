use serde::Serialize;
use serde_json::Value as JsonValue;

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
