use thiserror::Error;

#[derive(Debug, Clone, Error)]
#[error("store error: {0}")]
pub struct StoreError(String);

impl StoreError {
    pub fn store(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}
