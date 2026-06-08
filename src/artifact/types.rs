use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use crate::heartbeat::AgentId;
use crate::types::{AssignmentId, TaskId, Timestamp};

pub const ARTIFACT_PROTOCOL_V1: &str = "agent-artifact/v1";

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ArtifactId(String);

impl ArtifactId {
    pub fn new(value: impl Into<String>) -> Self {
        Self::try_new(value).expect("artifact id must not be empty")
    }

    pub fn try_new(value: impl Into<String>) -> Result<Self, ArtifactError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(ArtifactError::EmptyArtifactId);
        }

        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ArtifactId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for ArtifactId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for ArtifactId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MediaProfileId(String);

impl MediaProfileId {
    pub fn new(value: impl Into<String>) -> Self {
        Self::try_new(value).expect("media profile id must not be empty")
    }

    pub fn try_new(value: impl Into<String>) -> Result<Self, ArtifactError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(ArtifactError::EmptyMediaProfile);
        }

        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for MediaProfileId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for MediaProfileId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for MediaProfileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct HashDigest(String);

impl HashDigest {
    pub fn sha256(value: impl Into<String>) -> Result<Self, ArtifactError> {
        let value = value.into();
        let Some(hex) = value.strip_prefix("sha256:") else {
            return Err(ArtifactError::InvalidHash(value));
        };
        if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(ArtifactError::InvalidHash(value));
        }

