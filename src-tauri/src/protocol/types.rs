//! Protocol type definitions for Claude CLI control protocol
//!
//! This module defines the message types used for communication between
//! the CLI and the Claude SDK.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Permission mode for agent execution
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    /// All operations allowed without approval
    Allow,
    /// All operations require approval
    Deny,
    /// Operations follow configured rules
    Auto,
}

impl Default for PermissionMode {
    fn default() -> Self {
        Self::Auto
    }
}

impl PermissionMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            PermissionMode::Allow => "allow",
            PermissionMode::Deny => "deny",
            PermissionMode::Auto => "auto",
        }
    }
}

/// Type of control request from SDK to CLI
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ControlRequestType {
    /// Request to use a specific tool
    CanUseTool {
        tool_name: String,
        tool_input: serde_json::Value,
        tool_use_id: String,
    },
    /// Hook callback (e.g., for git hooks)
    HookCallback {
        hook_name: String,
        hook_input: serde_json::Value,
    },
    /// Request for permission mode change
    PermissionModeChange {
        requested_mode: PermissionMode,
    },
    /// Notification that agent is waiting for approval
    WaitingForApproval {
        tool_name: String,
        tool_use_id: String,
    },
}

/// Control request from SDK to CLI
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SDKControlRequest {
    /// Unique request ID
    pub request_id: String,
    /// Type of control request
    #[serde(flatten)]
    pub request_type: ControlRequestType,
}

impl SDKControlRequest {
    pub fn new(request_type: ControlRequestType) -> Self {
        Self {
            request_id: uuid::Uuid::new_v4().to_string(),
            request_type,
        }
    }
}

/// Response from CLI to SDK for a control request
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ControlResponseType {
    /// Allow the operation
    Allow {
        #[serde(skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
    },
    /// Deny the operation
    Deny {
        #[serde(skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// Request approval from user (for async approval flow)
    Pending {
        request_id: String,
    },
    /// Acknowledgment (for notifications)
    Ack {
        #[serde(skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
    },
}

/// Message sent from CLI to SDK
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum CLIMessage {
    /// Response to a control request
    ControlResponse {
        request_id: String,
        response: ControlResponseType,
    },
    /// Notification that permission mode changed
    PermissionModeChanged {
        mode: PermissionMode,
    },
    /// Request to interrupt the agent
    Interrupt {
        reason: Option<String>,
    },
    /// Heartbeat/keep-alive message
    Heartbeat {
        timestamp: i64,
    },
    /// Initialize the session
    Initialize {
        session_id: String,
        permission_mode: PermissionMode,
        #[serde(skip_serializing_if = "Option::is_none")]
        allowed_tools: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        denied_tools: Option<Vec<String>>,
    },
    /// Request to check if a tool is allowed
    CanUseToolQuery {
        tool_name: String,
    },
}

impl CLIMessage {
    /// Serialize message to JSON string
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Deserialize message from JSON string
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }
}

/// Tool permission rule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolPermission {
    /// Tool name (can use * for wildcard)
    pub tool_name: String,
    /// Whether this tool is allowed
    pub allowed: bool,
    /// Optional description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Agent permission configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPermissions {
    /// Allow file read operations
    pub enable_file_read: bool,
    /// Allow file write operations
    pub enable_file_write: bool,
    /// Allow network access
    pub enable_network: bool,
    /// Allowed tool list (if empty, all tools are allowed)
    #[serde(default)]
    pub tools: Vec<String>,
    /// Denied tool list (takes precedence over allowed)
    #[serde(default)]
    pub denied_tools: Vec<String>,
    /// Custom permission rules
    #[serde(default)]
    pub tool_permissions: Vec<ToolPermission>,
}

impl Default for AgentPermissions {
    fn default() -> Self {
        Self {
            enable_file_read: true,
            enable_file_write: true,
            enable_network: true,
            tools: Vec::new(),
            denied_tools: Vec::new(),
            tool_permissions: Vec::new(),
        }
    }
}

/// Approval status for a tool request
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalStatus {
    /// Operation is approved
    Approved,
    /// Operation is denied
    Denied,
    /// Approval is pending (waiting for user)
    Pending,
    /// Approval timed out
    Timeout,
}

impl ApprovalStatus {
    pub fn to_response_type(&self, request_id: &str) -> ControlResponseType {
        match self {
            ApprovalStatus::Approved => ControlResponseType::Allow {
                request_id: Some(request_id.to_string()),
            },
            ApprovalStatus::Denied => ControlResponseType::Deny {
                request_id: Some(request_id.to_string()),
                reason: Some("Denied by approval policy".to_string()),
            },
            ApprovalStatus::Pending => ControlResponseType::Pending {
                request_id: request_id.to_string(),
            },
            ApprovalStatus::Timeout => ControlResponseType::Deny {
                request_id: Some(request_id.to_string()),
                reason: Some("Approval timeout".to_string()),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permission_mode() {
        assert_eq!(PermissionMode::Allow.as_str(), "allow");
        assert_eq!(PermissionMode::Deny.as_str(), "deny");
        assert_eq!(PermissionMode::Auto.as_str(), "auto");
    }

    #[test]
    fn test_cli_message_serialization() {
        let msg = CLIMessage::ControlResponse {
            request_id: "test-123".to_string(),
            response: ControlResponseType::Allow {
                request_id: Some("test-123".to_string()),
            },
        };
        let json = msg.to_json().unwrap();
        assert!(json.contains("control_response"));
        assert!(json.contains("allow"));
    }

    #[test]
    fn test_can_use_tool_request() {
        let request = SDKControlRequest::new(ControlRequestType::CanUseTool {
            tool_name: "Read".to_string(),
            tool_input: serde_json::json!({"path": "/tmp/test.txt"}),
            tool_use_id: "use-123".to_string(),
        });
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("can_use_tool"));
        assert!(json.contains("Read"));
    }
}
