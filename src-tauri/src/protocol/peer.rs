//! ProtocolPeer implementation for Claude CLI control protocol
//!
//! This module provides the ProtocolPeer struct which handles bidirectional
//! communication with the Claude SDK process.

use crate::protocol::types::{
    ApprovalStatus, CLIMessage, ControlResponseType, PermissionMode, SDKControlRequest,
};
use futures::StreamExt;
use log::{debug, error, info, warn};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::{broadcast, mpsc, Mutex, Notify};

/// Callback for handling control requests from the SDK
pub trait ControlRequestHandler: Send + Sync {
    /// Handle a tool use approval request
    fn on_can_use_tool(
        &self,
        tool_name: &str,
        tool_input: serde_json::Value,
        tool_use_id: &str,
    ) -> impl std::future::Future<Output = ApprovalStatus> + Send;

    /// Handle a hook callback
    fn on_hook_callback(
        &self,
        hook_name: &str,
        hook_input: serde_json::Value,
    ) -> impl std::future::Future<Output = bool> + Send;
}

/// Default handler that denies all requests
pub struct DefaultHandler;

impl ControlRequestHandler for DefaultHandler {
    async fn on_can_use_tool(
        &self,
        _tool_name: &str,
        _tool_input: serde_json::Value,
        _tool_use_id: &str,
    ) -> ApprovalStatus {
        ApprovalStatus::Denied
    }

    async fn on_hook_callback(
        &self,
        _hook_name: &str,
        _hook_input: serde_json::Value,
    ) -> bool {
        false
    }
}

/// ProtocolPeer handles communication with a Claude SDK process
#[derive(Clone)]
pub struct ProtocolPeer {
    /// Stdin for sending control messages
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    /// Flag to control the read loop
    running: Arc<Mutex<bool>>,
    /// Notification for shutdown
    shutdown: Arc<Notify>,
    /// Broadcast channel for control requests
    request_tx: broadcast::Sender<SDKControlRequest>,
    /// Current permission mode
    permission_mode: Arc<Mutex<PermissionMode>>,
}

impl ProtocolPeer {
    /// Create a new ProtocolPeer
    pub fn new(stdin: ChildStdin, stdout: ChildStdout) -> (Self, broadcast::Receiver<SDKControlRequest>) {
        let (request_tx, request_rx) = broadcast::channel(100);
        let request_tx_clone = request_tx.clone();

        let peer = Self {
            stdin: Arc::new(Mutex::new(Some(stdin))),
            running: Arc::new(Mutex::new(true)),
            shutdown: Arc::new(Notify::new()),
            request_tx,
            permission_mode: Arc::new(Mutex::new(PermissionMode::Auto)),
        };

        // Start the read loop
        let running = peer.running.clone();
        let shutdown = peer.shutdown.clone();

        tokio::spawn(async move {
            Self::read_loop(stdout, running, shutdown, request_tx_clone).await;
        });

        (peer, request_rx)
    }

    /// Create a ProtocolPeer without starting the read loop (for testing)
    #[cfg(test)]
    pub fn new_mock(stdin: ChildStdin) -> Self {
        Self {
            stdin: Arc::new(Mutex::new(Some(stdin))),
            running: Arc::new(Mutex::new(false)),
            shutdown: Arc::new(Notify::new()),
            request_tx: broadcast::channel(100).0,
            permission_mode: Arc::new(Mutex::new(PermissionMode::Auto)),
        }
    }

