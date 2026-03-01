use log::info;
use regex::Regex;
use rusqlite::{params, Connection};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

use crate::commands::agents::AgentDb;
use crate::commands::message::Message;
use crate::process::ProcessRegistry;
use crate::process::registry::build_claude_message;
use crate::process::registry::parse_multimodal_input;

/// Message types
#[derive(Debug, Clone)]
pub enum MessageType {
    /// User message from frontend
    User,
    /// Claude thinking output
    Thinking,
    /// Claude final response
    Response,
    /// Error output
    Error,
}

impl MessageType {
    pub fn as_str(&self) -> &str {
        match self {
            MessageType::User => "user",
            MessageType::Thinking => "thinking",
            MessageType::Response => "response",
            MessageType::Error => "error",
        }
    }
}

/// Message target types
#[derive(Debug, Clone)]
pub enum MessageTarget {
    /// Send to a project agent by name
    AgentName(String),
    /// Send directly to a process by run_id
    RunId(String),
}

/// Message middleware for unified message handling
pub struct MessageMiddleware {
    /// Database connection (Arc<Mutex<Connection>>)
    db: Arc<Mutex<Connection>>,
    /// Process registry
    registry: Arc<ProcessRegistry>,
}

impl MessageMiddleware {
    /// Create a new MessageMiddleware
    pub fn new(db: Arc<Mutex<Connection>>, registry: Arc<ProcessRegistry>) -> Self {
        Self { db, registry }
    }

    /// Parse message content and extract the target
    /// Supports two formats:
    /// - @username - send to project agent
    /// - @run_id:message - send directly to a running process
    pub fn parse_message_target(content: &str) -> Option<(MessageTarget, String)> {
        // First try to match @run_id:pattern (run_id followed by colon)
        // run_id is a UUID format
        let run_id_pattern = Regex::new(r"@([0-9a-f-]{36}):(.*)").ok()?;

        if let Some(caps) = run_id_pattern.captures(content) {
            let run_id = caps.get(1)?.as_str().to_string();
            let message = caps.get(2).map(|m| m.as_str()).unwrap_or("").to_string();
            return Some((MessageTarget::RunId(run_id), message));
        }

        // Then try @username pattern (alphanumeric and underscore)
        let username_pattern = Regex::new(r"@(\w+)").ok()?;

        if let Some(caps) = username_pattern.captures(content) {
            let username = caps.get(1)?.as_str().to_string();
            // Make sure it's not a run_id pattern that we already checked
            // If the username looks like a UUID, skip it
            if !username.contains('-') || username.len() != 36 {
                return Some((MessageTarget::AgentName(username), content.to_string()));
            }
        }

        None
    }

    /// Find agent by name in a specific project
    fn find_agent_by_name(
        &self,
        project_id: &str,
        agent_name: &str,
    ) -> Result<Option<(String, String)>, String> {
        let conn = self.db.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT a.id, a.name
                 FROM agents a
                 INNER JOIN project_agents pa ON a.id = pa.agent_id
                 WHERE pa.project_id = ?1 AND a.name = ?2",
            )
            .map_err(|e| e.to_string())?;

        let result = stmt
            .query_row(params![project_id, agent_name], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .ok();

        Ok(result)
    }

    /// Get project path from project_id
    fn get_project_path(&self, project_id: &str) -> Result<String, String> {
        let conn = self.db.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT working_dir FROM projects WHERE id = ?1")
            .map_err(|e| e.to_string())?;

        let result = stmt
            .query_row(params![project_id], |row| row.get::<_, String>(0))
            .map_err(|e| format!("Project not found: {}", e))?;

        Ok(result)
    }

    /// Find agent by run_id in project_agents
    fn get_agent_info_by_run_id(
        &self,
        project_id: &str,
        run_id: &str,
    ) -> Result<(String, String, Option<String>, Option<String>), String> {
        let conn = self.db.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT pa.agent_id, a.name, a.icon, a.color
                 FROM project_agents pa
                 INNER JOIN agents a ON pa.agent_id = a.id
                 WHERE pa.project_id = ?1 AND pa.id = ?2",
            )
            .map_err(|e| e.to_string())?;

