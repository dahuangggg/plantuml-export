use serde::Serialize;

use crate::AppError;

pub const JSON_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JsonEnvelope<T: Serialize> {
    pub schema_version: u32,
    pub ok: bool,
    pub command: String,
    pub data: Option<T>,
    pub error: Option<JsonError>,
}

#[derive(Debug, Serialize)]
pub struct JsonError {
    pub kind: crate::ErrorKind,
    pub code: String,
    pub message: String,
}

impl<T: Serialize> JsonEnvelope<T> {
    pub fn success(command: impl Into<String>, data: T) -> Self {
        Self {
            schema_version: JSON_SCHEMA_VERSION,
            ok: true,
            command: command.into(),
            data: Some(data),
            error: None,
        }
    }

    pub fn failure_with_data(command: impl Into<String>, error: &AppError, data: T) -> Self {
        Self {
            schema_version: JSON_SCHEMA_VERSION,
            ok: false,
            command: command.into(),
            data: Some(data),
            error: Some(JsonError::from(error)),
        }
    }
}

impl JsonEnvelope<serde_json::Value> {
    pub fn failure(command: impl Into<String>, error: &AppError) -> Self {
        Self {
            schema_version: JSON_SCHEMA_VERSION,
            ok: false,
            command: command.into(),
            data: None,
            error: Some(JsonError::from(error)),
        }
    }
}

impl From<&AppError> for JsonError {
    fn from(error: &AppError) -> Self {
        Self {
            kind: error.kind,
            code: error.code.clone(),
            message: error.message.clone(),
        }
    }
}