    /// Read loop - processes messages from stdout
    async fn read_loop(
        stdout: ChildStdout,
        running: Arc<Mutex<bool>>,
        shutdown: Arc<Notify>,
        request_tx: broadcast::Sender<SDKControlRequest>,
    ) {
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();

        loop {
            // Check if we should stop
            {
                let is_running = running.lock().await;
                if !*is_running {
                    info!("ProtocolPeer read loop stopped");
                    break;
                }
            }

            // Wait for shutdown signal or new data
            tokio::select! {
                _ = shutdown.notified() => {
                    info!("ProtocolPeer received shutdown signal");
                    break;
                }
                result = lines.next_line() => {
                    match result {
                        Ok(Some(line)) => {
                            debug!("Received from SDK: {}", line);
                            if let Ok(request) = serde_json::from_str::<SDKControlRequest>(&line) {
                                if let Err(e) = request_tx.send(request) {
                                    warn!("No receivers for control request: {}", e);
                                }
                            } else {
                                // Try parsing as any JSON to see what we received
                                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
                                    debug!("Received non-control message: {:?}", value.get("type"));
                                }
                            }
                        }
                        Ok(None) => {
                            info!("SDK stdout closed");
                            break;
                        }
                        Err(e) => {
                            error!("Error reading from SDK: {}", e);
                            break;
                        }
                    }
                }
            }
        }
    }

    /// Send a control response to the SDK
    pub async fn send_control_response(
        &self,
        request_id: &str,
        response: ControlResponseType,
    ) -> Result<(), String> {
        let msg = CLIMessage::ControlResponse {
            request_id: request_id.to_string(),
            response,
        };
        self.send_message(msg).await
    }

    /// Send a hook response
    pub async fn send_hook_response(&self, request_id: &str, allowed: bool) -> Result<(), String> {
        let response = if allowed {
            ControlResponseType::Allow {
                request_id: Some(request_id.to_string()),
            }
        } else {
            ControlResponseType::Deny {
                request_id: Some(request_id.to_string()),
                reason: Some("Denied by policy".to_string()),
            }
        };

        self.send_control_response(request_id, response).await
    }

    /// Send an interrupt signal to the agent
    pub async fn interrupt(&self, reason: Option<String>) -> Result<(), String> {
        let msg = CLIMessage::Interrupt { reason };
        self.send_message(msg).await
    }

    /// Set the permission mode
    pub async fn set_permission_mode(&self, mode: PermissionMode) -> Result<(), String> {
        let mut current = self.permission_mode.lock().await;
        *current = mode;

        let msg = CLIMessage::PermissionModeChanged { mode };
        self.send_message(msg).await
    }

    /// Get current permission mode
    pub async fn get_permission_mode(&self) -> PermissionMode {
        *self.permission_mode.lock().await
    }

    /// Initialize the session with the SDK
    pub async fn initialize(
        &self,
        session_id: &str,
        permission_mode: PermissionMode,
        allowed_tools: Option<Vec<String>>,
        denied_tools: Option<Vec<String>>,
    ) -> Result<(), String> {
        let msg = CLIMessage::Initialize {
            session_id: session_id.to_string(),
            permission_mode,
            allowed_tools,
            denied_tools,
        };
        self.send_message(msg).await
    }

    /// Check if a tool is allowed without sending a full request
    pub async fn query_can_use_tool(&self, tool_name: &str) -> Result<(), String> {
        let msg = CLIMessage::CanUseToolQuery {
            tool_name: tool_name.to_string(),
        };
        self.send_message(msg).await
    }

    /// Send any CLIMessage to the SDK
    async fn send_message(&self, msg: CLIMessage) -> Result<(), String> {
        let json = msg.to_json().map_err(|e| e.to_string())?;
        let line = format!("{}\n", json);

        let mut stdin_guard = self.stdin.lock().await;
        if let Some(ref mut stdin) = *stdin_guard {
            use tokio::io::AsyncWriteExt;
            stdin.write_all(line.as_bytes()).await.map_err(|e| e.to_string())?;
            stdin.flush().await.map_err(|e| e.to_string())?;
            debug!("Sent to SDK: {}", json);
            Ok(())
        } else {
            Err("stdin not available".to_string())
        }
    }

    /// Stop the read loop
    pub async fn stop(&self) {
        {
            let mut is_running = self.running.lock().await;
            *is_running = false;
        }
        self.shutdown.notify_waiters();
        info!("ProtocolPeer stopped");
    }

    /// Subscribe to control requests
    pub fn subscribe(&self) -> broadcast::Receiver<SDKControlRequest> {
        self.request_tx.subscribe()
    }
}

impl Drop for ProtocolPeer {
    fn drop(&mut self) {
        // Trigger shutdown
        self.shutdown.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{ChildStdin, Stdio};
    use tokio::process::Command;

    #[tokio::test]
    async fn test_permission_mode() {
        let mode = PermissionMode::Allow;
        assert_eq!(mode.as_str(), "allow");
    }

    #[test]
    fn test_control_response_serialization() {
        let response = ControlResponseType::Allow {
            request_id: Some("test-123".to_string()),
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("allow"));
    }
}
