use thiserror::Error;

#[derive(Debug, Clone, Error)]
pub enum StoreError {
    #[error("store internal error: {0}")]
    Internal(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("database error: {0}")]
    Database(String),
    #[error("runtime activation must be retried: {0}")]
    RetryActivation(String),
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("batch {batch_id} is no longer owned by node uuid '{node_uuid}'")]
    BatchOwnershipLost { batch_id: i64, node_uuid: String },
}

impl StoreError {
    pub fn store(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }

    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::InvalidInput(message.into())
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }

    pub fn batch_ownership_lost(batch_id: i64, node_uuid: impl Into<String>) -> Self {
        Self::BatchOwnershipLost {
            batch_id,
            node_uuid: node_uuid.into(),
        }
    }

    pub fn retry_activation(message: impl Into<String>) -> Self {
        Self::RetryActivation(message.into())
    }

    pub fn is_retry_activation(&self) -> bool {
        matches!(self, Self::RetryActivation(_))
    }

    pub fn is_batch_ownership_lost(&self) -> bool {
        matches!(self, Self::BatchOwnershipLost { .. })
    }

    pub fn is_database_error(&self) -> bool {
        matches!(self, Self::Database(_))
    }
}

impl From<sqlx::Error> for StoreError {
    fn from(value: sqlx::Error) -> Self {
        Self::Database(value.to_string())
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(value: serde_json::Error) -> Self {
        Self::Serialization(value.to_string())
    }
}

#[derive(Debug, Clone, Error)]
pub enum GammaBoardEngineError {
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

impl GammaBoardEngineError {
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

pub type EvalError = GammaBoardEngineError;
pub type BuildError = GammaBoardEngineError;
pub type EngineError = GammaBoardEngineError;

impl From<std::io::Error> for GammaBoardEngineError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value.to_string())
    }
}

impl From<serde_json::Error> for GammaBoardEngineError {
    fn from(value: serde_json::Error) -> Self {
        Self::Serialization(value.to_string())
    }
}

/// Map any `Display` error into a `GammaBoardEngineError` variant by its
/// stringified message, replacing `.map_err(|err| X::ctor(err.to_string()))`.
pub trait EngineResultExt<T> {
    fn engine_err(self) -> Result<T, GammaBoardEngineError>;
    fn build_err(self) -> Result<T, GammaBoardEngineError>;
    fn eval_err(self) -> Result<T, GammaBoardEngineError>;
}

impl<T, E: std::fmt::Display> EngineResultExt<T> for Result<T, E> {
    fn engine_err(self) -> Result<T, GammaBoardEngineError> {
        self.map_err(|err| GammaBoardEngineError::engine(err.to_string()))
    }
    fn build_err(self) -> Result<T, GammaBoardEngineError> {
        self.map_err(|err| GammaBoardEngineError::build(err.to_string()))
    }
    fn eval_err(self) -> Result<T, GammaBoardEngineError> {
        self.map_err(|err| GammaBoardEngineError::eval(err.to_string()))
    }
}

/// Map any `Display` error into `StoreError::Internal`, replacing
/// `.map_err(|err| StoreError::store(err.to_string()))`.
pub trait StoreResultExt<T> {
    fn store_err(self) -> Result<T, StoreError>;
}

impl<T, E: std::fmt::Display> StoreResultExt<T> for Result<T, E> {
    fn store_err(self) -> Result<T, StoreError> {
        self.map_err(|err| StoreError::store(err.to_string()))
    }
}
