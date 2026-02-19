use thiserror::Error;

#[derive(Debug, Clone, Error)]
pub enum GammaboardEngineError {
    #[error("evaluation error: {0}")]
    Eval(String),
    #[error("build error: {0}")]
    Build(String),
    #[error("engine error: {0}")]
    Engine(String),
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
}

pub type EvalError = GammaboardEngineError;
pub type BuildError = GammaboardEngineError;
pub type EngineError = GammaboardEngineError;