        stmt.query_row(params![project_id, run_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(|e| format!("Agent not found for run_id: {}", e))
    }

    /// Save a message to database
    fn save_message(&self, message: &Message) -> Result<(), String> {
        let conn = self.db.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO messages (id, project_id, sender_id, sender_name, target_id, target_name, content, json_content, message_type, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                message.id,
                message.project_id,
                message.sender_id,
                message.sender_name,
                message.target_id,
                message.target_name,
                message.content,
                message.json_content,
                message.message_type,
                message.created_at,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Handle incoming message from frontend
    /// 1. Parse target (run_id / agent)
    /// 2. Build JSON using build_claude_message
    /// 3. Send to target process
    /// 4. Save to database
    ///
    /// Note: This method assumes the agent is already running.
    /// If the agent is not running, caller should start it first.
    pub async fn handle_incoming(
        &self,
        app: AppHandle,
        project_id: String,
        content: String,
        sender: String,
        sender_name: String,
    ) -> Result<Message, String> {
        info!(
            "MessageMiddleware::handle_incoming: project_id={}, sender={}, content={}",
            project_id, sender, content
        );

        // Step 1: Parse target from content
        let target_username = Self::parse_message_target(&content);

        // Step 2: Determine target agent (with lock)
        let (target_agent_id, target_agent_name, project_path, json_content) = {
            let conn = self.db.lock().map_err(|e| e.to_string())?;

            // Determine target agent
            let (agent_id, agent_name) = if let Some((MessageTarget::AgentName(username), _)) = &target_username {
                // Find agent by username in this project
                let mut stmt = conn
                    .prepare(
                        "SELECT a.id, a.name
                         FROM agents a
                         INNER JOIN project_agents pa ON a.id = pa.agent_id
                         WHERE pa.project_id = ?1 AND a.name = ?2",
                    )
                    .map_err(|e| e.to_string())?;

                let result = stmt
                    .query_row(params![&project_id, username], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })
                    .ok()
                    .ok_or_else(|| format!("Agent @{} not found in project", username))?;
                (result.0, Some(result.1))
            } else {
                // No @ mention, find TeamLead
                let mut stmt = conn
                    .prepare(
                        "SELECT a.id, a.name
                         FROM agents a
                         INNER JOIN project_agents pa ON a.id = pa.agent_id
                         WHERE pa.project_id = ?1 AND a.role_type = 'teamlead'",
                    )
                    .map_err(|e| e.to_string())?;

                let result = stmt
                    .query_row(params![&project_id], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })
                    .ok()
                    .ok_or_else(|| "No TeamLead found in project".to_string())?;
                (result.0, Some(result.1))
            };

            // Get project path
            let mut stmt = conn
                .prepare("SELECT working_dir FROM projects WHERE id = ?1")
                .map_err(|e| e.to_string())?;

            let project_path: String = stmt
                .query_row(params![&project_id], |row| row.get::<_, String>(0))
                .map_err(|e| format!("Project not found: {}", e))?;

            // Build JSON using build_claude_message (no lock needed)
            let json_content = parse_multimodal_input(&content);
            let json_content = build_claude_message(&json_content).to_string();

            (agent_id, agent_name, project_path, json_content)
        };
        // Lock released here

        // Step 3: Send to target process (requires agent to be running)
        let run_id = self
            .registry
            .find_teammate_run_id(&project_path, &target_agent_id)
            .ok_or_else(|| "Agent is not running".to_string())?;

        // Send the message to process (lock released inside send_to_process_async)
        if let Err(e) = self.registry
            .send_to_process_async(&run_id, &json_content)
            .await
        {
            return Err(format!("Failed to send message to agent: {}", e));
        }

        // Step 4: Save to database (acquire lock again)
        let message = Message {
            id: Uuid::new_v4().to_string(),
            project_id: project_id.clone(),
            sender_id: sender.clone(),
            sender_name: sender_name.clone(),
            sender_avatar: None,
            sender_color: None,
            target_id: target_agent_id.clone(),
            target_name: target_agent_name.clone(),
            content: content.clone(),
            json_content: Some(json_content),
            message_type: MessageType::User.as_str().to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };

        self.save_message(&message)?;

        info!("Message sent successfully: {}", message.id);

        Ok(message)
    }

    /// Handle outgoing message from agent process
    /// 1. Parse message type (thinking/response/error)
    /// 2. Build structured JSON
    /// 3. Save to database
    /// 4. Detect @mention and forward
    /// 5. Push to frontend (skip system-init messages)
    pub async fn handle_outgoing(
        &self,
        app: AppHandle,
        run_id: String,
        project_id: String,
        output: String,
    ) -> Result<Option<Message>, String> {
        info!("MessageMiddleware::handle_outgoing: run_id={}", run_id);

        // Check if this is a system-init message (should save to DB but not emit to frontend)
        let is_system_init = if let Ok(json) = serde_json::from_str::<serde_json::Value>(&output) {
            json.get("type").and_then(|v| v.as_str()) == Some("system")
                && json.get("subtype").and_then(|v| v.as_str()) == Some("init")
        } else {
            false
        };

        // Step 1: Parse message type from output
        let (message_type, content) = self.parse_agent_output(&output)?;

        // Step 2: Get agent info
        let (agent_id, agent_name, agent_icon, agent_color) =
            self.get_agent_info_by_run_id(&project_id, &run_id)?;

        // Step 3: Build message
        let message = Message {
            id: Uuid::new_v4().to_string(),
            project_id: project_id.clone(),
            sender_id: agent_id,
            sender_name: agent_name,
            sender_avatar: agent_icon,
            sender_color: agent_color,
            target_id: "user".to_string(),
            target_name: Some("You".to_string()),
            content,
            json_content: Some(output.clone()),
            message_type: message_type.as_str().to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };

        // Step 4: Save to database
        self.save_message(&message)?;

        // Step 5: Check @mention and forward (if response)
        if matches!(message_type, MessageType::Response) {
            self.handle_mention_forward(&run_id, &project_id, &message.content).await?;
        }

        // Step 6: Emit to frontend (skip system-init messages - they are saved but not displayed)
        if !is_system_init {
            let _ = app.emit("project-message-update", &project_id);
        }

        info!(
            "Saved {} message: {} (emit: {})",
            message_type.as_str(),
            message.id,
            !is_system_init
        );

        Ok(Some(message))
    }

    /// Parse agent output to determine message type and content
    fn parse_agent_output(&self, output: &str) -> Result<(MessageType, String), String> {
        // Try to parse as JSON
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(output) {
            let msg_type = json.get("type").and_then(|v| v.as_str()).unwrap_or("response");

            let content = if msg_type == "thinking" {
                // For thinking, get the thinking content
                json.get("thinking").and_then(|v| v.as_str()).map(|s| s.to_string())
            } else {
                // For result/response, get the result or message text
                json.get("result").and_then(|v| v.as_str()).map(|s| s.to_string())
                    .or_else(|| {
                        if let Some(msg) = json.get("message") {
                            if let Some(content) = msg.get("content").and_then(|c| c.as_array()) {
                                for item in content {
                                    if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                                        if !text.is_empty() {
                                            return Some(text.to_string());
                                        }
                                    }
                                }
                            }
                        }
                        None
                    })
            };

            if let Some(text) = content {
                if !text.trim().is_empty() {
                    let mt = if msg_type == "thinking" {
                        MessageType::Thinking
                    } else {
                        MessageType::Response
                    };
                    return Ok((mt, text));
                }
            }
        }

        // Fallback: treat as response
        Ok((MessageType::Response, output.to_string()))
    }

