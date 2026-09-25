use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::{LaneId, ModuleId, TaskId, TriggerId};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    /// Fixed at creation, never renegotiated.
    pub lane: LaneId,
    pub op: String,
    pub params: Value,
    pub priority: Priority,
    pub origin: Origin,
    pub fallback: Option<Fallback>,
    pub enqueued_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    /// Scheduler-fired. The default source of truth for ordering.
    Scheduled,
    /// Manual or module-originated. Joins the lane in normal order.
    Normal,
    /// Explicitly promoted by the user, after confirmation. Audited.
    Overridden { by: String, at: DateTime<Utc> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Origin {
    Scheduler { trigger_id: TriggerId },
    User,
    Module { id: ModuleId },
}

/// Set at trigger-creation time, never invented at failure time (§11.3, ADR 0005).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fallback {
    /// `fallback_op` must be a `notify.*` op (e.g. selecting a sink or template). Any other
    /// namespace is rejected at `scheduler.add` time, not at failure time. A `notify.*` op
    /// MUST NOT itself declare a `fallback` — no nested fallbacks.
    Notify { fallback_op: String, params: Value, priority: NotifyPriority },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotifyPriority {
    Low,
    Normal,
    High,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskOutcome {
    Succeeded,
    Failed { error: String, retryable: bool },
    Cancelled,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn fallback_wire_shape() {
        let f = Fallback::Notify {
            fallback_op: "notify.send".into(),
            params: json!({"message": "didn't run"}),
            priority: NotifyPriority::High,
        };
        let v = serde_json::to_value(&f).unwrap();
        assert_eq!(
            v,
            json!({"notify": {"fallback_op": "notify.send", "params": {"message": "didn't run"}, "priority": "high"}})
        );
        assert_eq!(serde_json::from_value::<Fallback>(v).unwrap(), f);
    }

    #[test]
    fn the_old_notify_only_shape_is_gone() {
        let old = json!({"notify_only": {"message": "didn't run", "priority": "high"}});
        assert!(serde_json::from_value::<Fallback>(old).is_err());
    }
}
