use log::info;
use regex::Regex;
use rusqlite::{params, Connection};
use std::sync::{Arc, Mutex};

use crate::process::ProcessRegistry;

/// Message target types
#[derive(Debug, Clone)]
pub enum MessageTarget {
    /// Send to a project agent by name
    AgentName(String),
    /// Send directly to a process by run_id
    RunId(String),
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

/// Process teammate agent output, detect @mention and forward
/// This version uses project_path to enable @run_id: resolution
/// Note: @username forwarding requires database access which needs additional lifetime handling
pub async fn process_teammate_output_by_path(
    run_id: &str,
    project_path: &str,
    agent_id: &str,
    project_id: &str,
    output: &str,
    registry: Arc<ProcessRegistry>,
) -> Result<(), String> {
    // Parse the output to find @mentions
    if let Some((target, message)) = parse_message_target(output) {
        match target {
            MessageTarget::RunId(target_run_id) => {
                info!(
                    "Detected @run_id: mention in output from {} to {}",
                    run_id, target_run_id
                );
                // Forward directly to the target run_id
                forward_to_target(MessageTarget::RunId(target_run_id), &message, registry).await?;
            }
            MessageTarget::AgentName(agent_name) => {
                info!(
                    "Detected @agent mention in output from {} to @{}",
                    run_id, agent_name
                );
                // @username forwarding requires database access
                // This will be implemented in a future version with proper lifetime handling
                info!(
                    "@username forwarding for @{} requires database lookup (not yet implemented)",
                    agent_name
                );
            }
        }
    }

    Ok(())
}

/// Legacy function that takes db for full agent name resolution
#[allow(dead_code)]
pub async fn process_teammate_output_with_db(
    run_id: &str,
    project_id: &str,
    output: &str,
    registry: Arc<ProcessRegistry>,
    db: Arc<std::sync::Mutex<rusqlite::Connection>>,
) -> Result<(), String> {
    // Parse the output to find @mentions
    if let Some((target, message)) = parse_message_target(output) {
        match target {
            MessageTarget::RunId(target_run_id) => {
                info!(
                    "Detected @run_id: mention in output from {} to {}",
                    run_id, target_run_id
                );
                // Forward directly to the target run_id
                forward_to_target(MessageTarget::RunId(target_run_id), &message, registry).await?;
            }
            MessageTarget::AgentName(agent_name) => {
                info!(
                    "Detected @agent mention in output from {} to @{}",
                    run_id, agent_name
                );

                // Find the agent by name in the project
                let target_agent_id = {
                    let conn = db.lock().map_err(|e| e.to_string())?;

                    let mut stmt = conn
                        .prepare(
                            "SELECT a.id
                             FROM agents a
                             INNER JOIN project_agents pa ON a.id = pa.agent_id
                             WHERE pa.project_id = ?1 AND a.name = ?2",
                        )
                        .map_err(|e| e.to_string())?;

                    let result: Option<String> = stmt
                        .query_row(params![project_id, agent_name], |row| {
                            row.get(0)
                        })
                        .ok();

                    result
                };

                if let Some(agent_id) = target_agent_id {
                    // Get project path
                    let project_path = {
                        let conn = db.lock().map_err(|e| e.to_string())?;

                        let mut stmt = conn
                            .prepare("SELECT working_dir FROM projects WHERE id = ?1")
                            .map_err(|e| e.to_string())?;

                        stmt.query_row(params![project_id], |row| {
                            row.get::<_, String>(0)
                        })
                        .ok()
                    };

                    if let Some(path) = project_path {
                        // Find the run_id for this agent in the project
                        let target_run_id = registry.find_teammate_run_id(&path, &agent_id);

                        if let Some(tid) = target_run_id {
                            // Send message directly to the agent
                            info!(
                                "Forwarding message from {} to agent {} (run_id: {})",
                                run_id, agent_name, tid
                            );
                            registry.send_to_process_async(&tid, &message).await?;
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
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_run_id_target() {
        let content = "@550e8400-e29b-41d4-a716-446655440000:Hello, this is a message";
        let result = parse_message_target(content);
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
        let result = parse_message_target(content);
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
        let result = parse_message_target(content);
        assert!(result.is_none());
    }
}
