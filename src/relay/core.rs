use std::collections::HashMap;

use crate::types::Timestamp;

use super::types::{
    CreatedRelaySlot, RelayConfig, RelayDownload, RelayError, RelayId, RelayMetadata, RelaySlot,
    RelayStatus, RelayTokenHash,
};

#[derive(Debug)]
pub struct RelayCore {
    slots: HashMap<RelayId, RelaySlot>,
    config: RelayConfig,
    next_relay: u64,
}

impl Default for RelayCore {
    fn default() -> Self {
        Self::new(RelayConfig::default()).expect("default relay config should be valid")
    }
}

impl RelayCore {
    pub fn new(config: RelayConfig) -> Result<Self, RelayError> {
        validate_config(config)?;
        Ok(Self {
            slots: HashMap::new(),
            config,
            next_relay: 1,
        })
    }

    pub fn create_slot(
        &mut self,
        size_bytes: u64,
        ttl_secs: Option<u64>,
        max_downloads: Option<u32>,
        upload_token_hash: RelayTokenHash,
        download_token_hash: RelayTokenHash,
        now: Timestamp,
    ) -> Result<CreatedRelaySlot, RelayError> {
        if size_bytes == 0 || size_bytes > self.config.max_blob_size {
            return Err(RelayError::InvalidSize {
                size_bytes,
                max: self.config.max_blob_size,
            });
        }
        let ttl_secs = ttl_secs.unwrap_or(self.config.default_ttl_secs);
        if ttl_secs == 0 || ttl_secs > self.config.max_ttl_secs {
            return Err(RelayError::InvalidTtl {
                ttl_secs,
                max: self.config.max_ttl_secs,
            });
        }
        let max_downloads = max_downloads.unwrap_or(self.config.default_max_downloads);
        if max_downloads == 0 || max_downloads > self.config.max_downloads {
            return Err(RelayError::InvalidMaxDownloads {
                max_downloads,
                max: self.config.max_downloads,
            });
        }

        let relay_id = self.next_relay_id();
        let expires_at = Timestamp(now.0.saturating_add(ttl_secs.saturating_mul(1000)));
        let metadata = RelayMetadata {
            relay_id: relay_id.clone(),
            size_bytes,
            max_downloads,
            download_count: 0,
            status: RelayStatus::Created,
            created_at: now,
            expires_at,
        };
        self.slots.insert(
            relay_id.clone(),
            RelaySlot {
                metadata,
                upload_token_hash,
                download_token_hash,
                encrypted_blob: None,
            },
        );

        Ok(CreatedRelaySlot {
            relay_id,
            expires_at,
        })
    }

    pub fn upload(
        &mut self,
        relay_id: &RelayId,
        upload_token_hash: &RelayTokenHash,
        encrypted_blob: Vec<u8>,
        now: Timestamp,
    ) -> Result<RelayMetadata, RelayError> {
        let slot = self.slot_mut(relay_id)?;
        reject_if_time_went_backwards(&slot.metadata, now)?;
        reject_if_expired(&slot.metadata, now)?;
        if slot.upload_token_hash != *upload_token_hash {
            return Err(RelayError::Unauthorized);
        }
        if slot.metadata.status == RelayStatus::Deleted {
            return Err(RelayError::Deleted(relay_id.clone()));
        }
        if slot.encrypted_blob.is_some() || slot.metadata.status == RelayStatus::Uploaded {
            return Err(RelayError::AlreadyUploaded(relay_id.clone()));
        }
        let actual = encrypted_blob.len() as u64;
        if actual != slot.metadata.size_bytes {
            return Err(RelayError::SizeMismatch {
                relay_id: relay_id.clone(),
                expected: slot.metadata.size_bytes,
                actual,
            });
        }

        slot.encrypted_blob = Some(encrypted_blob);
        slot.metadata.status = RelayStatus::Uploaded;
        Ok(slot.metadata.clone())
    }

    pub fn download(
        &mut self,
        relay_id: &RelayId,
        download_token_hash: &RelayTokenHash,
        now: Timestamp,
    ) -> Result<RelayDownload, RelayError> {
        let slot = self.slot_mut(relay_id)?;
        reject_if_time_went_backwards(&slot.metadata, now)?;
        reject_if_expired(&slot.metadata, now)?;
        if slot.download_token_hash != *download_token_hash {
            return Err(RelayError::Unauthorized);
        }
        if slot.metadata.status == RelayStatus::Deleted {
            return Err(RelayError::Deleted(relay_id.clone()));
        }
        if slot.metadata.status == RelayStatus::Created {
            return Err(RelayError::NotUploaded(relay_id.clone()));
        }
        if slot.metadata.download_count >= slot.metadata.max_downloads {
            slot.metadata.status = RelayStatus::Consumed;
            return Err(RelayError::DownloadLimitExceeded(relay_id.clone()));
        }

        let Some(encrypted_blob) = slot.encrypted_blob.clone() else {
            return Err(RelayError::NotUploaded(relay_id.clone()));
        };
        slot.metadata.download_count += 1;
        if slot.metadata.download_count >= slot.metadata.max_downloads {
            slot.metadata.status = RelayStatus::Consumed;
            slot.encrypted_blob = None;
        }

        Ok(RelayDownload {
            metadata: slot.metadata.clone(),
            encrypted_blob,
        })
    }

    pub fn delete(
        &mut self,
        relay_id: &RelayId,
        upload_token_hash: &RelayTokenHash,
        now: Timestamp,
    ) -> Result<RelayMetadata, RelayError> {
        let slot = self.slot_mut(relay_id)?;
        reject_if_time_went_backwards(&slot.metadata, now)?;
        if slot.upload_token_hash != *upload_token_hash {
            return Err(RelayError::Unauthorized);
        }
        slot.encrypted_blob = None;
        slot.metadata.status = RelayStatus::Deleted;
        Ok(slot.metadata.clone())
    }

