use serde::Serialize;
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ErrorKind {
    Usage,
    Operation,
    Environment,
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct AppError {
    pub kind: ErrorKind,
    pub code: String,
    pub message: String,
}

impl AppError {
    pub fn usage(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Usage, code, message)
    }

    pub fn operation(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Operation, code, message)
    }

    pub fn environment(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Environment, code, message)
    }

    pub fn exit_code(&self) -> i32 {
        match self.kind {
            ErrorKind::Operation => 1,
            ErrorKind::Usage | ErrorKind::Environment => 2,
        }
    }

    fn new(kind: ErrorKind, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind,
            code: code.into(),
            message: message.into(),
        }
    }
}