    /// Handle @mention forwarding for agent responses
    async fn handle_mention_forward(
        &self,
        source_run_id: &str,
        project_id: &str,
        content: &str,
    ) -> Result<(), String> {
        if let Some((target, message)) = Self::parse_message_target(content) {
            match target {
                MessageTarget::RunId(target_run_id) => {
                    info!(
                        "Detected @run_id: mention in output from {} to {}",
                        source_run_id, target_run_id
                    );
                    // Forward directly to the target run_id
                    self.registry
                        .send_to_process_async(&target_run_id, &message)
                        .await?;
                }
                MessageTarget::AgentName(agent_name) => {
                    info!(
                        "Detected @agent mention in output from {} to @{}",
                        source_run_id, agent_name
                    );

                    // Find the agent by name in the project
                    if let Some((target_agent_id, _)) = self.find_agent_by_name(project_id, &agent_name)? {
                        // Get project path
                        let project_path = self.get_project_path(project_id)?;

                        // Find the run_id for this agent in the project
                        if let Some(target_run_id) = self.registry.find_teammate_run_id(&project_path, &target_agent_id) {
                            info!(
                                "Forwarding message from {} to agent {} (run_id: {})",
                                source_run_id, agent_name, target_run_id
                            );
                            self.registry
                                .send_to_process_async(&target_run_id, &message)
                                .await?;
                        } else {
                            info!(
                                "Agent {} is not running, message not forwarded",
                                agent_name
                            );
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

/// Forward message to target by directly getting stdin handle
pub async fn forward_to_target(
    target: MessageTarget,
    content: &str,
    registry: Arc<ProcessRegistry>,
) -> Result<(), String> {
    match target {
        MessageTarget::RunId(run_id) => {
            info!("Forwarding message directly to run_id: {}", run_id);
            registry.send_to_process_async(&run_id, content).await?;
            Ok(())
        }
        MessageTarget::AgentName(_) => {
            Err("Agent name forwarding requires database lookup".to_string())
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_run_id_target() {
        let content = "@550e8400-e29b-41d4-a716-446655440000:Hello, this is a message";
        let result = MessageMiddleware::parse_message_target(content);
        assert!(result.is_some());

        let (target, message) = result.unwrap();
        match target {
            MessageTarget::RunId(run_id) => {
                assert_eq!(run_id, "550e8400-e29b-41d4-a716-446655440000");
                assert_eq!(message, "Hello, this is a message");
            }
            _ => panic!("Expected RunId target"),
        }
    }

    #[test]
    fn test_parse_username_target() {
        let content = "@developer Please help me";
        let result = MessageMiddleware::parse_message_target(content);
        assert!(result.is_some());

        let (target, message) = result.unwrap();
        match target {
            MessageTarget::AgentName(name) => {
                assert_eq!(name, "developer");
                assert_eq!(message, "@developer Please help me");
            }
            _ => panic!("Expected AgentName target"),
        }
    }

    #[test]
    fn test_no_mention() {
        let content = "Just a regular message";
        let result = MessageMiddleware::parse_message_target(content);
        assert!(result.is_none());
    }
}
