use std::{error::Error, fmt};

#[derive(Debug, Clone)]
pub struct EvalError {
    pub message: String,
}

impl EvalError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "evaluation error: {}", self.message)
    }
}

impl Error for EvalError {}

#[derive(Debug, Clone)]
pub struct BuildError {
    pub message: String,
}

impl BuildError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "build error: {}", self.message)
    }
}

impl Error for BuildError {}

#[derive(Debug, Clone)]
pub struct EngineError {
    pub message: String,
}

impl EngineError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "engine error: {}", self.message)
    }
}

impl Error for EngineError {}

#[derive(Debug, Clone)]
pub struct StoreError {
    pub message: String,
}

impl StoreError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "store error: {}", self.message)
    }
}

impl Error for StoreError {}
