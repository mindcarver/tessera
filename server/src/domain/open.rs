//! Open-original-location request and response types.

use serde::{Deserialize, Serialize};

use crate::domain::source::SourceId;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenRequest {
    record_id: String,
}

impl OpenRequest {
    pub fn new(record_id: String) -> Result<Self, OpenRequestError> {
        let record_id = record_id.trim().to_string();
        if record_id.is_empty() || record_id.len() > 512 {
            return Err(OpenRequestError::Invalid);
        }
        if !record_id.starts_with("rec_") {
            return Err(OpenRequestError::Invalid);
        }
        Ok(Self { record_id })
    }

    pub fn record_id(&self) -> &str {
        &self.record_id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenRequestError {
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenResult {
    record_id: String,
    source_id: SourceId,
}

impl OpenResult {
    pub fn new(record_id: String, source_id: SourceId) -> Self {
        Self {
            record_id,
            source_id,
        }
    }

    pub fn record_id(&self) -> &str {
        &self.record_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenTarget {
    pub record_id: String,
    pub source_id: SourceId,
    pub native_locator: String,
    pub normalized_root_path: String,
}
