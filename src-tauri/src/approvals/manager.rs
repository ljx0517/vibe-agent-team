//! Approval manager implementation
//!
//! This module provides the manager that handles approval requests based on
//! agent permission configuration from the database.

use crate::approvals::service::{ApprovalEvent, ApprovalEventSender, ApprovalMode, ExecutorApprovalError, ExecutorApprovalService};
use crate::protocol::types::ApprovalStatus;
use async_trait::async_trait;
use log::{debug, info, warn};
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex, RwLock};

/// Agent permission configuration from database
#[derive(Debug, Clone)]
pub struct AgentPermissionConfig {
    pub agent_id: String,
    pub enable_file_read: bool,
    pub enable_file_write: bool,
    pub enable_network: bool,
    pub tools: Vec<String>,
    pub denied_tools: Vec<String>,
}

impl Default for AgentPermissionConfig {
    fn default() -> Self {
        Self {
            agent_id: String::new(),
            enable_file_read: true,
            enable_file_write: true,
            enable_network: true,
            tools: Vec::new(),
            denied_tools: Vec::new(),
        }
    }
}

/// Manager for handling approval requests based on agent configuration
pub struct ApprovalManager {
    /// Cache of agent permission configs
    config_cache: Arc<RwLock<HashMap<String, AgentPermissionConfig>>>,
    /// Approval mode for each agent
    approval_modes: Arc<RwLock<HashMap<String, ApprovalMode>>>,
    /// Event sender for approval requests
    event_sender: Option<ApprovalEventSender>,
    /// Database path
    db_path: Option<PathBuf>,
}

impl ApprovalManager {
    /// Create a new approval manager
    pub fn new() -> Self {
        Self {
            config_cache: Arc::new(RwLock::new(HashMap::new())),
            approval_modes: Arc::new(RwLock::new(HashMap::new())),
            event_sender: None,
            db_path: None,
        }
    }

    /// Set the database path
    pub fn with_db_path(mut self, path: PathBuf) -> Self {
        self.db_path = Some(path);
        self
    }

    /// Set the approval event sender
    pub fn with_event_sender(mut self, sender: ApprovalEventSender) -> Self {
        self.event_sender = Some(sender);
        self
    }

    /// Load agent config from database
    pub async fn load_agent_config(&self, agent_id: &str) -> Result<AgentPermissionConfig, ExecutorApprovalError> {
        // Check cache first
        {
            let cache = self.config_cache.read().await;
            if let Some(config) = cache.get(agent_id) {
                return Ok(config.clone());
            }
        }

        // Load from database if available
        if let Some(ref db_path) = self.db_path {
            if let Ok(conn) = Connection::open(db_path) {
                let config = self.load_from_connection(&conn, agent_id);
                if let Ok(config) = config {
                    let mut cache = self.config_cache.write().await;
                    cache.insert(agent_id.to_string(), config.clone());
                    return Ok(config);
                }
            }
        }

        // Return default config if not found
        Ok(AgentPermissionConfig {
            agent_id: agent_id.to_string(),
            ..Default::default()
        })
    }

    fn load_from_connection(&self, conn: &Connection, agent_id: &str) -> Result<AgentPermissionConfig, ExecutorApprovalError> {
        let mut stmt = conn
            .prepare(
                "SELECT id, enable_file_read, enable_file_write, enable_network, tools
                 FROM agents WHERE id = ?",
            )
            .map_err(|e| ExecutorApprovalError::RequestFailed(e.to_string()))?;

        let config = stmt
            .query_row([agent_id], |row| {
                let tools_str: Option<String> = row.get(4)?;
                let tools: Vec<String> = tools_str
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default();

                Ok(AgentPermissionConfig {
                    agent_id: row.get(0)?,
                    enable_file_read: row.get::<_, i32>(1)? != 0,
                    enable_file_write: row.get::<_, i32>(2)? != 0,
                    enable_network: row.get::<_, i32>(3)? != 0,
                    tools,
                    denied_tools: Vec::new(),
                })
            })
            .map_err(|_| ExecutorApprovalError::AgentNotFound(agent_id.to_string()))?;

        Ok(config)
    }

    /// Set approval mode for an agent
    pub async fn set_approval_mode(&self, agent_id: &str, mode: ApprovalMode) {
        let mut modes = self.approval_modes.write().await;
        modes.insert(agent_id.to_string(), mode);
    }

    /// Get approval mode for an agent
    pub async fn get_approval_mode(&self, agent_id: &str) -> ApprovalMode {
        let modes = self.approval_modes.read().await;
        modes.get(agent_id).copied().unwrap_or(ApprovalMode::Auto)
    }

    /// Clear cache for an agent
    pub async fn clear_cache(&self, agent_id: &str) {
        let mut cache = self.config_cache.write().await;
        cache.remove(agent_id);
    }

