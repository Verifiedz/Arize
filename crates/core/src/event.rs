//! The one event type: bus, IPC stream and JSONL log all carry exactly this.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ulid::Ulid;

use crate::ids::ModuleId;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// Sortable, generated on emit.
    pub id: Ulid,
    pub at: DateTime<Utc>,
    pub source: ModuleId,
    /// `<module>.<entity>.<verb>`, verb in past tense.
    pub topic: String,
    pub payload: Value,
}

impl Event {
    /// The id's timestamp is `at`, so ids sort with the clock.
    pub fn new(source: ModuleId, topic: impl Into<String>, payload: Value, at: DateTime<Utc>) -> Self {
        Self { id: Ulid::from_datetime(at.into()), at, source, topic: topic.into(), payload }
    }
}
