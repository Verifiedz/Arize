//! Enqueue handle. Available to any module, not just the scheduler (§5).

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::error::Result;
use crate::ids::{LaneId, ModuleId, TaskId};
use crate::task::{Fallback, Origin, Priority};

#[derive(Clone, Debug, PartialEq)]
pub struct EnqueueRequest {
    pub op: String,
    pub params: Value,
    /// `None` = the lane the op's `CommandSpec` declares, else `default`.
    pub lane: Option<LaneId>,
    pub priority: Priority,
    pub origin: Origin,
    pub fallback: Option<Fallback>,
}

/// What the caller learns immediately. The result arrives later, as events.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskHandle {
    pub task_id: TaskId,
    pub lane: LaneId,
    /// Tasks ahead of this one in the lane's waiting list; 0 = next up.
    pub position: usize,
}

#[async_trait]
pub trait TaskSubmitter: Send + Sync {
    async fn submit(&self, req: EnqueueRequest) -> Result<TaskHandle>;
}

#[derive(Clone)]
pub struct QueueHandle {
    module: ModuleId,
    submitter: Arc<dyn TaskSubmitter>,
}

impl QueueHandle {
    pub fn new(module: ModuleId, submitter: Arc<dyn TaskSubmitter>) -> Self {
        Self { module, submitter }
    }

    /// Enqueue at `Normal` priority. Origin is always this module: a module cannot
    /// impersonate the scheduler or the user, and cannot request `Overridden`.
    pub async fn enqueue(&self, op: &str, params: Value) -> Result<TaskHandle> {
        self.enqueue_in(op, params, None).await
    }

    pub async fn enqueue_in(&self, op: &str, params: Value, lane: Option<LaneId>) -> Result<TaskHandle> {
        self.submitter
            .submit(EnqueueRequest {
                op: op.to_owned(),
                params,
                lane,
                priority: Priority::Normal,
                origin: Origin::Module { id: self.module.clone() },
                fallback: None,
            })
            .await
    }
}
