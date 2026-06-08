use std::collections::{HashMap, HashSet};

use crate::heartbeat::AgentId;
use crate::types::{TaskId, Timestamp};

use super::types::{Task, TaskError, TaskStatus};

#[derive(Debug, Default)]
pub struct TaskCore {
    tasks: HashMap<TaskId, Task>,
    active_tasks_by_agent: HashMap<AgentId, HashSet<TaskId>>,
    task_history_by_agent: HashMap<AgentId, HashSet<TaskId>>,
    tasks_by_publisher: HashMap<AgentId, HashSet<TaskId>>,
    next_task: u64,
}

impl TaskCore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create(
        &mut self,
        publisher: AgentId,
        created_at: Timestamp,
    ) -> Result<TaskId, TaskError> {
        let task_id = self.next_task_id();
        self.tasks.insert(
            task_id.clone(),
            Task {
                task_id: task_id.clone(),
                publisher: publisher.clone(),
                active_participants: HashSet::new(),
                participant_history: HashSet::new(),
                status: TaskStatus::Active,
                created_at,
                updated_at: created_at,
            },
        );
        self.tasks_by_publisher
            .entry(publisher)
            .or_default()
            .insert(task_id.clone());

        Ok(task_id)
    }

    pub fn add_participant(
        &mut self,
        task_id: &TaskId,
        agent_id: AgentId,
        updated_at: Timestamp,
    ) -> Result<(), TaskError> {
        self.validate_active_at(task_id, updated_at)?;

        let task = self
            .tasks
            .get_mut(task_id)
            .ok_or_else(|| TaskError::TaskNotFound(task_id.clone()))?;
        task.active_participants.insert(agent_id.clone());
        task.participant_history.insert(agent_id.clone());
        task.updated_at = updated_at;

        self.active_tasks_by_agent
            .entry(agent_id.clone())
            .or_default()
            .insert(task_id.clone());
        self.task_history_by_agent
            .entry(agent_id)
            .or_default()
            .insert(task_id.clone());

        Ok(())
    }

    pub fn remove_participant(
        &mut self,
        task_id: &TaskId,
        agent_id: &AgentId,
        updated_at: Timestamp,
    ) -> Result<bool, TaskError> {
        self.validate_active_at(task_id, updated_at)?;

        let removed = self
            .tasks
            .get_mut(task_id)
            .ok_or_else(|| TaskError::TaskNotFound(task_id.clone()))?
            .active_participants
            .remove(agent_id);
        if !removed {
            return Ok(false);
        }

        if let Some(task_ids) = self.active_tasks_by_agent.get_mut(agent_id) {
            task_ids.remove(task_id);
            if task_ids.is_empty() {
                self.active_tasks_by_agent.remove(agent_id);
            }
        }

        if let Some(task) = self.tasks.get_mut(task_id) {
            task.updated_at = updated_at;
        }

        Ok(true)
    }

    pub fn complete(&mut self, task_id: &TaskId, completed_at: Timestamp) -> Result<(), TaskError> {
        self.close(task_id, TaskStatus::Completed, completed_at)
    }

    pub fn cancel(&mut self, task_id: &TaskId, cancelled_at: Timestamp) -> Result<(), TaskError> {
        self.close(task_id, TaskStatus::Cancelled, cancelled_at)
    }

    pub fn get(&self, task_id: &TaskId) -> Option<&Task> {
        self.tasks.get(task_id)
    }

    pub fn active_tasks_by_agent(&self, agent_id: &AgentId) -> Vec<Task> {
        self.tasks_from_index(self.active_tasks_by_agent.get(agent_id))
    }

    pub fn task_history_by_agent(&self, agent_id: &AgentId) -> Vec<Task> {
        self.tasks_from_index(self.task_history_by_agent.get(agent_id))
    }

    pub fn tasks_by_publisher(&self, agent_id: &AgentId) -> Vec<Task> {
        self.tasks_from_index(self.tasks_by_publisher.get(agent_id))
    }

    pub fn task_count(&self) -> usize {
        self.tasks.len()
    }

    fn close(
        &mut self,
        task_id: &TaskId,
        status: TaskStatus,
        updated_at: Timestamp,
    ) -> Result<(), TaskError> {
        self.validate_active_at(task_id, updated_at)?;

        let participants = self
            .tasks
            .get(task_id)
            .ok_or_else(|| TaskError::TaskNotFound(task_id.clone()))?
            .active_participants
            .clone();
        for participant in participants {
            if let Some(task_ids) = self.active_tasks_by_agent.get_mut(&participant) {
                task_ids.remove(task_id);
                if task_ids.is_empty() {
                    self.active_tasks_by_agent.remove(&participant);
                }
            }
        }

        let task = self
            .tasks
            .get_mut(task_id)
            .ok_or_else(|| TaskError::TaskNotFound(task_id.clone()))?;
        task.active_participants.clear();
        task.status = status;
        task.updated_at = updated_at;

        Ok(())
    }

    fn validate_active_at(&self, task_id: &TaskId, attempted: Timestamp) -> Result<(), TaskError> {
        let task = self
            .tasks
            .get(task_id)
            .ok_or_else(|| TaskError::TaskNotFound(task_id.clone()))?;
        if task.status != TaskStatus::Active {
            return Err(TaskError::TaskNotActive {
                task_id: task_id.clone(),
                status: task.status,
            });
        }
        if attempted < task.updated_at {
            return Err(TaskError::TimestampWentBackwards {
                task_id: task_id.clone(),
                current: task.updated_at,
                attempted,
            });
        }

        Ok(())
    }

    fn tasks_from_index(&self, task_ids: Option<&HashSet<TaskId>>) -> Vec<Task> {
        let Some(task_ids) = task_ids else {
            return Vec::new();
        };

        let mut task_ids = task_ids.iter().cloned().collect::<Vec<_>>();
        task_ids.sort();
        task_ids
            .into_iter()
            .filter_map(|task_id| self.tasks.get(&task_id).cloned())
            .collect()
    }

    fn next_task_id(&mut self) -> TaskId {
        self.next_task += 1;
        TaskId::new(format!("task-{}", self.next_task))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(id: &str) -> AgentId {
        AgentId::from(id)
    }

    fn create_task(core: &mut TaskCore, publisher: &str) -> TaskId {
        core.create(agent(publisher), Timestamp(1)).unwrap()
    }

    #[test]
    fn create_stores_active_task_and_publisher_index() {
        let mut core = TaskCore::new();

        let task_id = create_task(&mut core, "publisher");
        let task = core.get(&task_id).unwrap();

        assert_eq!(core.task_count(), 1);
        assert_eq!(task.status, TaskStatus::Active);
        assert_eq!(task.publisher, agent("publisher"));
        assert_eq!(task.created_at, Timestamp(1));
        assert_eq!(task.updated_at, Timestamp(1));
        assert_eq!(core.tasks_by_publisher(&agent("publisher")).len(), 1);
    }

    #[test]
    fn add_participant_updates_active_and_history_indexes() {
        let mut core = TaskCore::new();
        let task_id = create_task(&mut core, "publisher");

        core.add_participant(&task_id, agent("worker"), Timestamp(2))
            .unwrap();
        core.add_participant(&task_id, agent("worker"), Timestamp(2))
            .unwrap();

        let task = core.get(&task_id).unwrap();
        assert_eq!(task.active_participants.len(), 1);
        assert_eq!(task.participant_history.len(), 1);
        assert_eq!(task.updated_at, Timestamp(2));
        assert_eq!(core.active_tasks_by_agent(&agent("worker")).len(), 1);
        assert_eq!(core.task_history_by_agent(&agent("worker")).len(), 1);
    }

    #[test]
    fn remove_participant_keeps_history() {
        let mut core = TaskCore::new();
        let task_id = create_task(&mut core, "publisher");
        core.add_participant(&task_id, agent("worker"), Timestamp(2))
            .unwrap();

        assert!(
            core.remove_participant(&task_id, &agent("worker"), Timestamp(3))
                .unwrap()
        );

        let task = core.get(&task_id).unwrap();
        assert!(!task.active_participants.contains(&agent("worker")));
        assert!(task.participant_history.contains(&agent("worker")));
        assert_eq!(core.active_tasks_by_agent(&agent("worker")).len(), 0);
        assert_eq!(core.task_history_by_agent(&agent("worker")).len(), 1);
    }

    #[test]
    fn remove_missing_active_participant_returns_false() {
        let mut core = TaskCore::new();
        let task_id = create_task(&mut core, "publisher");

        assert!(
            !core
                .remove_participant(&task_id, &agent("worker"), Timestamp(2))
                .unwrap()
        );
        assert_eq!(core.get(&task_id).unwrap().updated_at, Timestamp(1));
    }

    #[test]
    fn complete_makes_task_read_only_and_clears_active_index() {
        let mut core = TaskCore::new();
        let task_id = create_task(&mut core, "publisher");
        core.add_participant(&task_id, agent("worker"), Timestamp(2))
            .unwrap();

        core.complete(&task_id, Timestamp(3)).unwrap();

        let task = core.get(&task_id).unwrap();
        assert_eq!(task.status, TaskStatus::Completed);
        assert!(task.active_participants.is_empty());
        assert_eq!(task.updated_at, Timestamp(3));
        assert_eq!(core.active_tasks_by_agent(&agent("worker")).len(), 0);
        assert_eq!(core.task_history_by_agent(&agent("worker")).len(), 1);
        assert_eq!(
            core.add_participant(&task_id, agent("other"), Timestamp(4))
                .unwrap_err(),
            TaskError::TaskNotActive {
                task_id,
                status: TaskStatus::Completed
            }
        );
    }

    #[test]
    fn cancel_makes_task_read_only_and_clears_active_index() {
        let mut core = TaskCore::new();
        let task_id = create_task(&mut core, "publisher");
        core.add_participant(&task_id, agent("worker"), Timestamp(2))
            .unwrap();

        core.cancel(&task_id, Timestamp(3)).unwrap();

        assert_eq!(core.get(&task_id).unwrap().status, TaskStatus::Cancelled);
        assert_eq!(core.active_tasks_by_agent(&agent("worker")).len(), 0);
        assert_eq!(
            core.remove_participant(&task_id, &agent("worker"), Timestamp(4))
                .unwrap_err(),
            TaskError::TaskNotActive {
                task_id,
                status: TaskStatus::Cancelled
            }
        );
    }

    #[test]
    fn unknown_task_returns_not_found() {
        let mut core = TaskCore::new();
        let task_id = TaskId::from("missing");

        assert_eq!(
            core.add_participant(&task_id, agent("worker"), Timestamp(1))
                .unwrap_err(),
            TaskError::TaskNotFound(task_id)
        );
    }

    #[test]
    fn timestamp_cannot_go_backwards() {
        let mut core = TaskCore::new();
        let task_id = create_task(&mut core, "publisher");
        core.add_participant(&task_id, agent("worker"), Timestamp(3))
            .unwrap();

        assert_eq!(
            core.remove_participant(&task_id, &agent("worker"), Timestamp(2))
                .unwrap_err(),
            TaskError::TimestampWentBackwards {
                task_id,
                current: Timestamp(3),
                attempted: Timestamp(2)
            }
        );
    }

    #[test]
    fn query_results_are_sorted_by_task_id() {
        let mut core = TaskCore::new();
        let first = create_task(&mut core, "publisher");
        let second = create_task(&mut core, "publisher");
        core.add_participant(&second, agent("worker"), Timestamp(2))
            .unwrap();
        core.add_participant(&first, agent("worker"), Timestamp(2))
            .unwrap();

        let tasks = core.active_tasks_by_agent(&agent("worker"));

        assert_eq!(
            tasks
                .iter()
                .map(|task| task.task_id.clone())
                .collect::<Vec<_>>(),
            vec![first, second]
        );
    }
}
