use thiserror::Error;

#[derive(Debug, Clone, Error)]
pub enum GammaboardError {
    #[error("evaluation error: {0}")]
    Eval(String),
    #[error("build error: {0}")]
    Build(String),
    #[error("engine error: {0}")]
    Engine(String),
    #[error("store error: {0}")]
    Store(String),
}

impl GammaboardError {
    pub fn eval(message: impl Into<String>) -> Self {
        Self::Eval(message.into())
    }

    pub fn build(message: impl Into<String>) -> Self {
        Self::Build(message.into())
    }

    pub fn engine(message: impl Into<String>) -> Self {
        Self::Engine(message.into())
    }

    pub fn store(message: impl Into<String>) -> Self {
        Self::Store(message.into())
    }
}

pub type EvalError = GammaboardError;
pub type BuildError = GammaboardError;
pub type EngineError = GammaboardError;
pub type StoreError = GammaboardError;