    /// Emit approval event
    pub fn emit_approval_event(&self, event: ApprovalEvent) {
        if let Some(ref sender) = self.event_sender {
            if let Err(e) = sender.emit(event) {
                warn!("Failed to emit approval event: {}", e);
            }
        }
    }

    /// Check if tool is allowed based on config
    fn check_tool_allowed(&self, config: &AgentPermissionConfig, tool_name: &str) -> bool {
        // Check denied tools first
        if config.denied_tools.iter().any(|d| tool_name.starts_with(d) || tool_name == d) {
            return false;
        }

        // Check allowed tools list
        if config.tools.is_empty() {
            // Empty list means all tools are allowed
            return true;
        }

        config.tools.iter().any(|t| tool_name.starts_with(t) || tool_name == t)
    }
}

impl Default for ApprovalManager {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ExecutorApprovalService for ApprovalManager {
    async fn request_tool_approval(
        &self,
        agent_id: &str,
        tool_name: &str,
        tool_input: serde_json::Value,
        tool_use_id: &str,
        request_id: &str,
        run_id: &str,
    ) -> Result<ApprovalStatus, ExecutorApprovalError> {
        // Get approval mode
        let mode = self.get_approval_mode(agent_id).await;

        debug!("Approval request for tool '{}' with mode {:?}", tool_name, mode);

        match mode {
            ApprovalMode::Allow => {
                return Ok(ApprovalStatus::Approved);
            }
            ApprovalMode::Deny => {
                return Ok(ApprovalStatus::Denied);
            }
            ApprovalMode::Auto => {
                // Check config for auto-approval rules
                let config = self.load_agent_config(agent_id).await?;

                // Check tool-specific permissions
                if self.check_tool_allowed(&config, tool_name) {
                    return Ok(ApprovalStatus::Approved);
                }

                // Check category permissions
                let category = tool_name.split("::").next().unwrap_or(tool_name);
                match category {
                    "Read" | "Glob" | "Grep" | "SearchFiles" => {
                        if config.enable_file_read {
                            return Ok(ApprovalStatus::Approved);
                        }
                    }
                    "Write" | "Edit" | "Create" | "Delete" => {
                        if config.enable_file_write {
                            return Ok(ApprovalStatus::Approved);
                        }
                    }
                    "Bash" | "Tool" | "WebFetch" | "WebSearch" => {
                        if config.enable_network {
                            return Ok(ApprovalStatus::Approved);
                        }
                    }
                    _ => {}
                }

                // Deny if not auto-approved
                return Ok(ApprovalStatus::Denied);
            }
            ApprovalMode::Manual => {
                // Emit event and wait for manual approval
                self.emit_approval_event(ApprovalEvent {
                    run_id: run_id.to_string(),
                    agent_id: agent_id.to_string(),
                    tool_name: tool_name.to_string(),
                    tool_input,
                    tool_use_id: tool_use_id.to_string(),
                    request_id: request_id.to_string(),
                });

                // Return pending - actual response will come from event handler
                return Ok(ApprovalStatus::Pending);
            }
        }
    }

    fn is_tool_auto_approved(&self, agent_id: &str, tool_name: &str) -> bool {
        // Synchronous check using cached config
        let config = self.config_cache.blocking_read();
        if let Some(config) = config.get(agent_id) {
            return self.check_tool_allowed(config, tool_name);
        }
        false
    }

    fn get_approval_mode(&self, agent_id: &str) -> ApprovalMode {
        // Synchronous check
        let modes = self.approval_modes.blocking_read();
        modes.get(agent_id).copied().unwrap_or(ApprovalMode::Auto)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AgentPermissionConfig::default();
        assert!(config.enable_file_read);
        assert!(config.enable_file_write);
        assert!(config.enable_network);
        assert!(config.tools.is_empty());
    }

    #[tokio::test]
    async fn test_approval_manager_auto_mode() {
        let manager = ApprovalManager::new();
        manager.set_approval_mode("agent-1", ApprovalMode::Auto).await;
        let mode = manager.get_approval_mode("agent-1").await;
        assert_eq!(mode, ApprovalMode::Auto);
    }

    #[tokio::test]
    async fn test_tool_check() {
        let manager = ApprovalManager::new();
        let config = AgentPermissionConfig {
            agent_id: "agent-1".to_string(),
            tools: vec!["Read".to_string(), "Glob".to_string()],
            denied_tools: vec!["Bash".to_string()],
            ..Default::default()
        };

        let mut cache = manager.config_cache.write().await;
        cache.insert("agent-1".to_string(), config);

        assert!(manager.is_tool_auto_approved("agent-1", "Read"));
        assert!(manager.is_tool_auto_approved("agent-1", "Glob"));
        assert!(!manager.is_tool_auto_approved("agent-1", "Bash"));
    }
}
