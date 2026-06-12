use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::types::Timestamp;

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct RelayId(String);

impl RelayId {
    pub fn new(value: impl Into<String>) -> Self {
        Self::try_new(value).expect("relay id must not be empty")
    }

    pub fn try_new(value: impl Into<String>) -> Result<Self, RelayError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(RelayError::EmptyRelayId);
        }

        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for RelayId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for RelayId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for RelayId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct RelayTokenHash(String);

impl RelayTokenHash {
    pub fn new(value: impl Into<String>) -> Self {
        Self::try_new(value).expect("relay token hash must not be empty")
    }

    pub fn try_new(value: impl Into<String>) -> Result<Self, RelayError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(RelayError::EmptyRelayTokenHash);
        }

        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for RelayTokenHash {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for RelayTokenHash {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for RelayTokenHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum RelayStatus {
    Created,
    Uploaded,
    Consumed,
    Expired,
    Deleted,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RelayMetadata {
    pub relay_id: RelayId,
    pub size_bytes: u64,
    pub max_downloads: u32,
    pub download_count: u32,
    pub status: RelayStatus,
    pub created_at: Timestamp,
    pub expires_at: Timestamp,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RelaySlot {
    pub metadata: RelayMetadata,
    pub upload_token_hash: RelayTokenHash,
    pub download_token_hash: RelayTokenHash,
    pub encrypted_blob: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CreatedRelaySlot {
    pub relay_id: RelayId,
    pub expires_at: Timestamp,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelayDownload {
    pub metadata: RelayMetadata,
    pub encrypted_blob: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RelayConfig {
    pub max_blob_size: u64,
    pub default_ttl_secs: u64,
    pub max_ttl_secs: u64,
    pub default_max_downloads: u32,
    pub max_downloads: u32,
}

impl Default for RelayConfig {
    fn default() -> Self {
        Self {
            max_blob_size: 50 * 1024 * 1024,
            default_ttl_secs: 60 * 60,
            max_ttl_secs: 24 * 60 * 60,
            default_max_downloads: 3,
            max_downloads: 10,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RelayError {
    EmptyRelayId,
    EmptyRelayTokenHash,
    InvalidConfig(&'static str),
    InvalidSize {
        size_bytes: u64,
        max: u64,
    },
    InvalidTtl {
        ttl_secs: u64,
        max: u64,
    },
    InvalidMaxDownloads {
        max_downloads: u32,
        max: u32,
    },
    RelayNotFound(RelayId),
    Unauthorized,
    Expired(RelayId),
    AlreadyUploaded(RelayId),
    NotUploaded(RelayId),
    DownloadLimitExceeded(RelayId),
    Deleted(RelayId),
    TimestampWentBackwards {
        relay_id: RelayId,
        current: Timestamp,
        attempted: Timestamp,
    },
    SizeMismatch {
        relay_id: RelayId,
        expected: u64,
        actual: u64,
    },
}

impl fmt::Display for RelayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RelayError::EmptyRelayId => f.write_str("relay id must not be empty"),
            RelayError::EmptyRelayTokenHash => f.write_str("relay token hash must not be empty"),
            RelayError::InvalidConfig(message) => write!(f, "invalid relay config: {message}"),
            RelayError::InvalidSize { size_bytes, max } => {
                write!(
                    f,
                    "relay blob size is invalid: size={size_bytes}, max={max}"
                )
            }
            RelayError::InvalidTtl { ttl_secs, max } => {
                write!(f, "relay ttl is invalid: ttl={ttl_secs}, max={max}")
            }
            RelayError::InvalidMaxDownloads { max_downloads, max } => write!(
                f,
                "relay max downloads is invalid: max_downloads={max_downloads}, max={max}"
            ),
            RelayError::RelayNotFound(relay_id) => write!(f, "relay not found: {relay_id}"),
            RelayError::Unauthorized => f.write_str("relay token is unauthorized"),
            RelayError::Expired(relay_id) => write!(f, "relay expired: {relay_id}"),
            RelayError::AlreadyUploaded(relay_id) => {
                write!(f, "relay already uploaded: {relay_id}")
            }
            RelayError::NotUploaded(relay_id) => write!(f, "relay is not uploaded: {relay_id}"),
            RelayError::DownloadLimitExceeded(relay_id) => {
                write!(f, "relay download limit exceeded: {relay_id}")
            }
            RelayError::Deleted(relay_id) => write!(f, "relay deleted: {relay_id}"),
            RelayError::TimestampWentBackwards {
                relay_id,
                current,
                attempted,
            } => write!(
                f,
                "relay timestamp went backwards: relay={relay_id}, current={}, attempted={}",
                current.0, attempted.0
            ),
            RelayError::SizeMismatch {
                relay_id,
                expected,
                actual,
            } => write!(
                f,
                "relay blob size mismatch: relay={relay_id}, expected={expected}, actual={actual}"
            ),
        }
    }
}

impl Error for RelayError {}
