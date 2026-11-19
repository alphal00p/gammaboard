use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymbolicaParams {
    pub args: Vec<String>,
    pub source: SymbolicaSource,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SymbolicaSource {
    Expressions {
        components: Vec<String>,
        exprs: Vec<String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        constants: BTreeMap<String, toml::Value>,
    },
    Expression {
        expr: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        constants: BTreeMap<String, toml::Value>,
    },
    Compiled {
        path: PathBuf,
        name: String,
        #[serde(default = "default_symbolica_outputs")]
        outputs: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        compiled_args: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        bindings: BTreeMap<String, toml::Value>,
    },
}

fn default_symbolica_outputs() -> usize {
    1
}
