use thiserror::Error;

#[derive(Debug, Clone, Error)]
pub enum GammaboardEngineError {
    #[error("evaluation error: {0}")]
    Eval(String),
    #[error("build error: {0}")]
    Build(String),
    #[error("engine error: {0}")]
    Engine(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("incompatible configuration: {0}")]
    Incompatible(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("serialization error: {0}")]
    Serialization(String),
}

impl GammaboardEngineError {
    pub fn eval(message: impl Into<String>) -> Self {
        Self::Eval(message.into())
    }

    pub fn build(message: impl Into<String>) -> Self {
        Self::Build(message.into())
    }

    pub fn engine(message: impl Into<String>) -> Self {
        Self::Engine(message.into())
    }

    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::InvalidInput(message.into())
    }

    pub fn incompatible(message: impl Into<String>) -> Self {
        Self::Incompatible(message.into())
    }

    pub fn io(message: impl Into<String>) -> Self {
        Self::Io(message.into())
    }
}

pub type EvalError = GammaboardEngineError;
pub type BuildError = GammaboardEngineError;
pub type EngineError = GammaboardEngineError;

impl From<std::io::Error> for GammaboardEngineError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value.to_string())
    }
}

impl From<serde_json::Error> for GammaboardEngineError {
    fn from(value: serde_json::Error) -> Self {
        Self::Serialization(value.to_string())
    }
}
