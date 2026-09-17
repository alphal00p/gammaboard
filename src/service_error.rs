use crate::core::{EngineError, StoreError};

#[derive(Debug, Clone, thiserror::Error)]
pub enum ServiceError {
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<StoreError> for ServiceError {
    fn from(value: StoreError) -> Self {
        match value {
            StoreError::InvalidInput(message) => Self::BadRequest(message),
            StoreError::NotFound(message) => Self::NotFound(message),
            StoreError::Internal(message)
            | StoreError::Database(message)
            | StoreError::RetryActivation(message)
            | StoreError::Serialization(message) => Self::Internal(message),
            StoreError::BatchOwnershipLost {
                batch_id,
                node_uuid,
            } => Self::Internal(format!(
                "batch {batch_id} is no longer owned by node uuid '{node_uuid}'"
            )),
        }
    }
}

impl From<EngineError> for ServiceError {
    fn from(value: EngineError) -> Self {
        Self::BadRequest(value.to_string())
    }
}
