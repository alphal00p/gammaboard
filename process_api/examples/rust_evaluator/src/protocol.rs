use serde::Deserialize;
use serde_json::Value;

#[path = "../../common/wire.rs"]
mod wire;
pub use wire::{read_request, send_error, send_result};

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EvaluatorOutput {
    Values { components: Vec<String> },
}

#[derive(Debug, Deserialize)]
pub struct InitializeParams {
    pub protocol: String,
    pub role: String,
    #[serde(default)]
    pub args: Value,
    pub output: EvaluatorOutput,
    #[serde(default)]
    pub domain: Value,
}

#[derive(Debug, Deserialize)]
pub struct EvalBatchParams {
    pub nr_samples: usize,
    pub xs_discrete_offsets: Option<Vec<usize>>,
    pub xs_continuous_offsets: Option<Vec<usize>>,
}
