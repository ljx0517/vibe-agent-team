use log::{error, info};
use regex::Regex;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use uuid::Uuid;

use crate::commands::agents::AgentDb;
use crate::process::ProcessRegistryState;
use crate::commands::teammate::send_to_teammate;

/// Message structure for the database
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Message {
    pub id: String,
    pub project_id: String,
    pub sender_id: String,
    pub sender_name: String,
    pub target_id: String,
    pub target_name: Option<String>,
    pub content: String,
    pub message_type: String, // "user", "thinking", "response"
    pub created_at: String,
}

/// Parse @username from content and return the username if found
fn parse_target_username(content: &str) -> Option<String> {
    // Match @username pattern (alphanumeric and underscore)
    let re = Regex::new(r"@(\w+)").ok()?;
    re.captures(content).map(|caps| caps.get(1).unwrap().as_str().to_string())
}

/// Find agent by name in a specific project
fn find_agent_by_name(
    conn: &rusqlite::Connection,
    project_id: &str,
    agent_name: &str,
) -> Result<Option<(String, String)>, String> {
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

/// Find the TeamLead agent in a specific project
fn find_teamlead(
    conn: &rusqlite::Connection,
    project_id: &str,
) -> Result<Option<(String, String)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT a.id, a.name
             FROM agents a
             INNER JOIN project_agents pa ON a.id = pa.agent_id
             WHERE pa.project_id = ?1 AND a.role_type = 'teamlead'",
        )
        .map_err(|e| e.to_string())?;

    let result = stmt
        .query_row(params![project_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .ok();

    Ok(result)
}

/// Find the project_agent_id (run_id) for a specific agent in a project
fn find_project_agent_run_id(
    conn: &rusqlite::Connection,
    project_id: &str,
    agent_id: &str,
) -> Result<Option<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT project_agent_id
             FROM project_agents
             WHERE project_id = ?1 AND agent_id = ?2",
        )
        .map_err(|e| e.to_string())?;

    let result = stmt
        .query_row(params![project_id, agent_id], |row| {
            row.get::<_, String>(0)
        })
        .ok();

    Ok(result)
}

/// Get project path from project_id
fn get_project_path(
    conn: &rusqlite::Connection,
    project_id: &str,
) -> Result<String, String> {
    let mut stmt = conn
        .prepare("SELECT working_dir FROM projects WHERE id = ?1")
        .map_err(|e| e.to_string())?;

    let result = stmt
        .query_row(params![project_id], |row| row.get::<_, String>(0))
        .map_err(|e| format!("Project not found: {}", e))?;

    Ok(result)
}

/// Send a message to a specific teammate agent
async fn send_message_to_agent(
    app: AppHandle,
    registry: State<'_, ProcessRegistryState>,
    project_path: &str,
    agent_id: &str,
    message: &str,
) -> Result<String, String> {
    // Get the run_id from registry for this agent in this project
    let run_id = registry
        .0
        .find_teammate_run_id(project_path, agent_id)
        .ok_or_else(|| "Agent is not running".to_string())?;

    // Send the message
    send_to_teammate(run_id.clone(), message.to_string(), registry).await?;

    Ok(run_id)
}

/// Start a teammate agent if not already running
async fn start_teammate_agent_only(
    app: AppHandle,
    db: State<'_, AgentDb>,
    registry: State<'_, ProcessRegistryState>,
    project_id: String,
    agent_id: String,
) -> Result<String, String> {
    // Get project path and agent info
    let (project_path, model) = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        let project_path = get_project_path(&conn, &project_id)?;

        let mut stmt = conn
            .prepare("SELECT name, model FROM agents WHERE id = ?1")
            .map_err(|e| e.to_string())?;

        let model: String = stmt
            .query_row(params![agent_id], |row| row.get::<_, String>(1))
            .map_err(|e| format!("Agent not found: {}", e))?;

        (project_path, model)
    };

    // Start the teammate agent
    let run_id = crate::commands::teammate::start_teammate_agent(
        app,
        agent_id,
        project_path,
        Some(model),
        db,
        registry,
    )
    .await?;

    Ok(run_id)
}