        Ok(Self(format!("sha256:{}", hex.to_ascii_lowercase())))
    }

    pub fn from_sha256_hex(hex: impl Into<String>) -> Result<Self, ArtifactError> {
        Self::sha256(format!("sha256:{}", hex.into()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for HashDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactKind {
    Single,
    Bundle,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchemaRef {
    pub name: String,
    pub version: String,
    pub hash: HashDigest,
}

impl SchemaRef {
    pub fn new(name: impl Into<String>, version: impl Into<String>, hash: HashDigest) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            hash,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtifactProperty {
    Bool(bool),
    Integer(u64),
    Text(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactFile {
    pub path: Option<String>,
    pub uri: String,
    pub content_hash: HashDigest,
    pub media_type: String,
    pub media_profile: MediaProfileId,
    pub schema: Option<SchemaRef>,
    pub size_bytes: u64,
    pub properties: BTreeMap<String, ArtifactProperty>,
}

impl ArtifactFile {
    pub fn new(
        uri: impl Into<String>,
        content_hash: HashDigest,
        media_type: impl Into<String>,
        media_profile: impl Into<MediaProfileId>,
        size_bytes: u64,
    ) -> Self {
        Self {
            path: None,
            uri: uri.into(),
            content_hash,
            media_type: media_type.into(),
            media_profile: media_profile.into(),
            schema: None,
            size_bytes,
            properties: BTreeMap::new(),
        }
    }

    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    pub fn with_schema(mut self, schema: SchemaRef) -> Self {
        self.schema = Some(schema);
        self
    }

    pub fn with_property(mut self, name: impl Into<String>, value: ArtifactProperty) -> Self {
        self.properties.insert(name.into(), value);
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactManifest {
    pub protocol: String,
    pub artifact_id: ArtifactId,
    pub task_id: TaskId,
    pub assignment_id: AssignmentId,
    pub producer_agent_id: AgentId,
    pub kind: ArtifactKind,
    pub files: Vec<ArtifactFile>,
    pub created_at: Timestamp,
    pub manifest_hash: Option<HashDigest>,
    pub signature: Option<String>,
}

impl ArtifactManifest {
    pub fn new(
        artifact_id: impl Into<ArtifactId>,
        task_id: impl Into<TaskId>,
        assignment_id: impl Into<AssignmentId>,
        producer_agent_id: impl Into<AgentId>,
        kind: ArtifactKind,
        files: Vec<ArtifactFile>,
        created_at: Timestamp,
    ) -> Self {
        Self {
            protocol: ARTIFACT_PROTOCOL_V1.to_string(),
            artifact_id: artifact_id.into(),
            task_id: task_id.into(),
            assignment_id: assignment_id.into(),
            producer_agent_id: producer_agent_id.into(),
            kind,
            files,
            created_at,
            manifest_hash: None,
            signature: None,
        }
    }

    pub fn with_manifest_hash(mut self, manifest_hash: HashDigest) -> Self {
        self.manifest_hash = Some(manifest_hash);
        self
    }

    pub fn with_signature(mut self, signature: impl Into<String>) -> Self {
        self.signature = Some(signature.into());
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtifactError {
    EmptyArtifactId,
    EmptyMediaProfile,
    InvalidHash(String),
    UnsupportedProtocol(String),
    MissingManifestHash,
    ManifestHashMismatch {
        expected: HashDigest,
        actual: HashDigest,
    },
    EmptyFiles,
    SingleManifestMustHaveOneFile,
    BundleFileMissingPath {
        index: usize,
    },
    DuplicateBundlePath(String),
    EmptyUri {
        index: usize,
    },
    ZeroSize {
        index: usize,
    },
    UnsupportedMediaProfile {
        index: usize,
        profile: MediaProfileId,
    },
    MediaTypeMismatch {
        index: usize,
        profile: MediaProfileId,
        expected: String,
        actual: String,
    },
    MissingProperty {
        index: usize,
        profile: MediaProfileId,
        property: String,
    },
    PropertyTypeMismatch {
        index: usize,
        property: String,
        expected: PropertyType,
    },
    InvalidPropertyValue {
        index: usize,
        property: String,
        expected: String,
        actual: String,
    },
    MissingSchema {
        index: usize,
        media_profile: MediaProfileId,
    },
    InvalidSchema {
        index: usize,
    },
    AssignmentMismatch {
        expected: AssignmentId,
        actual: AssignmentId,
    },
    ProducerMismatch {
        expected: AgentId,
        actual: AgentId,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PropertyType {
    Bool,
    Integer,
    Text,
}

impl fmt::Display for ArtifactError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArtifactError::EmptyArtifactId => f.write_str("artifact id must not be empty"),
            ArtifactError::EmptyMediaProfile => f.write_str("media profile must not be empty"),
            ArtifactError::InvalidHash(value) => write!(f, "invalid hash digest: {value}"),
            ArtifactError::UnsupportedProtocol(protocol) => {
                write!(f, "unsupported artifact protocol: {protocol}")
            }
            ArtifactError::MissingManifestHash => f.write_str("manifest hash is required"),
            ArtifactError::ManifestHashMismatch { expected, actual } => write!(
                f,
                "manifest hash mismatch: expected={expected}, actual={actual}"
            ),
            ArtifactError::EmptyFiles => f.write_str("artifact manifest must include files"),
            ArtifactError::SingleManifestMustHaveOneFile => {
                f.write_str("single artifact manifest must include exactly one file")
            }
            ArtifactError::BundleFileMissingPath { index } => {
                write!(f, "bundle file is missing path: index={index}")
            }
            ArtifactError::DuplicateBundlePath(path) => {
                write!(f, "duplicate bundle file path: {path}")
            }
            ArtifactError::EmptyUri { index } => write!(f, "artifact file uri is empty: {index}"),
            ArtifactError::ZeroSize { index } => {
                write!(f, "artifact file size must be greater than zero: {index}")
            }
            ArtifactError::UnsupportedMediaProfile { index, profile } => {
                write!(f, "unsupported media profile at file {index}: {profile}")
            }
            ArtifactError::MediaTypeMismatch {
                index,
                profile,
                expected,
                actual,
            } => write!(
                f,
                "media type mismatch at file {index}: profile={profile}, expected={expected}, actual={actual}"
            ),
            ArtifactError::MissingProperty {
                index,
                profile,
                property,
            } => write!(
                f,
                "missing property at file {index}: profile={profile}, property={property}"
            ),
            ArtifactError::PropertyTypeMismatch {
                index,
                property,
                expected,
            } => write!(
                f,
                "property type mismatch at file {index}: property={property}, expected={expected:?}"
            ),
            ArtifactError::InvalidPropertyValue {
                index,
                property,
                expected,
                actual,
            } => write!(
                f,
                "invalid property value at file {index}: property={property}, expected={expected}, actual={actual}"
            ),
            ArtifactError::MissingSchema {
                index,
                media_profile,
            } => write!(
                f,
                "schema is required at file {index}: media_profile={media_profile}"
            ),
            ArtifactError::InvalidSchema { index } => {
                write!(f, "schema name and version must not be empty: {index}")
            }
            ArtifactError::AssignmentMismatch { expected, actual } => write!(
                f,
                "artifact assignment mismatch: expected={expected}, actual={actual}"
            ),
            ArtifactError::ProducerMismatch { expected, actual } => write!(
                f,
                "artifact producer mismatch: expected={expected}, actual={actual}"
            ),
        }
    }
}

impl Error for ArtifactError {}
