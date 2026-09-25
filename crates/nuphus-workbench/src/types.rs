use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApiError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl ApiError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }

    pub fn details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}

impl From<rusqlite::Error> for ApiError {
    fn from(error: rusqlite::Error) -> Self {
        Self::new("storage_error", error.to_string())
    }
}

impl From<std::io::Error> for ApiError {
    fn from(error: std::io::Error) -> Self {
        Self::new("storage_error", error.to_string())
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(error: serde_json::Error) -> Self {
        Self::new("invalid_document", error.to_string())
    }
}

pub type Result<T> = std::result::Result<T, ApiError>;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuthoringMode {
    Internal,
    External,
}

impl AuthoringMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Internal => "internal",
            Self::External => "external",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Project {
    pub project_id: String,
    pub name: String,
    pub directory: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Draft {
    pub project_id: String,
    pub workflow_id: String,
    pub revision: u64,
    pub authoring_mode: AuthoringMode,
    pub document: Value,
    pub layout: Value,
    #[serde(default)]
    pub layout_revision: u64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Version {
    pub version_id: String,
    pub workflow_id: String,
    pub revision: u64,
    pub document: Value,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Starting,
    Running,
    Paused,
    AwaitingHuman,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::AwaitingHuman => "awaiting_human",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }

    pub fn terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }

    pub fn allows(self, next: Self) -> bool {
        self == next
            || (!self.terminal()
                && match next {
                    Self::Starting => false,
                    Self::Running => {
                        matches!(self, Self::Starting | Self::Paused | Self::AwaitingHuman)
                    }
                    Self::Paused | Self::AwaitingHuman => self == Self::Running,
                    Self::Completed => {
                        matches!(self, Self::Running | Self::Paused | Self::AwaitingHuman)
                    }
                    Self::Failed | Self::Cancelled | Self::Interrupted => true,
                })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Run {
    pub run_id: String,
    pub project_id: String,
    pub workflow_id: String,
    pub version_id: String,
    pub status: RunStatus,
    pub inputs: Value,
    pub snapshots: Value,
    pub created_at: i64,
    pub updated_at: i64,
    pub result: Option<Value>,
    #[serde(default)]
    pub pending_request: Option<HumanRequest>,
    #[serde(default)]
    pub debug: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HumanRequest {
    pub request_id: String,
    pub step_id: String,
    pub prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Event {
    pub cursor: u64,
    pub project_id: String,
    pub run_id: Option<String>,
    pub workflow_id: String,
    pub kind: String,
    pub data: Value,
    pub created_at: i64,
}