    pub fn expire(&mut self, now: Timestamp) -> usize {
        let mut expired = 0;
        for slot in self.slots.values_mut() {
            if matches!(
                slot.metadata.status,
                RelayStatus::Consumed | RelayStatus::Deleted | RelayStatus::Expired
            ) {
                continue;
            }
            if now >= slot.metadata.expires_at {
                slot.encrypted_blob = None;
                slot.metadata.status = RelayStatus::Expired;
                expired += 1;
            }
        }
        expired
    }

    pub fn metadata(&self, relay_id: &RelayId) -> Option<&RelayMetadata> {
        self.slots.get(relay_id).map(|slot| &slot.metadata)
    }

    fn next_relay_id(&mut self) -> RelayId {
        let relay_id = RelayId::from(format!("relay-{}", self.next_relay));
        self.next_relay += 1;
        relay_id
    }

    fn slot_mut(&mut self, relay_id: &RelayId) -> Result<&mut RelaySlot, RelayError> {
        self.slots
            .get_mut(relay_id)
            .ok_or_else(|| RelayError::RelayNotFound(relay_id.clone()))
    }
}

fn validate_config(config: RelayConfig) -> Result<(), RelayError> {
    if config.max_blob_size == 0 {
        return Err(RelayError::InvalidConfig("max_blob_size must be > 0"));
    }
    if config.default_ttl_secs == 0 || config.default_ttl_secs > config.max_ttl_secs {
        return Err(RelayError::InvalidConfig(
            "default_ttl_secs must be between 1 and max_ttl_secs",
        ));
    }
    if config.default_max_downloads == 0 || config.default_max_downloads > config.max_downloads {
        return Err(RelayError::InvalidConfig(
            "default_max_downloads must be between 1 and max_downloads",
        ));
    }
    Ok(())
}

fn reject_if_time_went_backwards(
    metadata: &RelayMetadata,
    now: Timestamp,
) -> Result<(), RelayError> {
    if now < metadata.created_at {
        return Err(RelayError::TimestampWentBackwards {
            relay_id: metadata.relay_id.clone(),
            current: metadata.created_at,
            attempted: now,
        });
    }
    Ok(())
}

fn reject_if_expired(metadata: &RelayMetadata, now: Timestamp) -> Result<(), RelayError> {
    if now >= metadata.expires_at {
        return Err(RelayError::Expired(metadata.relay_id.clone()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(value: &str) -> RelayTokenHash {
        RelayTokenHash::from(value)
    }

    #[test]
    fn creates_uploads_and_downloads_encrypted_blob() {
        let mut relay = RelayCore::default();
        let created = relay
            .create_slot(
                3,
                Some(60),
                Some(2),
                hash("upload"),
                hash("download"),
                Timestamp(1),
            )
            .unwrap();

        let metadata = relay
            .upload(
                &created.relay_id,
                &hash("upload"),
                vec![1, 2, 3],
                Timestamp(2),
            )
            .unwrap();
        assert_eq!(metadata.status, RelayStatus::Uploaded);

        let download = relay
            .download(&created.relay_id, &hash("download"), Timestamp(3))
            .unwrap();
        assert_eq!(download.encrypted_blob, vec![1, 2, 3]);
        assert_eq!(download.metadata.download_count, 1);
    }

    #[test]
    fn rejects_wrong_tokens_and_size_mismatch() {
        let mut relay = RelayCore::default();
        let created = relay
            .create_slot(
                3,
                None,
                None,
                hash("upload"),
                hash("download"),
                Timestamp(1),
            )
            .unwrap();

        assert_eq!(
            relay
                .upload(
                    &created.relay_id,
                    &hash("wrong"),
                    vec![1, 2, 3],
                    Timestamp(2),
                )
                .unwrap_err(),
            RelayError::Unauthorized
        );
        assert_eq!(
            relay
                .upload(&created.relay_id, &hash("upload"), vec![1, 2], Timestamp(2),)
                .unwrap_err(),
            RelayError::SizeMismatch {
                relay_id: created.relay_id,
                expected: 3,
                actual: 2,
            }
        );
    }

    #[test]
    fn enforces_download_limit_and_expiration() {
        let mut relay = RelayCore::default();
        let created = relay
            .create_slot(
                1,
                Some(1),
                Some(1),
                hash("upload"),
                hash("download"),
                Timestamp(1_000),
            )
            .unwrap();
        relay
            .upload(
                &created.relay_id,
                &hash("upload"),
                vec![1],
                Timestamp(1_001),
            )
            .unwrap();

        relay
            .download(&created.relay_id, &hash("download"), Timestamp(1_002))
            .unwrap();
        assert_eq!(
            relay
                .download(&created.relay_id, &hash("download"), Timestamp(1_003))
                .unwrap_err(),
            RelayError::DownloadLimitExceeded(created.relay_id.clone())
        );

        let expired = relay.expire(Timestamp(2_000));
        assert_eq!(expired, 0);
    }

    #[test]
    fn expire_removes_unconsumed_blob() {
        let mut relay = RelayCore::default();
        let created = relay
            .create_slot(
                1,
                Some(1),
                Some(2),
                hash("upload"),
                hash("download"),
                Timestamp(1_000),
            )
            .unwrap();
        relay
            .upload(
                &created.relay_id,
                &hash("upload"),
                vec![1],
                Timestamp(1_001),
            )
            .unwrap();

        assert_eq!(relay.expire(Timestamp(2_000)), 1);
        assert_eq!(
            relay
                .download(&created.relay_id, &hash("download"), Timestamp(2_001))
                .unwrap_err(),
            RelayError::Expired(created.relay_id)
        );
    }
}
