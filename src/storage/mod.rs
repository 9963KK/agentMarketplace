mod core;
mod service;
mod types;

pub use core::StorageCore;
pub use service::{StorageHandle, StorageService, StorageServiceError};
pub use types::{
    ArtifactLocator, AuthCredential, IdempotencyDecision, IdempotencyKey, IdempotencyOutcome,
    IdempotencyRecord, IdempotentOperation, StorageError, StoreOutcome, TokenHash,
};
