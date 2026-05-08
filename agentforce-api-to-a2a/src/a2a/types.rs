//! A2A 0.3.0 protocol data objects.
//!
//! We model only the pieces the policy actually needs to produce or
//! consume - the spec contains a much larger surface (push-notification
//! configs, file/data parts, streaming events, etc.) that we don't yet
//! support. Optional fields are skipped on serialize when empty so the
//! emitted JSON stays tight.

use serde::{Deserialize, Serialize};

/// A2A `TaskState` enum (spec §6.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskState {
    Submitted,
    Working,
    InputRequired,
    Completed,
    Canceled,
    Failed,
    Rejected,
    AuthRequired,
    Unknown,
}

impl TaskState {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Canceled | Self::Failed | Self::Rejected
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStatus {
    pub state: TaskState,
    /// ISO-8601 timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    /// Optional message accompanying the status (e.g. agent thinking).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<Message>,
}

/// A2A `Task` (spec §6.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    #[serde(rename = "contextId")]
    pub context_id: String,
    pub kind: String, // always "task"
    pub status: TaskStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<Message>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<Artifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

impl Task {
    pub fn new(id: String, status: TaskStatus) -> Self {
        Self {
            context_id: id.clone(),
            id,
            kind: "task".into(),
            status,
            history: Vec::new(),
            artifacts: Vec::new(),
            metadata: None,
        }
    }
}

/// A2A `Message` (spec §6.4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// "message" literal per spec.
    pub kind: String,
    #[serde(rename = "messageId")]
    pub message_id: String,
    /// "user" or "agent".
    pub role: String,
    pub parts: Vec<Part>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "taskId")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "contextId")]
    pub context_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

impl Message {
    pub fn new(role: &str, message_id: String, parts: Vec<Part>) -> Self {
        Self {
            kind: "message".into(),
            message_id,
            role: role.into(),
            parts,
            task_id: None,
            context_id: None,
            metadata: None,
        }
    }
}

/// A2A `Part` discriminated union (spec §6.5). v1 only handles TextPart.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Part {
    Text {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        metadata: Option<serde_json::Value>,
    },
    /// Catch-all for parts the policy can pass through but doesn't
    /// generate (file/data). Stored as raw JSON to round-trip cleanly.
    #[serde(other)]
    Unknown,
}

impl Part {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text {
            text: text.into(),
            metadata: None,
        }
    }
}

/// A2A `Artifact` (spec §6.7).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    #[serde(rename = "artifactId")]
    pub artifact_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub parts: Vec<Part>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// A2A `MessageSendParams` (spec §7.1.1).
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct MessageSendParams {
    pub message: Message,
    #[serde(default)]
    pub configuration: Option<MessageSendConfiguration>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct MessageSendConfiguration {
    #[serde(default)]
    #[serde(rename = "acceptedOutputModes")]
    pub accepted_output_modes: Option<Vec<String>>,
    #[serde(default)]
    #[serde(rename = "historyLength")]
    pub history_length: Option<u32>,
    #[serde(default)]
    pub blocking: Option<bool>,
}

/// A2A `TaskQueryParams` (spec §7.3.1).
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct TaskQueryParams {
    pub id: String,
    #[serde(default)]
    #[serde(rename = "historyLength")]
    pub history_length: Option<u32>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// A2A `TaskIdParams` (spec §7.4.1).
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct TaskIdParams {
    pub id: String,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// Helper to extract concatenated text from the parts of a Message. v1
/// only supports text input; non-text parts are silently dropped here and
/// the caller decides whether to reject the message upstream.
pub fn concat_text(parts: &[Part]) -> String {
    let mut out = String::new();
    for p in parts {
        if let Part::Text { text, .. } = p {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(text);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_state_terminality() {
        assert!(TaskState::Completed.is_terminal());
        assert!(TaskState::Canceled.is_terminal());
        assert!(TaskState::Failed.is_terminal());
        assert!(TaskState::Rejected.is_terminal());
        assert!(!TaskState::Working.is_terminal());
        assert!(!TaskState::Submitted.is_terminal());
        assert!(!TaskState::InputRequired.is_terminal());
    }

    #[test]
    fn task_state_serializes_kebab_case() {
        let v = serde_json::to_value(TaskState::InputRequired).unwrap();
        assert_eq!(v, serde_json::json!("input-required"));
        let v = serde_json::to_value(TaskState::AuthRequired).unwrap();
        assert_eq!(v, serde_json::json!("auth-required"));
        let v = serde_json::to_value(TaskState::Completed).unwrap();
        assert_eq!(v, serde_json::json!("completed"));
    }

    #[test]
    fn part_serializes_with_kind_discriminator() {
        let p = Part::text("hi");
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["kind"], "text");
        assert_eq!(v["text"], "hi");
    }

    #[test]
    fn message_round_trips() {
        let m = Message::new("user", "m-1".into(), vec![Part::text("hi")]);
        let bytes = serde_json::to_vec(&m).unwrap();
        let back: Message = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back.role, "user");
        assert_eq!(back.message_id, "m-1");
        assert_eq!(back.parts.len(), 1);
        if let Part::Text { text, .. } = &back.parts[0] {
            assert_eq!(text, "hi");
        } else {
            panic!("expected text part");
        }
    }

    #[test]
    fn task_omits_empty_arrays() {
        let t = Task::new(
            "t1".into(),
            TaskStatus {
                state: TaskState::Completed,
                timestamp: None,
                message: None,
            },
        );
        let v = serde_json::to_value(&t).unwrap();
        assert!(v.get("history").is_none());
        assert!(v.get("artifacts").is_none());
        assert_eq!(v["kind"], "task");
        assert_eq!(v["contextId"], "t1");
    }

    #[test]
    fn concat_text_joins_text_parts() {
        let parts = vec![Part::text("hello"), Part::text("world")];
        assert_eq!(concat_text(&parts), "hello\nworld");
    }

    #[test]
    fn concat_text_skips_unknown_parts() {
        let parts = vec![Part::text("hi"), Part::Unknown];
        assert_eq!(concat_text(&parts), "hi");
    }

    #[test]
    fn message_send_params_parses() {
        let body = serde_json::json!({
            "message": {
                "kind": "message",
                "messageId": "m1",
                "role": "user",
                "parts": [{"kind":"text","text":"hi"}]
            },
            "configuration": { "blocking": true }
        });
        let p: MessageSendParams = serde_json::from_value(body).unwrap();
        assert_eq!(p.message.role, "user");
        assert_eq!(p.configuration.unwrap().blocking, Some(true));
    }
}
