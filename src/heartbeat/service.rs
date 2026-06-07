use std::error::Error;
use std::fmt;
use std::future::Future;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, oneshot};
use tokio::time::{self, MissedTickBehavior};

use super::{
    AgentId, HeartbeatConfig, HeartbeatConfigError, HeartbeatCore, HeartbeatEvent, PingOutcome,
};

const DEFAULT_COMMAND_BUFFER: usize = 128;
const DEFAULT_PUBLISH_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeartbeatServiceOptions {
    pub command_buffer: usize,
    pub publish_timeout: Duration,
}

impl Default for HeartbeatServiceOptions {
    fn default() -> Self {
        Self {
            command_buffer: DEFAULT_COMMAND_BUFFER,
            publish_timeout: DEFAULT_PUBLISH_TIMEOUT,
        }
    }
}

impl HeartbeatServiceOptions {
    fn validate(&self) -> Result<(), HeartbeatServiceStartError> {
        if self.command_buffer == 0 {
            return Err(HeartbeatServiceStartError::ZeroCommandBuffer);
        }
        if self.publish_timeout.is_zero() {
            return Err(HeartbeatServiceStartError::ZeroPublishTimeout);
        }

        Ok(())
    }
}

#[derive(Debug)]
pub struct PublishError {
    message: String,
}

impl PublishError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for PublishError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for PublishError {}

pub trait HeartbeatEventSink: Send + Sync + 'static {
    fn publish(
        &self,
        event: HeartbeatEvent,
    ) -> impl Future<Output = Result<(), PublishError>> + Send;
}

#[derive(Debug)]
pub enum HeartbeatCommand {
    Ping {
        agent_id: AgentId,
        busy: bool,
        reply: oneshot::Sender<PingOutcome>,
    },
    IsAlive {
        agent_id: AgentId,
        reply: oneshot::Sender<bool>,
    },
    Forget {
        agent_id: AgentId,
        reply: oneshot::Sender<bool>,
    },
    Shutdown {
        reply: oneshot::Sender<()>,
    },
}

#[derive(Clone, Debug)]
pub struct HeartbeatHandle {
    commands: mpsc::Sender<HeartbeatCommand>,
}

impl HeartbeatHandle {
    pub async fn ping(
        &self,
        agent_id: impl Into<AgentId>,
        busy: bool,
    ) -> Result<PingOutcome, HeartbeatServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(HeartbeatCommand::Ping {
            agent_id: agent_id.into(),
            busy,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| HeartbeatServiceError::ResponseDropped)
    }

    pub async fn is_alive(
        &self,
        agent_id: impl Into<AgentId>,
    ) -> Result<bool, HeartbeatServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(HeartbeatCommand::IsAlive {
            agent_id: agent_id.into(),
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| HeartbeatServiceError::ResponseDropped)
    }

    pub async fn forget(
        &self,
        agent_id: impl Into<AgentId>,
    ) -> Result<bool, HeartbeatServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(HeartbeatCommand::Forget {
            agent_id: agent_id.into(),
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| HeartbeatServiceError::ResponseDropped)
    }

    pub async fn shutdown(&self) -> Result<(), HeartbeatServiceError> {
        let (reply, response) = oneshot::channel();
        self.send(HeartbeatCommand::Shutdown { reply }).await?;
        response
            .await
            .map_err(|_| HeartbeatServiceError::ResponseDropped)
    }

    async fn send(&self, command: HeartbeatCommand) -> Result<(), HeartbeatServiceError> {
        self.commands
            .send(command)
            .await
            .map_err(|_| HeartbeatServiceError::Stopped)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeartbeatServiceError {
    Stopped,
    ResponseDropped,
}

impl fmt::Display for HeartbeatServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HeartbeatServiceError::Stopped => f.write_str("heartbeat service is stopped"),
            HeartbeatServiceError::ResponseDropped => {
                f.write_str("heartbeat service dropped the response")
            }
        }
    }
}

impl Error for HeartbeatServiceError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeartbeatServiceStartError {
    InvalidConfig(HeartbeatConfigError),
    ZeroCommandBuffer,
    ZeroPublishTimeout,
}

impl fmt::Display for HeartbeatServiceStartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HeartbeatServiceStartError::InvalidConfig(error) => {
                write!(f, "invalid heartbeat config: {error}")
            }
            HeartbeatServiceStartError::ZeroCommandBuffer => {
                f.write_str("heartbeat command buffer must be greater than zero")
            }
            HeartbeatServiceStartError::ZeroPublishTimeout => {
                f.write_str("heartbeat publish timeout must be greater than zero")
            }
        }
    }
}

