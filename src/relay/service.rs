use std::error::Error;
use std::fmt;

use tokio::sync::{mpsc, oneshot};

use crate::types::Timestamp;

use super::RelayCore;
use super::types::{
    CreatedRelaySlot, RelayConfig, RelayDownload, RelayError, RelayId, RelayMetadata,
    RelayTokenHash,
};

const DEFAULT_COMMAND_BUFFER: usize = 128;

#[derive(Debug)]
pub enum RelayCommand {
    CreateSlot {
        size_bytes: u64,
        ttl_secs: Option<u64>,
        max_downloads: Option<u32>,
        upload_token_hash: RelayTokenHash,
        download_token_hash: RelayTokenHash,
        now: Timestamp,
        reply: oneshot::Sender<Result<CreatedRelaySlot, RelayError>>,
    },
    Upload {
        relay_id: RelayId,
        upload_token_hash: RelayTokenHash,
        encrypted_blob: Vec<u8>,
        now: Timestamp,
        reply: oneshot::Sender<Result<RelayMetadata, RelayError>>,
    },
    Download {
        relay_id: RelayId,
        download_token_hash: RelayTokenHash,
        now: Timestamp,
        reply: oneshot::Sender<Result<RelayDownload, RelayError>>,
    },
    Delete {
        relay_id: RelayId,
        upload_token_hash: RelayTokenHash,
        now: Timestamp,
        reply: oneshot::Sender<Result<RelayMetadata, RelayError>>,
    },
    Expire {
        now: Timestamp,
        reply: oneshot::Sender<usize>,
    },
    Shutdown {
        reply: oneshot::Sender<()>,
    },
}

#[derive(Clone, Debug)]
pub struct RelayHandle {
    commands: mpsc::Sender<RelayCommand>,
}

impl RelayHandle {
    pub async fn create_slot(
        &self,
        size_bytes: u64,
        ttl_secs: Option<u64>,
        max_downloads: Option<u32>,
        upload_token_hash: RelayTokenHash,
        download_token_hash: RelayTokenHash,
        now: Timestamp,
    ) -> Result<CreatedRelaySlot, RelayServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(RelayCommand::CreateSlot {
            size_bytes,
            ttl_secs,
            max_downloads,
            upload_token_hash,
            download_token_hash,
            now,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| RelayServiceError::ResponseDropped)?
            .map_err(RelayServiceError::Relay)
    }

    pub async fn upload(
        &self,
        relay_id: impl Into<RelayId>,
        upload_token_hash: RelayTokenHash,
        encrypted_blob: Vec<u8>,
        now: Timestamp,
    ) -> Result<RelayMetadata, RelayServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(RelayCommand::Upload {
            relay_id: relay_id.into(),
            upload_token_hash,
            encrypted_blob,
            now,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| RelayServiceError::ResponseDropped)?
            .map_err(RelayServiceError::Relay)
    }

    pub async fn download(
        &self,
        relay_id: impl Into<RelayId>,
        download_token_hash: RelayTokenHash,
        now: Timestamp,
    ) -> Result<RelayDownload, RelayServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(RelayCommand::Download {
            relay_id: relay_id.into(),
            download_token_hash,
            now,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| RelayServiceError::ResponseDropped)?
            .map_err(RelayServiceError::Relay)
    }

    pub async fn delete(
        &self,
        relay_id: impl Into<RelayId>,
        upload_token_hash: RelayTokenHash,
        now: Timestamp,
    ) -> Result<RelayMetadata, RelayServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(RelayCommand::Delete {
            relay_id: relay_id.into(),
            upload_token_hash,
            now,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| RelayServiceError::ResponseDropped)?
            .map_err(RelayServiceError::Relay)
    }