/// Send a message to a project (dispatch to target agent or TeamLead)
#[tauri::command]
pub async fn send_message(
    app: AppHandle,
    project_id: String,
    content: String,
    sender: String,
    sender_name: String,
    db: State<'_, AgentDb>,
    registry: State<'_, ProcessRegistryState>,
) -> Result<Message, String> {
    info!(
        "send_message: project_id={}, sender={}, content={}",
        project_id, sender, content
    );

    // Get all database info first, before any async operations
    let (target_agent_id, target_agent_name, project_path) = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;

        // Parse target from content
        let target_username = parse_target_username(&content);

        // Determine target agent
        let (agent_id, agent_name) = if let Some(username) = target_username {
            // Find agent by username in this project
            let result = find_agent_by_name(&conn, &project_id, &username)
                .map_err(|e| format!("Failed to find agent: {}", e))?
                .ok_or_else(|| format!("Agent @{} not found in project", username))?;
            (result.0, Some(result.1))
        } else {
            // No @ mention, find TeamLead
            let result = find_teamlead(&conn, &project_id)
                .map_err(|e| format!("Failed to find teamlead: {}", e))?
                .ok_or_else(|| "No TeamLead found in project".to_string())?;
            (result.0, Some(result.1))
        };

        // Get project path
        let path = get_project_path(&conn, &project_id)?;

        (agent_id, agent_name, path)
    };

    // Try to send to running agent, or start new one
    let run_id = match send_message_to_agent(
        app.clone(),
        registry.clone(),
        &project_path,
        &target_agent_id,
        &content,
    )
    .await
    {
        Ok(rid) => rid,
        Err(_) => {
            // Agent not running, start new one
            let new_run_id = start_teammate_agent_only(
                app.clone(),
                db.clone(),
                registry.clone(),
                project_id.clone(),
                target_agent_id.clone(),
            )
            .await?;

            // Wait a bit for the agent to initialize
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

            // Send the message
            send_to_teammate(new_run_id.clone(), content.clone(), registry.clone()).await?;

            new_run_id
        }
    };

    // Update project_agent_id in database
    {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE project_agents SET project_agent_id = ?1 WHERE project_id = ?2 AND agent_id = ?3",
            params![run_id, project_id, target_agent_id],
        )
        .map_err(|e| e.to_string())?;
    }

    // Save message to database
    let message = Message {
        id: Uuid::new_v4().to_string(),
        project_id: project_id.clone(),
        sender_id: sender,
        sender_name,
        target_id: target_agent_id,
        target_name: target_agent_name,
        content: content.clone(),
        message_type: "user".to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };

    {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO messages (id, project_id, sender_id, sender_name, target_id, target_name, content, message_type, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                message.id,
                message.project_id,
                message.sender_id,
                message.sender_name,
                message.target_id,
                message.target_name,
                message.content,
                message.message_type,
                message.created_at,
            ],
        )
        .map_err(|e| e.to_string())?;
    }

    info!("Message sent successfully: {}", message.id);

    Ok(message)
}

/// Get messages for a project
#[tauri::command]
pub async fn get_messages(
    project_id: String,
    db: State<'_, AgentDb>,
) -> Result<Vec<Message>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;

    let mut stmt = conn
        .prepare(
            "SELECT id, project_id, sender_id, sender_name, target_id, target_name, content, message_type, created_at
             FROM messages
             WHERE project_id = ?1
             ORDER BY created_at ASC",
        )
        .map_err(|e| e.to_string())?;

    let messages = stmt
        .query_map(params![project_id], |row| {
            Ok(Message {
                id: row.get(0)?,
                project_id: row.get(1)?,
                sender_id: row.get(2)?,
                sender_name: row.get(3)?,
                target_id: row.get(4)?,
                target_name: row.get(5)?,
                content: row.get(6)?,
                message_type: row.get(7)?,
                created_at: row.get(8)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(messages)
}

/// Save a message response from an agent
#[tauri::command]
pub async fn save_message_response(
    project_id: String,
    run_id: String,
    content: String,
    message_type: String,
    db: State<'_, AgentDb>,
) -> Result<Message, String> {
    // Find agent by run_id in project_agents
    let (agent_id, agent_name) = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;

        let mut stmt = conn
            .prepare(
                "SELECT pa.agent_id, a.name
                 FROM project_agents pa
                 INNER JOIN agents a ON pa.agent_id = a.id
                 WHERE pa.project_id = ?1 AND pa.project_agent_id = ?2",
            )
            .map_err(|e| e.to_string())?;

        stmt
            .query_row(params![project_id, run_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("Agent not found for run_id: {}", e))?
    };

    let message = Message {
        id: Uuid::new_v4().to_string(),
        project_id: project_id.clone(),
        sender_id: agent_id,
        sender_name: agent_name,
        target_id: "user".to_string(),
        target_name: Some("You".to_string()),
        content,
        message_type,
        created_at: chrono::Utc::now().to_rfc3339(),
    };

    {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO messages (id, project_id, sender_id, sender_name, target_id, target_name, content, message_type, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                message.id,
                message.project_id,
                message.sender_id,
                message.sender_name,
                message.target_id,
                message.target_name,
                message.content,
                message.message_type,
                message.created_at,
            ],
        )
        .map_err(|e| e.to_string())?;
    }

    Ok(message)
}