impl Error for HeartbeatServiceStartError {}

pub struct HeartbeatService<S> {
    core: HeartbeatCore,
    sink: S,
    commands: mpsc::Receiver<HeartbeatCommand>,
    options: HeartbeatServiceOptions,
}

impl<S> HeartbeatService<S>
where
    S: HeartbeatEventSink,
{
    pub fn spawn(
        config: HeartbeatConfig,
        sink: S,
    ) -> Result<HeartbeatHandle, HeartbeatServiceStartError> {
        Self::spawn_with_options(config, sink, HeartbeatServiceOptions::default())
    }

    pub fn spawn_with_buffer(
        config: HeartbeatConfig,
        sink: S,
        command_buffer: usize,
    ) -> Result<HeartbeatHandle, HeartbeatServiceStartError> {
        Self::spawn_with_options(
            config,
            sink,
            HeartbeatServiceOptions {
                command_buffer,
                ..HeartbeatServiceOptions::default()
            },
        )
    }

    pub fn spawn_with_options(
        config: HeartbeatConfig,
        sink: S,
        options: HeartbeatServiceOptions,
    ) -> Result<HeartbeatHandle, HeartbeatServiceStartError> {
        config
            .validate()
            .map_err(HeartbeatServiceStartError::InvalidConfig)?;
        options.validate()?;

        let (commands, receiver) = mpsc::channel(options.command_buffer);
        let service = Self {
            core: HeartbeatCore::new(config),
            sink,
            commands: receiver,
            options,
        };

        tokio::spawn(service.run());

        Ok(HeartbeatHandle { commands })
    }

    async fn run(mut self) {
        let mut ticker = time::interval(self.core.config().scan_interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut shutdown_reply = None;

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    let events = self.core.scan(Instant::now());
                    self.publish_events(events).await;
                }
                command = self.commands.recv() => {
                    let Some(command) = command else {
                        break;
                    };

                    if let Some(reply) = self.handle_command(command).await {
                        shutdown_reply = Some(reply);
                        break;
                    }
                }
            }
        }

        if let Some(reply) = shutdown_reply {
            let _ = reply.send(());
        }
    }

    async fn handle_command(&mut self, command: HeartbeatCommand) -> Option<oneshot::Sender<()>> {
        match command {
            HeartbeatCommand::Ping {
                agent_id,
                busy,
                reply,
            } => {
                let recovered_agent_id = agent_id.clone();
                let outcome = self.core.ping(agent_id, busy, Instant::now());
                let _ = reply.send(outcome);

                if outcome == PingOutcome::RecoveredFromTimeout {
                    self.publish_events(vec![HeartbeatEvent::AgentRecovered {
                        agent_id: recovered_agent_id,
                    }])
                    .await;
                }

                None
            }
            HeartbeatCommand::IsAlive { agent_id, reply } => {
                let alive = self.core.is_alive(&agent_id, Instant::now());
                let _ = reply.send(alive);
                None
            }
            HeartbeatCommand::Forget { agent_id, reply } => {
                let removed = self.core.forget(&agent_id);
                let _ = reply.send(removed);
                None
            }
            HeartbeatCommand::Shutdown { reply } => Some(reply),
        }
    }

    async fn publish_events(&self, events: Vec<HeartbeatEvent>) {
        for event in events {
            match time::timeout(self.options.publish_timeout, self.sink.publish(event)).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    eprintln!("failed to publish heartbeat event: {error}");
                }
                Err(_) => {
                    eprintln!("timed out publishing heartbeat event");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use tokio::sync::Mutex;

    use super::*;

    #[derive(Clone, Default)]
    struct RecordingSink {
        events: Arc<Mutex<Vec<HeartbeatEvent>>>,
    }

    impl RecordingSink {
        async fn events(&self) -> Vec<HeartbeatEvent> {
            self.events.lock().await.clone()
        }
    }

    impl HeartbeatEventSink for RecordingSink {
        async fn publish(&self, event: HeartbeatEvent) -> Result<(), PublishError> {
            self.events.lock().await.push(event);
            Ok(())
        }
    }

    struct FailingSink;

    impl HeartbeatEventSink for FailingSink {
        async fn publish(&self, _event: HeartbeatEvent) -> Result<(), PublishError> {
            Err(PublishError::new("sink is unavailable"))
        }
    }

    struct SlowSink;

    impl HeartbeatEventSink for SlowSink {
        async fn publish(&self, _event: HeartbeatEvent) -> Result<(), PublishError> {
            time::sleep(Duration::from_secs(60)).await;
            Ok(())
        }
    }

    fn fast_config() -> HeartbeatConfig {
        HeartbeatConfig {
            scan_interval: Duration::from_millis(5),
            idle_timeout: Duration::from_millis(50),
            busy_timeout: Duration::from_millis(10),
        }
    }

    #[tokio::test]
    async fn service_accepts_ping_and_reports_alive() {
        let sink = RecordingSink::default();
        let handle = HeartbeatService::spawn(fast_config(), sink).unwrap();

        let outcome = handle.ping("agent-1", false).await.unwrap();
        let alive = handle.is_alive("agent-1").await.unwrap();

        assert_eq!(outcome, PingOutcome::FirstSeen);
        assert!(alive);

        handle.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn service_publishes_timeout_events() {
        let sink = RecordingSink::default();
        let handle = HeartbeatService::spawn(fast_config(), sink.clone()).unwrap();

        handle.ping("agent-1", true).await.unwrap();
        time::sleep(Duration::from_millis(30)).await;

        assert_eq!(
            sink.events().await,
            vec![HeartbeatEvent::AgentTimedOut {
                agent_id: AgentId::from("agent-1")
            }]
        );
        assert!(!handle.is_alive("agent-1").await.unwrap());

        handle.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn service_forget_removes_agent() {
        let sink = RecordingSink::default();
        let handle = HeartbeatService::spawn(fast_config(), sink.clone()).unwrap();

        handle.ping("agent-1", true).await.unwrap();

        assert!(handle.forget("agent-1").await.unwrap());
        time::sleep(Duration::from_millis(30)).await;

        assert!(!handle.is_alive("agent-1").await.unwrap());
        assert!(sink.events().await.is_empty());

        handle.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn service_stops_after_shutdown() {
        let sink = RecordingSink::default();
        let handle = HeartbeatService::spawn(fast_config(), sink).unwrap();

        handle.shutdown().await.unwrap();

        assert_eq!(
            handle.ping("agent-1", false).await.unwrap_err(),
            HeartbeatServiceError::Stopped
        );
    }

    #[tokio::test]
    async fn publish_failure_does_not_stop_service() {
        let handle = HeartbeatService::spawn(fast_config(), FailingSink).unwrap();

        handle.ping("agent-1", true).await.unwrap();
        time::sleep(Duration::from_millis(30)).await;

        handle.ping("agent-1", false).await.unwrap();
        assert!(handle.is_alive("agent-1").await.unwrap());

        handle.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn service_publishes_recovered_event() {
        let sink = RecordingSink::default();
        let handle = HeartbeatService::spawn(fast_config(), sink.clone()).unwrap();

        handle.ping("agent-1", true).await.unwrap();
        time::sleep(Duration::from_millis(30)).await;
        handle.ping("agent-1", false).await.unwrap();

        assert_eq!(
            sink.events().await,
            vec![
                HeartbeatEvent::AgentTimedOut {
                    agent_id: AgentId::from("agent-1")
                },
                HeartbeatEvent::AgentRecovered {
                    agent_id: AgentId::from("agent-1")
                }
            ]
        );

        handle.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn service_rejects_invalid_startup_options() {
        assert_eq!(
            HeartbeatService::spawn(
                HeartbeatConfig {
                    scan_interval: Duration::ZERO,
                    idle_timeout: Duration::from_millis(50),
                    busy_timeout: Duration::from_millis(10),
                },
                RecordingSink::default(),
            )
            .unwrap_err(),
            HeartbeatServiceStartError::InvalidConfig(HeartbeatConfigError::ZeroScanInterval)
        );

        assert_eq!(
            HeartbeatService::spawn_with_buffer(fast_config(), RecordingSink::default(), 0)
                .unwrap_err(),
            HeartbeatServiceStartError::ZeroCommandBuffer
        );

        assert_eq!(
            HeartbeatService::spawn_with_options(
                fast_config(),
                RecordingSink::default(),
                HeartbeatServiceOptions {
                    command_buffer: 1,
                    publish_timeout: Duration::ZERO,
                }
            )
            .unwrap_err(),
            HeartbeatServiceStartError::ZeroPublishTimeout
        );
    }

    #[tokio::test]
    async fn slow_publish_does_not_block_service_forever() {
        let handle = HeartbeatService::spawn_with_options(
            fast_config(),
            SlowSink,
            HeartbeatServiceOptions {
                command_buffer: 8,
                publish_timeout: Duration::from_millis(5),
            },
        )
        .unwrap();

        handle.ping("agent-1", true).await.unwrap();
        time::sleep(Duration::from_millis(30)).await;

        handle.ping("agent-1", false).await.unwrap();
        assert!(handle.is_alive("agent-1").await.unwrap());

        handle.shutdown().await.unwrap();
    }
}
