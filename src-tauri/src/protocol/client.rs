//! Approval client for permission requests
//!
//! This module provides the client for sending approval requests to the frontend
//! or other approval handlers.

use crate::protocol::types::{ApprovalStatus, ControlRequestType, SDKControlRequest};
use log::{debug, info, warn};
use std::sync::Arc;
use tokio::sync::{mpsc, Notify};

/// Event emitted when approval is needed
#[derive(Debug, Clone)]
pub struct ApprovalRequest {
    /// Unique request ID
    pub request_id: String,
    /// Tool name
    pub tool_name: String,
    /// Tool input (JSON)
    pub tool_input: serde_json::Value,
    /// Tool use ID
    pub tool_use_id: String,
    /// Sender for the response
    pub response_tx: mpsc::Sender<ApprovalStatus>,
}

/// Client for managing approval requests
pub struct ApprovalClient {
    /// Channel for sending approval requests
    request_tx: mpsc::Sender<ApprovalRequest>,
    /// Channel for receiving approval requests
    request_rx: Option<mpsc::Receiver<ApprovalRequest>>,
}

impl ApprovalClient {
    /// Create a new approval client
    pub fn new(buffer_size: usize) -> (Self, mpsc::Receiver<ApprovalRequest>) {
        let (request_tx, request_rx) = mpsc::channel(buffer_size);

        let client = Self {
            request_tx,
            request_rx: None,
        };

        (client, request_rx)
    }

    /// Get the sender for approval requests
    pub fn sender(&self) -> mpsc::Sender<ApprovalRequest> {
        self.request_tx.clone()
    }

    /// Request approval for a tool use
    pub async fn request_tool_approval(
        &self,
        request: &SDKControlRequest,
    ) -> ApprovalStatus {
        if let ControlRequestType::CanUseTool {
            ref tool_name,
            ref tool_input,
            ref tool_use_id,
        } = request.request_type
        {
            // Create response channel
            let (response_tx, mut response_rx) = mpsc::channel(1);

            // Send approval request
            let approval_request = ApprovalRequest {
                request_id: request.request_id.clone(),
                tool_name: tool_name.clone(),
                tool_input: tool_input.clone(),
                tool_use_id: tool_use_id.clone(),
                response_tx,
            };

            if let Err(e) = self.request_tx.send(approval_request).await {
                warn!("Failed to send approval request: {}", e);
                return ApprovalStatus::Denied;
            }

            // Wait for response with timeout
            let timeout = tokio::time::Duration::from_secs(30);
            match tokio::time::timeout(timeout, response_rx.recv()).await {
                Ok(Some(status)) => {
                    debug!("Received approval status: {:?}", status);
                    status
                }
                Ok(None) => {
                    warn!("Approval channel closed unexpectedly");
                    ApprovalStatus::Denied
                }
                Err(_) => {
                    warn!("Approval request timed out");
                    ApprovalStatus::Timeout
                }
            }
        } else {
            warn!("Received non-CanUseTool request: {:?}", request.request_type);
            ApprovalStatus::Denied
        }
    }
}

/// Default approval client that denies all requests
pub struct DenyAllApprovalClient;

impl DenyAllApprovalClient {
    pub async fn request_tool_approval(&self, _request: &SDKControlRequest) -> ApprovalStatus {
        ApprovalStatus::Denied
    }
}

/// Approval client that allows all requests
pub struct AllowAllApprovalClient;

impl AllowAllApprovalClient {
    pub async fn request_tool_approval(&self, _request: &SDKControlRequest) -> ApprovalStatus {
        ApprovalStatus::Approved
    }
}

/// Auto approval client based on rules
pub struct RuleBasedApprovalClient {
    /// Rules for auto-approving certain tools
    auto_approve_tools: Vec<String>,
    /// Rules for denying certain tools
    deny_tools: Vec<String>,
}

impl RuleBasedApprovalClient {
    pub fn new(auto_approve_tools: Vec<String>, deny_tools: Vec<String>) -> Self {
        Self {
            auto_approve_tools,
            deny_tools,
        }
    }

    pub async fn request_tool_approval(&self, request: &SDKControlRequest) -> ApprovalStatus {
        if let ControlRequestType::CanUseTool { ref tool_name, .. } = request.request_type {
            // Check deny list first
            if self.deny_tools.iter().any(|d| tool_name.starts_with(d)) {
                return ApprovalStatus::Denied;
            }

            // Check auto-approve list
            if self.auto_approve_tools.iter().any(|a| tool_name.starts_with(a)) {
                return ApprovalStatus::Approved;
            }

            // Default to denied for unknown tools
            ApprovalStatus::Denied
        } else {
            ApprovalStatus::Denied
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_approval_request_creation() {
        let (client, _rx) = ApprovalClient::new(10);
        assert!(!client.sender().is_closed());
    }

    #[tokio::test]
    async fn test_deny_all_client() {
        let client = DenyAllApprovalClient;
        let request = SDKControlRequest::new(ControlRequestType::CanUseTool {
            tool_name: "Read".to_string(),
            tool_input: serde_json::json!({"path": "/tmp/test.txt"}),
            tool_use_id: "use-123".to_string(),
        });

        let status = client.request_tool_approval(&request).await;
        assert_eq!(status, ApprovalStatus::Denied);
    }

    #[tokio::test]
    async fn test_allow_all_client() {
        let client = AllowAllApprovalClient;
        let request = SDKControlRequest::new(ControlRequestType::CanUseTool {
            tool_name: "Read".to_string(),
            tool_input: serde_json::json!({"path": "/tmp/test.txt"}),
            tool_use_id: "use-123".to_string(),
        });

        let status = client.request_tool_approval(&request).await;
        assert_eq!(status, ApprovalStatus::Approved);
    }

    #[tokio::test]
    async fn test_rule_based_client() {
        let client = RuleBasedApprovalClient::new(
            vec!["Read".to_string(), "Glob".to_string()],
            vec!["Bash".to_string()],
        );

        // Should be approved
        let request = SDKControlRequest::new(ControlRequestType::CanUseTool {
            tool_name: "Read".to_string(),
            tool_input: serde_json::json!({"path": "/tmp/test.txt"}),
            tool_use_id: "use-123".to_string(),
        });
        let status = client.request_tool_approval(&request).await;
        assert_eq!(status, ApprovalStatus::Approved);

        // Should be denied
        let request = SDKControlRequest::new(ControlRequestType::CanUseTool {
            tool_name: "Bash".to_string(),
            tool_input: serde_json::json!({"command": "ls"}),
            tool_use_id: "use-124".to_string(),
        });
        let status = client.request_tool_approval(&request).await;
        assert_eq!(status, ApprovalStatus::Denied);
    }
}
