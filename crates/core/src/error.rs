//! One error type for modules, the daemon and the wire. Codes are the stable strings from
//! `docs/protocol.md`; clients branch on `code`, never on `message`.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    UnsupportedVersion,
    BadRequest,
    UnknownOp,
    InvalidParams,
    NotFound,
    Conflict,
    ConfirmationRequired,
    WorkspaceDirty,
    LaneUnknown,
    NotCancellable,
    ModuleError,
    Unavailable,
    Internal,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnsupportedVersion => "unsupported_version",
            Self::BadRequest => "bad_request",
            Self::UnknownOp => "unknown_op",
            Self::InvalidParams => "invalid_params",
            Self::NotFound => "not_found",
            Self::Conflict => "conflict",
            Self::ConfirmationRequired => "confirmation_required",
            Self::WorkspaceDirty => "workspace_dirty",
            Self::LaneUnknown => "lane_unknown",
            Self::NotCancellable => "not_cancellable",
            Self::ModuleError => "module_error",
            Self::Unavailable => "unavailable",
            Self::Internal => "internal",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, thiserror::Error, Serialize, Deserialize)]
#[error("{code}: {message}")]
pub struct Error {
    pub code: ErrorCode,
    pub message: String,
    /// Optional structured context. Clients must tolerate it being absent.
    #[serde(default)]
    pub detail: Option<Value>,
}

pub type Result<T> = std::result::Result<T, Error>;

macro_rules! ctor {
    ($($fn:ident => $code:ident),* $(,)?) => {
        $(pub fn $fn(message: impl Into<String>) -> Self {
            Self::new(ErrorCode::$code, message)
        })*
    };
}

impl Error {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), detail: None }
    }

    pub fn with_detail(mut self, detail: Value) -> Self {
        self.detail = Some(detail);
        self
    }

    ctor! {
        bad_request => BadRequest,
        unknown_op => UnknownOp,
        invalid_params => InvalidParams,
        not_found => NotFound,
        conflict => Conflict,
        lane_unknown => LaneUnknown,
        not_cancellable => NotCancellable,
        module_error => ModuleError,
        unavailable => Unavailable,
        internal => Internal,
    }

    /// Whether `ctx.retry_with_backoff` should try again. Only `unavailable`: the network or
    /// a script may recover. Retrying `conflict` or `invalid_params` unchanged cannot help.
    pub fn is_retryable(&self) -> bool {
        self.code == ErrorCode::Unavailable
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::internal(format!("io error: {e}"))
    }
}
