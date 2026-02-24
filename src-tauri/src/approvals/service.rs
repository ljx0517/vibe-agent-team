//! Approval service trait and implementations
//!
//! This module defines the ExecutorApprovalService trait for handling tool approval requests.

use crate::protocol::types::ApprovalStatus;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::broadcast;

/// Error type for approval operations
#[derive(Debug, thiserror::Error)]
pub enum ExecutorApprovalError {
    #[error("Approval service not available")]
    ServiceUnavailable,
    #[error("Approval request failed: {0}")]
    RequestFailed(String),
    #[error("Timeout waiting for approval")]
    Timeout,
    #[error("Agent not found: {0}")]
    AgentNotFound(String),
}

/// Event emitted when approval is needed
#[derive(Debug, Clone)]
pub struct ApprovalEvent {
    /// Run ID
    pub run_id: String,
    /// Agent ID
    pub agent_id: String,
    /// Tool name
    pub tool_name: String,
    /// Tool input (JSON)
    pub tool_input: serde_json::Value,
    /// Tool use ID
    pub tool_use_id: String,
    /// Request ID from protocol
    pub request_id: String,
}

/// Trait for approval service implementations
#[async_trait]
pub trait ExecutorApprovalService: Send + Sync {
    /// Request approval for a tool use
    async fn request_tool_approval(
        &self,
        agent_id: &str,
        tool_name: &str,
        tool_input: serde_json::Value,
        tool_use_id: &str,
        request_id: &str,
        run_id: &str,
    ) -> Result<ApprovalStatus, ExecutorApprovalError>;

    /// Check if a tool is auto-approved for an agent
    fn is_tool_auto_approved(&self, agent_id: &str, tool_name: &str) -> bool;

    /// Get approval mode for an agent
    fn get_approval_mode(&self, agent_id: &str) -> ApprovalMode;
}

/// Approval mode for an agent
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalMode {
    /// All operations allowed without approval
    Allow,
    /// All operations denied
    Deny,
    /// Operations require approval based on rules
    Auto,
    /// Operations require manual approval
    Manual,
}

impl Default for ApprovalMode {
    fn default() -> Self {
        Self::Auto
    }
}

impl ApprovalMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ApprovalMode::Allow => "allow",
            ApprovalMode::Deny => "deny",
            ApprovalMode::Auto => "auto",
            ApprovalMode::Manual => "manual",
        }
    }
}

/// Approval service that denies all requests
pub struct DenyAllApprovalService;

#[async_trait]
impl ExecutorApprovalService for DenyAllApprovalService {
    async fn request_tool_approval(
        &self,
        _agent_id: &str,
        _tool_name: &str,
        _tool_input: serde_json::Value,
        _tool_use_id: &str,
        _request_id: &str,
        _run_id: &str,
    ) -> Result<ApprovalStatus, ExecutorApprovalError> {
        Ok(ApprovalStatus::Denied)
    }

    fn is_tool_auto_approved(&self, _agent_id: &str, _tool_name: &str) -> bool {
        false
    }

    fn get_approval_mode(&self, _agent_id: &str) -> ApprovalMode {
        ApprovalMode::Deny
    }
}

/// Approval service that allows all requests
pub struct AllowAllApprovalService;

#[async_trait]
impl ExecutorApprovalService for AllowAllApprovalService {
    async fn request_tool_approval(
        &self,
        _agent_id: &str,
        _tool_name: &str,
        _tool_input: serde_json::Value,
        _tool_use_id: &str,
        _request_id: &str,
        _run_id: &str,
    ) -> Result<ApprovalStatus, ExecutorApprovalError> {
        Ok(ApprovalStatus::Approved)
    }

    fn is_tool_auto_approved(&self, _agent_id: &str, _tool_name: &str) -> bool {
        true
    }

    fn get_approval_mode(&self, _agent_id: &str) -> ApprovalMode {
        ApprovalMode::Allow
    }
}

/// Approval event sender
pub struct ApprovalEventSender {
    tx: broadcast::Sender<ApprovalEvent>,
}

impl ApprovalEventSender {
    pub fn new(buffer_size: usize) -> (Self, broadcast::Receiver<ApprovalEvent>) {
        let (tx, rx) = broadcast::channel(buffer_size);
        (Self { tx }, rx)
    }

    pub fn sender(&self) -> broadcast::Sender<ApprovalEvent> {
        self.tx.clone()
    }

    pub fn emit(&self, event: ApprovalEvent) -> Result<usize, tokio::sync::broadcast::error::SendError<ApprovalEvent>> {
        self.tx.send(event)
    }
}

impl Clone for ApprovalEventSender {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_deny_all_service() {
        let service = DenyAllApprovalService;
        let status = service
            .request_tool_approval(
                "agent-1",
                "Read",
                serde_json::json!({"path": "/tmp/test.txt"}),
                "use-123",
                "req-456",
                "run-789",
            )
            .await
            .unwrap();
        assert_eq!(status, ApprovalStatus::Denied);
    }

    #[tokio::test]
    async fn test_allow_all_service() {
        let service = AllowAllApprovalService;
        let status = service
            .request_tool_approval(
                "agent-1",
                "Read",
                serde_json::json!({"path": "/tmp/test.txt"}),
                "use-123",
                "req-456",
                "run-789",
            )
            .await
            .unwrap();
        assert_eq!(status, ApprovalStatus::Approved);
    }

    #[test]
    fn test_approval_mode() {
        assert_eq!(ApprovalMode::Allow.as_str(), "allow");
        assert_eq!(ApprovalMode::Deny.as_str(), "deny");
        assert_eq!(ApprovalMode::Auto.as_str(), "auto");
        assert_eq!(ApprovalMode::Manual.as_str(), "manual");
    }
}
