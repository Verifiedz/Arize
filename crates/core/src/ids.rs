//! Identifier newtypes. Plain strings on the wire, distinct types in code.

use std::fmt;

use serde::{Deserialize, Serialize};
use ulid::Ulid;

macro_rules! string_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(s: impl Into<String>) -> Self {
                Self(s.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self::new(s)
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self::new(s)
            }
        }
    };
}

string_id!(
    /// A module's identity, e.g. `records`. Also the prefix of every op and topic it owns.
    ModuleId
);
string_id!(
    /// An execution lane, e.g. `fetchers` (§6.1).
    LaneId
);
string_id!(
    /// A scheduler trigger.
    TriggerId
);

impl LaneId {
    /// The shared lane for anything that declares none.
    pub fn default_lane() -> Self {
        Self::new("default")
    }
}

/// A queued unit of work. ULID: sortable, generated on enqueue.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TaskId(Ulid);

impl TaskId {
    pub fn new() -> Self {
        Self(Ulid::new())
    }
}

impl Default for TaskId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::str::FromStr for TaskId {
    type Err = crate::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ulid::from_string(s).map(Self).map_err(|_| crate::Error::invalid_params(format!("'{s}' is not a task id")))
    }
}

/// True for `[a-z][a-z0-9_-]*`: safe as a module id, a data namespace and a path segment.
pub fn is_valid_name(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}
