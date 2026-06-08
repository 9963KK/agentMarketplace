use std::collections::{HashMap, HashSet};

use crate::artifact::{ArtifactManifest, validate_manifest_submission};
use crate::heartbeat::AgentId;
use crate::types::{AssignmentId, OutputHash, SessionId, TaskId, Timestamp};

use super::types::{
    Assignment, AssignmentKind, AssignmentStatus, LiveSession, LiveSessionError, LiveSessionStatus,
};

#[derive(Debug, Default)]
pub struct LiveSessionCore {
    sessions: HashMap<SessionId, LiveSession>,
    assignments: HashMap<AssignmentId, Assignment>,
    assignments_by_task: HashMap<TaskId, HashSet<AssignmentId>>,
    assignments_by_session: HashMap<SessionId, HashSet<AssignmentId>>,
    assignments_by_agent: HashMap<AgentId, HashSet<AssignmentId>>,
    next_session: u64,
    next_assignment: u64,
}

impl LiveSessionCore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_session(&mut self, task_id: TaskId, at: Timestamp) -> SessionId {
        let session_id = self.next_session_id();
        self.sessions.insert(
            session_id.clone(),
            LiveSession {
                session_id: session_id.clone(),
                task_id,
                assignment_ids: HashSet::new(),
                status: LiveSessionStatus::Running,
                created_at: at,
                updated_at: at,
            },
        );
        session_id
    }

    pub fn close_session(
        &mut self,
        session_id: &SessionId,
        at: Timestamp,
    ) -> Result<(), LiveSessionError> {
        self.validate_running_session_at(session_id, at)?;
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| LiveSessionError::SessionNotFound(session_id.clone()))?;
        session.status = LiveSessionStatus::Closed;
        session.updated_at = at;
        Ok(())
    }

    pub fn assign(
        &mut self,
        task_id: TaskId,
        session_id: &SessionId,
        agent_id: AgentId,
        kind: AssignmentKind,
        at: Timestamp,
    ) -> Result<AssignmentId, LiveSessionError> {
        self.validate_running_session_at(session_id, at)?;
        let session_task_id = self
            .sessions
            .get(session_id)
            .ok_or_else(|| LiveSessionError::SessionNotFound(session_id.clone()))?
            .task_id
            .clone();
        if session_task_id != task_id {
            return Err(LiveSessionError::SessionTaskMismatch {
                session_id: session_id.clone(),
                expected_task_id: session_task_id,
                actual_task_id: task_id,
            });
        }
        self.validate_assignment_kind(&task_id, &kind)?;

        let assignment_id = self.next_assignment_id();
        self.assignments.insert(
            assignment_id.clone(),
            Assignment {
                assignment_id: assignment_id.clone(),
                task_id: task_id.clone(),
                session_id: session_id.clone(),
                agent_id: agent_id.clone(),
                kind,
                status: AssignmentStatus::Assigned,
                output_hash: None,
                created_at: at,
                updated_at: at,
            },
        );
        self.sessions
            .get_mut(session_id)
            .expect("session was validated")
            .assignment_ids
            .insert(assignment_id.clone());
        self.sessions
            .get_mut(session_id)
            .expect("session was validated")
            .updated_at = at;
        self.assignments_by_task
            .entry(task_id)
            .or_default()
            .insert(assignment_id.clone());
        self.assignments_by_session
            .entry(session_id.clone())
            .or_default()
            .insert(assignment_id.clone());
        self.assignments_by_agent
            .entry(agent_id)
            .or_default()
            .insert(assignment_id.clone());

        Ok(assignment_id)
    }

    pub fn submit_output(
        &mut self,
        assignment_id: &AssignmentId,
        agent_id: AgentId,
        output_hash: OutputHash,
        at: Timestamp,
    ) -> Result<(), LiveSessionError> {
        self.validate_assigned_assignment_at(assignment_id, at)?;
        let session_id = {
            let assignment = self
                .assignments
                .get_mut(assignment_id)
                .ok_or_else(|| LiveSessionError::AssignmentNotFound(assignment_id.clone()))?;
            if assignment.agent_id != agent_id {
                return Err(LiveSessionError::AgentMismatch {
                    assignment_id: assignment_id.clone(),
                    expected: assignment.agent_id.clone(),
                    actual: agent_id,
                });
            }

            assignment.output_hash = Some(output_hash);
            assignment.status = AssignmentStatus::Submitted;
            assignment.updated_at = at;
            assignment.session_id.clone()
        };
        self.touch_session(&session_id, at)?;

        Ok(())
    }

    pub fn submit_artifact(
        &mut self,
        assignment_id: &AssignmentId,
        agent_id: AgentId,
        manifest: ArtifactManifest,
        at: Timestamp,
    ) -> Result<(), LiveSessionError> {
        let manifest_hash = validate_manifest_submission(&manifest, assignment_id, &agent_id)
            .map_err(LiveSessionError::InvalidArtifact)?;

        self.submit_output(
            assignment_id,
            agent_id,
            OutputHash::from(manifest_hash.to_string()),
            at,
        )
    }

    pub fn mark_approved(
        &mut self,
        assignment_id: &AssignmentId,
        at: Timestamp,
    ) -> Result<(), LiveSessionError> {
        self.mark_reviewed(assignment_id, AssignmentStatus::Approved, at)
    }

    pub fn mark_rejected(
        &mut self,
        assignment_id: &AssignmentId,
        at: Timestamp,
    ) -> Result<(), LiveSessionError> {
        self.mark_reviewed(assignment_id, AssignmentStatus::Rejected, at)
    }

    pub fn cancel_assignment(
        &mut self,
        assignment_id: &AssignmentId,
        at: Timestamp,
    ) -> Result<(), LiveSessionError> {
        self.validate_assignment_at(assignment_id, at)?;
        let session_id = {
            let assignment = self
                .assignments
                .get_mut(assignment_id)
                .ok_or_else(|| LiveSessionError::AssignmentNotFound(assignment_id.clone()))?;
            assignment.status = AssignmentStatus::Cancelled;
            assignment.updated_at = at;
            assignment.session_id.clone()
        };
        self.touch_session(&session_id, at)
    }

    pub fn get_session(&self, session_id: &SessionId) -> Option<&LiveSession> {
        self.sessions.get(session_id)
    }

    pub fn get_assignment(&self, assignment_id: &AssignmentId) -> Option<&Assignment> {
        self.assignments.get(assignment_id)
    }

    pub fn assignments_by_task(&self, task_id: &TaskId) -> Vec<Assignment> {
        self.assignments_from_index(self.assignments_by_task.get(task_id))
    }

    pub fn assignments_by_session(&self, session_id: &SessionId) -> Vec<Assignment> {
        self.assignments_from_index(self.assignments_by_session.get(session_id))
    }

    pub fn assignments_by_agent(&self, agent_id: &AgentId) -> Vec<Assignment> {
        self.assignments_from_index(self.assignments_by_agent.get(agent_id))
    }

    fn mark_reviewed(
        &mut self,
        assignment_id: &AssignmentId,
        status: AssignmentStatus,
        at: Timestamp,
    ) -> Result<(), LiveSessionError> {
        self.validate_submitted_assignment_at(assignment_id, at)?;
        let session_id = {
            let assignment = self
                .assignments
                .get_mut(assignment_id)
                .ok_or_else(|| LiveSessionError::AssignmentNotFound(assignment_id.clone()))?;
            assignment.status = status;
            assignment.updated_at = at;
            assignment.session_id.clone()
        };
        self.touch_session(&session_id, at)
    }

    fn validate_assignment_kind(
        &self,
        task_id: &TaskId,
        kind: &AssignmentKind,
    ) -> Result<(), LiveSessionError> {
        let AssignmentKind::Review {
            target_assignment_id,
        } = kind
        else {
            return Ok(());
        };
        let target = self.assignments.get(target_assignment_id).ok_or_else(|| {
            LiveSessionError::TargetAssignmentNotFound(target_assignment_id.clone())
        })?;
        if target.kind != AssignmentKind::Execute {
            return Err(LiveSessionError::TargetAssignmentKindMismatch {
                target_assignment_id: target_assignment_id.clone(),
                kind: target.kind.clone(),
            });
        }
        if target.task_id != *task_id {
            return Err(LiveSessionError::TargetAssignmentTaskMismatch {
                target_assignment_id: target_assignment_id.clone(),
                expected_task_id: task_id.clone(),
                actual_task_id: target.task_id.clone(),
            });
        }

        Ok(())
    }

    fn validate_running_session_at(
        &self,
        session_id: &SessionId,
        at: Timestamp,
    ) -> Result<(), LiveSessionError> {
        let session = self
            .sessions
            .get(session_id)
            .ok_or_else(|| LiveSessionError::SessionNotFound(session_id.clone()))?;
        if session.status != LiveSessionStatus::Running {
            return Err(LiveSessionError::SessionNotRunning {
                session_id: session_id.clone(),
                status: session.status,
            });
        }
        if at < session.updated_at {
            return Err(LiveSessionError::TimestampWentBackwards {
                current: session.updated_at,
                attempted: at,
            });
        }

        Ok(())
    }

    fn validate_assignment_at(
        &self,
        assignment_id: &AssignmentId,
        at: Timestamp,
    ) -> Result<(), LiveSessionError> {
        let assignment = self
            .assignments
            .get(assignment_id)
            .ok_or_else(|| LiveSessionError::AssignmentNotFound(assignment_id.clone()))?;
        if at < assignment.updated_at {
            return Err(LiveSessionError::TimestampWentBackwards {
                current: assignment.updated_at,
                attempted: at,
            });
        }

        Ok(())
    }

    fn validate_assigned_assignment_at(
        &self,
        assignment_id: &AssignmentId,
        at: Timestamp,
    ) -> Result<(), LiveSessionError> {
        self.validate_assignment_at(assignment_id, at)?;
        let assignment = self
            .assignments
            .get(assignment_id)
            .ok_or_else(|| LiveSessionError::AssignmentNotFound(assignment_id.clone()))?;
        if assignment.status != AssignmentStatus::Assigned {
            return Err(LiveSessionError::AssignmentNotAssigned {
                assignment_id: assignment_id.clone(),
                status: assignment.status,
            });
        }

        Ok(())
    }

    fn validate_submitted_assignment_at(
        &self,
        assignment_id: &AssignmentId,
        at: Timestamp,
    ) -> Result<(), LiveSessionError> {
        self.validate_assignment_at(assignment_id, at)?;
        let assignment = self
            .assignments
            .get(assignment_id)
            .ok_or_else(|| LiveSessionError::AssignmentNotFound(assignment_id.clone()))?;
        if assignment.status != AssignmentStatus::Submitted {
            return Err(LiveSessionError::AssignmentNotSubmitted {
                assignment_id: assignment_id.clone(),
                status: assignment.status,
            });
        }

        Ok(())
    }

    fn touch_session(
        &mut self,
        session_id: &SessionId,
        at: Timestamp,
    ) -> Result<(), LiveSessionError> {
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| LiveSessionError::SessionNotFound(session_id.clone()))?;
        if at < session.updated_at {
            return Err(LiveSessionError::TimestampWentBackwards {
                current: session.updated_at,
                attempted: at,
            });
        }
        session.updated_at = at;
        Ok(())
    }

    fn assignments_from_index(
        &self,
        assignment_ids: Option<&HashSet<AssignmentId>>,
    ) -> Vec<Assignment> {
        let Some(assignment_ids) = assignment_ids else {
            return Vec::new();
        };
        let mut assignment_ids = assignment_ids.iter().cloned().collect::<Vec<_>>();
        assignment_ids.sort();
        assignment_ids
            .into_iter()
            .filter_map(|assignment_id| self.assignments.get(&assignment_id).cloned())
            .collect()
    }

    fn next_session_id(&mut self) -> SessionId {
        self.next_session += 1;
        SessionId::new(format!("session-{}", self.next_session))
    }

    fn next_assignment_id(&mut self) -> AssignmentId {
        self.next_assignment += 1;
        AssignmentId::new(format!("assignment-{}", self.next_assignment))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::{
        ArtifactError, ArtifactFile, ArtifactKind, ArtifactManifest, HashDigest, MediaProfileId,
        seal_manifest,
    };

    fn agent(id: &str) -> AgentId {
        AgentId::from(id)
    }

    fn task(id: &str) -> TaskId {
        TaskId::from(id)
    }

    fn output(value: &str) -> OutputHash {
        OutputHash::from(value)
    }

    fn hash(value: u8) -> HashDigest {
        HashDigest::from_sha256_hex(format!("{value:064x}")).unwrap()
    }

    fn text_manifest(
        artifact_id: &str,
        task_id: TaskId,
        assignment_id: AssignmentId,
        producer_agent_id: AgentId,
        at: Timestamp,
    ) -> ArtifactManifest {
        let file = ArtifactFile::new(
            "https://agent.example/report.md",
            hash(1),
            "text/markdown",
            "text.markdown.utf8.v1",
            120,
        );
        seal_manifest(ArtifactManifest::new(
            artifact_id,
            task_id,
            assignment_id,
            producer_agent_id,
            ArtifactKind::Single,
            vec![file],
            at,
        ))
        .unwrap()
    }

    #[test]
    fn create_session_and_assign_execute_work() {
        let mut core = LiveSessionCore::new();

        let session_id = core.create_session(task("task-1"), Timestamp(1));
        let assignment_id = core
            .assign(
                task("task-1"),
                &session_id,
                agent("executor"),
                AssignmentKind::Execute,
                Timestamp(2),
            )
            .unwrap();

        let assignment = core.get_assignment(&assignment_id).unwrap();
        assert_eq!(assignment.status, AssignmentStatus::Assigned);
        assert_eq!(assignment.agent_id, agent("executor"));
        assert_eq!(core.assignments_by_session(&session_id).len(), 1);
        assert_eq!(core.assignments_by_agent(&agent("executor")).len(), 1);
    }

    #[test]
    fn review_assignment_targets_execute_assignment() {
        let mut core = LiveSessionCore::new();
        let session_id = core.create_session(task("task-1"), Timestamp(1));
        let execute = core
            .assign(
                task("task-1"),
                &session_id,
                agent("executor"),
                AssignmentKind::Execute,
                Timestamp(2),
            )
            .unwrap();

        let review = core
            .assign(
                task("task-1"),
                &session_id,
                agent("reviewer"),
                AssignmentKind::Review {
                    target_assignment_id: execute.clone(),
                },
                Timestamp(3),
            )
            .unwrap();

        assert_eq!(
            core.get_assignment(&review).unwrap().kind,
            AssignmentKind::Review {
                target_assignment_id: execute
            }
        );
    }

    #[test]
    fn review_assignment_rejects_missing_target() {
        let mut core = LiveSessionCore::new();
        let session_id = core.create_session(task("task-1"), Timestamp(1));

        assert_eq!(
            core.assign(
                task("task-1"),
                &session_id,
                agent("reviewer"),
                AssignmentKind::Review {
                    target_assignment_id: AssignmentId::from("missing"),
                },
                Timestamp(2),
            )
            .unwrap_err(),
            LiveSessionError::TargetAssignmentNotFound(AssignmentId::from("missing"))
        );
    }

    #[test]
    fn review_assignment_rejects_non_execute_target() {
        let mut core = LiveSessionCore::new();
        let session_id = core.create_session(task("task-1"), Timestamp(1));
        let execute = core
            .assign(
                task("task-1"),
                &session_id,
                agent("executor"),
                AssignmentKind::Execute,
                Timestamp(2),
            )
            .unwrap();
        let review = core
            .assign(
                task("task-1"),
                &session_id,
                agent("reviewer-1"),
                AssignmentKind::Review {
                    target_assignment_id: execute,
                },
                Timestamp(3),
            )
            .unwrap();

        assert_eq!(
            core.assign(
                task("task-1"),
                &session_id,
                agent("reviewer-2"),
                AssignmentKind::Review {
                    target_assignment_id: review.clone(),
                },
                Timestamp(4),
            )
            .unwrap_err(),
            LiveSessionError::TargetAssignmentKindMismatch {
                target_assignment_id: review,
                kind: AssignmentKind::Review {
                    target_assignment_id: AssignmentId::from("assignment-1")
                }
            }
        );
    }

    #[test]
    fn submit_output_requires_assigned_agent() {
        let mut core = LiveSessionCore::new();
        let session_id = core.create_session(task("task-1"), Timestamp(1));
        let assignment_id = core
            .assign(
                task("task-1"),
                &session_id,
                agent("executor"),
                AssignmentKind::Execute,
                Timestamp(2),
            )
            .unwrap();

        assert_eq!(
            core.submit_output(
                &assignment_id,
                agent("other"),
                output("hash-1"),
                Timestamp(3),
            )
            .unwrap_err(),
            LiveSessionError::AgentMismatch {
                assignment_id: assignment_id.clone(),
                expected: agent("executor"),
                actual: agent("other")
            }
        );

        core.submit_output(
            &assignment_id,
            agent("executor"),
            output("hash-1"),
            Timestamp(3),
        )
        .unwrap();
        let assignment = core.get_assignment(&assignment_id).unwrap();
        assert_eq!(assignment.status, AssignmentStatus::Submitted);
        assert_eq!(assignment.output_hash, Some(output("hash-1")));
    }

    #[test]
    fn submit_artifact_validates_and_stores_manifest_hash() {
        let mut core = LiveSessionCore::new();
        let session_id = core.create_session(task("task-1"), Timestamp(1));
        let assignment_id = core
            .assign(
                task("task-1"),
                &session_id,
                agent("executor"),
                AssignmentKind::Execute,
                Timestamp(2),
            )
            .unwrap();
        let manifest = text_manifest(
            "artifact-1",
            task("task-1"),
            assignment_id.clone(),
            agent("executor"),
            Timestamp(3),
        );
        let manifest_hash = manifest.manifest_hash.clone().unwrap();

        core.submit_artifact(&assignment_id, agent("executor"), manifest, Timestamp(4))
            .unwrap();

        let assignment = core.get_assignment(&assignment_id).unwrap();
        assert_eq!(assignment.status, AssignmentStatus::Submitted);
        assert_eq!(
            assignment.output_hash,
            Some(OutputHash::from(manifest_hash.to_string()))
        );
    }

    #[test]
    fn submit_artifact_rejects_assignment_mismatch() {
        let mut core = LiveSessionCore::new();
        let session_id = core.create_session(task("task-1"), Timestamp(1));
        let assignment_id = core
            .assign(
                task("task-1"),
                &session_id,
                agent("executor"),
                AssignmentKind::Execute,
                Timestamp(2),
            )
            .unwrap();
        let manifest = text_manifest(
            "artifact-1",
            task("task-1"),
            AssignmentId::from("other-assignment"),
            agent("executor"),
            Timestamp(3),
        );

        assert_eq!(
            core.submit_artifact(&assignment_id, agent("executor"), manifest, Timestamp(4),)
                .unwrap_err(),
            LiveSessionError::InvalidArtifact(ArtifactError::AssignmentMismatch {
                expected: assignment_id,
                actual: AssignmentId::from("other-assignment")
            })
        );
    }

    #[test]
    fn submit_artifact_rejects_producer_mismatch() {
        let mut core = LiveSessionCore::new();
        let session_id = core.create_session(task("task-1"), Timestamp(1));
        let assignment_id = core
            .assign(
                task("task-1"),
                &session_id,
                agent("executor"),
                AssignmentKind::Execute,
                Timestamp(2),
            )
            .unwrap();
        let manifest = text_manifest(
            "artifact-1",
            task("task-1"),
            assignment_id.clone(),
            agent("executor"),
            Timestamp(3),
        );

        assert_eq!(
            core.submit_artifact(&assignment_id, agent("other"), manifest, Timestamp(4))
                .unwrap_err(),
            LiveSessionError::InvalidArtifact(ArtifactError::ProducerMismatch {
                expected: agent("other"),
                actual: agent("executor")
            })
        );
    }

    #[test]
    fn submit_artifact_rejects_invalid_media_profile() {
        let mut core = LiveSessionCore::new();
        let session_id = core.create_session(task("task-1"), Timestamp(1));
        let assignment_id = core
            .assign(
                task("task-1"),
                &session_id,
                agent("executor"),
                AssignmentKind::Execute,
                Timestamp(2),
            )
            .unwrap();
        let file = ArtifactFile::new(
            "https://agent.example/blob.bin",
            hash(1),
            "application/octet-stream",
            "unknown.profile.v1",
            120,
        );
        let manifest = ArtifactManifest::new(
            "artifact-1",
            task("task-1"),
            assignment_id.clone(),
            agent("executor"),
            ArtifactKind::Single,
            vec![file],
            Timestamp(3),
        )
        .with_manifest_hash(hash(2));

        assert_eq!(
            core.submit_artifact(&assignment_id, agent("executor"), manifest, Timestamp(4),)
                .unwrap_err(),
            LiveSessionError::InvalidArtifact(ArtifactError::UnsupportedMediaProfile {
                index: 0,
                profile: MediaProfileId::from("unknown.profile.v1")
            })
        );
    }

    #[test]
    fn approved_and_rejected_require_submitted_assignment() {
        let mut core = LiveSessionCore::new();
        let session_id = core.create_session(task("task-1"), Timestamp(1));
        let assignment_id = core
            .assign(
                task("task-1"),
                &session_id,
                agent("executor"),
                AssignmentKind::Execute,
                Timestamp(2),
            )
            .unwrap();

        assert_eq!(
            core.mark_approved(&assignment_id, Timestamp(3))
                .unwrap_err(),
            LiveSessionError::AssignmentNotSubmitted {
                assignment_id: assignment_id.clone(),
                status: AssignmentStatus::Assigned
            }
        );

        core.submit_output(
            &assignment_id,
            agent("executor"),
            output("hash-1"),
            Timestamp(3),
        )
        .unwrap();
        core.mark_approved(&assignment_id, Timestamp(4)).unwrap();
        assert_eq!(
            core.get_assignment(&assignment_id).unwrap().status,
            AssignmentStatus::Approved
        );
    }

    #[test]
    fn cancel_assignment_updates_status() {
        let mut core = LiveSessionCore::new();
        let session_id = core.create_session(task("task-1"), Timestamp(1));
        let assignment_id = core
            .assign(
                task("task-1"),
                &session_id,
                agent("executor"),
                AssignmentKind::Execute,
                Timestamp(2),
            )
            .unwrap();

        core.cancel_assignment(&assignment_id, Timestamp(3))
            .unwrap();

        assert_eq!(
            core.get_assignment(&assignment_id).unwrap().status,
            AssignmentStatus::Cancelled
        );
    }

    #[test]
    fn close_session_blocks_new_assignments() {
        let mut core = LiveSessionCore::new();
        let session_id = core.create_session(task("task-1"), Timestamp(1));

        core.close_session(&session_id, Timestamp(2)).unwrap();

        assert_eq!(
            core.assign(
                task("task-1"),
                &session_id,
                agent("executor"),
                AssignmentKind::Execute,
                Timestamp(3),
            )
            .unwrap_err(),
            LiveSessionError::SessionNotRunning {
                session_id,
                status: LiveSessionStatus::Closed
            }
        );
    }
}