    pub async fn expire(&self, now: Timestamp) -> Result<usize, RelayServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(RelayCommand::Expire { now, reply }).await?;
        response
            .await
            .map_err(|_| RelayServiceError::ResponseDropped)
    }

    pub async fn shutdown(&self) -> Result<(), RelayServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(RelayCommand::Shutdown { reply }).await?;
        response
            .await
            .map_err(|_| RelayServiceError::ResponseDropped)
    }

    async fn send(&self, command: RelayCommand) -> Result<(), RelayServiceError> {
        self.commands
            .send(command)
            .await
            .map_err(|_| RelayServiceError::Stopped)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RelayServiceError {
    Relay(RelayError),
    Stopped,
    ResponseDropped,
}

impl fmt::Display for RelayServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RelayServiceError::Relay(error) => write!(f, "{error}"),
            RelayServiceError::Stopped => f.write_str("relay service is stopped"),
            RelayServiceError::ResponseDropped => f.write_str("relay service dropped the response"),
        }
    }
}

impl Error for RelayServiceError {}

pub struct RelayService {
    core: RelayCore,
    commands: mpsc::Receiver<RelayCommand>,
}

impl RelayService {
    pub fn spawn() -> RelayHandle {
        Self::spawn_with_config(RelayConfig::default()).expect("default relay config should start")
    }

    pub fn spawn_with_config(config: RelayConfig) -> Result<RelayHandle, RelayError> {
        let (commands, receiver) = mpsc::channel(DEFAULT_COMMAND_BUFFER);
        let service = Self {
            core: RelayCore::new(config)?,
            commands: receiver,
        };

        tokio::spawn(service.run());

        Ok(RelayHandle { commands })
    }

    async fn run(mut self) {
        let mut shutdown_reply = None;

        while let Some(command) = self.commands.recv().await {
            if let Some(reply) = self.handle_command(command) {
                shutdown_reply = Some(reply);
                break;
            }
        }

        if let Some(reply) = shutdown_reply {
            let _ = reply.send(());
        }
    }

    fn handle_command(&mut self, command: RelayCommand) -> Option<oneshot::Sender<()>> {
        match command {
            RelayCommand::CreateSlot {
                size_bytes,
                ttl_secs,
                max_downloads,
                upload_token_hash,
                download_token_hash,
                now,
                reply,
            } => {
                let _ = reply.send(self.core.create_slot(
                    size_bytes,
                    ttl_secs,
                    max_downloads,
                    upload_token_hash,
                    download_token_hash,
                    now,
                ));
                None
            }
            RelayCommand::Upload {
                relay_id,
                upload_token_hash,
                encrypted_blob,
                now,
                reply,
            } => {
                let _ = reply.send(self.core.upload(
                    &relay_id,
                    &upload_token_hash,
                    encrypted_blob,
                    now,
                ));
                None
            }
            RelayCommand::Download {
                relay_id,
                download_token_hash,
                now,
                reply,
            } => {
                let _ = reply.send(self.core.download(&relay_id, &download_token_hash, now));
                None
            }
            RelayCommand::Delete {
                relay_id,
                upload_token_hash,
                now,
                reply,
            } => {
                let _ = reply.send(self.core.delete(&relay_id, &upload_token_hash, now));
                None
            }
            RelayCommand::Expire { now, reply } => {
                let _ = reply.send(self.core.expire(now));
                None
            }
            RelayCommand::Shutdown { reply } => Some(reply),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(value: &str) -> RelayTokenHash {
        RelayTokenHash::from(value)
    }

    #[tokio::test]
    async fn service_creates_uploads_and_downloads_blob() {
        let relay = RelayService::spawn();
        let created = relay
            .create_slot(3, Some(60), Some(1), hash("up"), hash("down"), Timestamp(1))
            .await
            .unwrap();
        relay
            .upload(
                created.relay_id.clone(),
                hash("up"),
                vec![1, 2, 3],
                Timestamp(2),
            )
            .await
            .unwrap();
        let download = relay
            .download(created.relay_id, hash("down"), Timestamp(3))
            .await
            .unwrap();

        assert_eq!(download.encrypted_blob, vec![1, 2, 3]);
        relay.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn service_stops_after_shutdown() {
        let relay = RelayService::spawn();

        relay.shutdown().await.unwrap();

        assert_eq!(
            relay.expire(Timestamp(1)).await.unwrap_err(),
            RelayServiceError::Stopped
        );
    }
}
