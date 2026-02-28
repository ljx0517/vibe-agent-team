use log::{error, info, warn};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::process::Stdio;
use tauri::{AppHandle, Emitter, State};
use tokio::io::{AsyncBufReadExt, BufReader as TokioBufReader};
use tokio::process::Command;
use uuid::Uuid;

use crate::claude_binary::find_claude_binary;
use crate::commands::agents::{get_agent, AgentDb};
use crate::commands::message::save_message_response_internal;
use crate::process::ProcessRegistryState;

/// Extract the final result from Claude's JSON output
fn extract_result_from_output(output: &str) -> Option<String> {
    // Look for the last JSON line with "result" field
    for line in output.lines().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Try to parse as JSON and extract "result" field
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(result) = json.get("result").and_then(|v| v.as_str()) {
                if !result.is_empty() {
                    return Some(result.to_string());
                }
            }
            // Also check for "text" field in message content
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
        }
    }
    // Fallback: return last non-empty line
    output.lines().rev().find(|l| !l.trim().is_empty()).map(|s| s.to_string())
}

/// Get agent info by run_id (project_agent id)
fn get_agent_info_by_run_id(
    conn: &rusqlite::Connection,
    project_id: &str,
    run_id: &str,
) -> Result<(String, String), String> {
    let mut stmt = conn
        .prepare(
            "SELECT pa.agent_id, a.name
             FROM project_agents pa
             INNER JOIN agents a ON pa.agent_id = a.id
             WHERE pa.project_id = ?1 AND pa.id = ?2",
        )
        .map_err(|e| e.to_string())?;

    stmt.query_row(params![project_id, run_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })
    .map_err(|e| format!("Agent not found for run_id: {}", e))
}

/// Agent settings parsed from JSON
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentSettings {
    /// Maximum number of agentic turns before the subagent stops
    #[serde(default)]
    pub max_turns: Option<u32>,
    /// Background mode - run as background task
    #[serde(default)]
    pub background: Option<bool>,
    /// Memory scope: "user", "project", or "local"
    #[serde(default)]
    pub memory: Option<String>,
    /// Custom skills to preload
    #[serde(default)]
    pub skills: Option<Vec<String>>,
    /// MCP servers available to this agent
    #[serde(default)]
    pub mcp_servers: Option<serde_json::Value>,
}

/// Hook configuration parsed from JSON
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentHooks {
    /// PreToolUse hooks
    #[serde(default)]
    pub pre_tool_use: Option<Vec<HookConfig>>,
    /// PostToolUse hooks
    #[serde(default)]
    pub post_tool_use: Option<Vec<HookConfig>>,
    /// Stop hook
    #[serde(default)]
    pub stop: Option<Vec<HookConfig>>,
}

/// Hook configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookConfig {
    /// Event matcher
    #[serde(default)]
    pub matcher: Option<String>,
    /// Hooks to run
    #[serde(default)]
    pub hooks: Vec<HookDefinition>,
}

/// Hook definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookDefinition {
    /// Hook type: "command"
    #[serde(default)]
    pub r#type: Option<String>,
    /// Command to run
    #[serde(default)]
    pub command: Option<String>,
}

/// Represents a running teammate agent session
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TeammateSession {
    pub run_id: String,
    pub agent_id: String,
    pub agent_name: String,
    pub project_path: String,
    pub model: String,
    pub status: String, // 'running', 'stopped'
}

/// Determine permission mode based on agent configuration
fn get_permission_mode(
    enable_file_read: bool,
    enable_file_write: bool,
    enable_network: bool,
    skip_permissions: bool,
) -> String {
    if skip_permissions {
        "bypassPermissions".to_string()
    } else if enable_file_write {
        "acceptEdits".to_string()
    } else if enable_file_read && !enable_network {
        "plan".to_string()
    } else {
        "default".to_string()
    }
}

/// Build disallowed tools list based on permissions
fn get_disallowed_tools(
    enable_file_read: bool,
    enable_file_write: bool,
    enable_network: bool,
) -> Vec<String> {
    let mut disallowed = Vec::new();

    if !enable_file_read {
        disallowed.push("Read".to_string());
    }
    if !enable_file_write {
        disallowed.push("Write".to_string());
        disallowed.push("Edit".to_string());
        disallowed.push("Create".to_string());
        disallowed.push("Delete".to_string());
    }
    if !enable_network {
        disallowed.push("Bash".to_string());
        disallowed.push("WebFetch".to_string());
        disallowed.push("WebSearch".to_string());
    }

    disallowed
}

/// Parse agent settings from JSON string
fn parse_agent_settings(settings_json: &Option<String>) -> AgentSettings {
    match settings_json {
        Some(json) => serde_json::from_str(json).unwrap_or_default(),
        None => AgentSettings::default(),
    }
}

/// Parse agent hooks from JSON string
fn parse_agent_hooks(hooks_json: &Option<String>) -> AgentHooks {
    match hooks_json {
        Some(json) => serde_json::from_str(json).unwrap_or_default(),
        None => AgentHooks::default(),
    }
}

/// Find and return the claude binary path
fn find_claude_bin(app_handle: &AppHandle) -> Result<String, String> {
    find_claude_binary(app_handle)
}

/// Create a command with proper environment variables
fn create_command_with_env(program: &str) -> Command {
    let mut cmd = Command::new(program);

    // Copy environment variables
    for (key, value) in std::env::vars() {
        if key == "PATH"
            || key == "HOME"
            || key == "USER"
            || key == "SHELL"
            || key == "LANG"
            || key == "LC_ALL"
            || key.starts_with("LC_")
            || key == "NODE_PATH"
            || key == "NVM_DIR"
            || key == "NVM_BIN"
            || key == "HOMEBREW_PREFIX"
            || key == "HOMEBREW_CELLAR"
        {
            cmd.env(&key, &value);
        }
    }

    // Add NVM support
    if program.contains("/.nvm/versions/node/") {
        if let Some(node_bin_dir) = std::path::Path::new(program).parent() {
            let current_path = std::env::var("PATH").unwrap_or_default();
            let node_bin_str = node_bin_dir.to_string_lossy();
            if !current_path.contains(&node_bin_str.as_ref()) {
                let new_path = format!("{}:{}", node_bin_str, current_path);
                cmd.env("PATH", new_path);
            }
        }
    }

    // Ensure PATH contains common Homebrew locations
    if let Ok(existing_path) = std::env::var("PATH") {
        let mut paths: Vec<&str> = existing_path.split(':').collect();
        for p in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"].iter() {
            if !paths.contains(p) {
                paths.push(p);
            }
        }
        let joined = paths.join(":");
        cmd.env("PATH", joined);
    } else {
        cmd.env("PATH", "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin");
    }

    cmd
}

/// Start a teammate agent process
#[tauri::command]
pub async fn start_teammate_agent(
    app: AppHandle,
    agent_id: String,
    project_path: String,
    project_id: String,
    model: Option<String>,
    run_id: Option<String>,
    db: State<'_, AgentDb>,
    registry: State<'_, ProcessRegistryState>,
) -> Result<String, String> {
    info!(
        "Starting teammate agent: {} in project: {}",
        agent_id, project_path
    );

    // Get agent from database
    let agent = get_agent(db.clone(), agent_id.clone()).await?;

    // Determine model to use
    let execution_model = model.unwrap_or_else(|| agent.model.clone());

    // Parse settings and hooks
    let settings = parse_agent_settings(&agent.settings);
    let hooks = parse_agent_hooks(&agent.hooks);

    // Use provided run_id or generate new one
    let run_id = run_id.unwrap_or_else(|| Uuid::new_v4().to_string());

    // Determine if we should skip permissions (for backwards compatibility, default to true)
    let skip_permissions = true;

    // Get permission mode based on agent configuration
    let permission_mode = get_permission_mode(
        agent.enable_file_read,
        agent.enable_file_write,
        agent.enable_network,
        skip_permissions,
    );

    // Get disallowed tools based on permissions
    let disallowed_tools = get_disallowed_tools(
        agent.enable_file_read,
        agent.enable_file_write,
        agent.enable_network,
    );

    // Get tools list from agent config
    let tools: Vec<String> = agent
        .tools
        .as_ref()
        .and_then(|t| serde_json::from_str::<Vec<String>>(t).ok())
        .unwrap_or_default();

    // Build agents JSON config for --agents flag (Claude Code Subagent format)
    let mut agent_config = serde_json::json!({
        "description": agent.default_task.clone().unwrap_or_else(|| format!("Teammate agent: {}", agent.name)),
        "prompt": agent.system_prompt,
        "tools": tools,
        "model": execution_model.clone(),
        "permissionMode": permission_mode,
    });

    // Add disallowed tools if any
    if !disallowed_tools.is_empty() {
        agent_config["disallowedTools"] = serde_json::json!(disallowed_tools);
    }

    // Add maxTurns if specified
    if let Some(max_turns) = settings.max_turns {
        agent_config["maxTurns"] = serde_json::json!(max_turns);
    }

    // Add background mode if specified
    if let Some(background) = settings.background {
        agent_config["background"] = serde_json::json!(background);
    }

    // Add memory scope if specified
    if let Some(memory) = settings.memory {
        agent_config["memory"] = serde_json::json!(memory);
    }

    // Add skills if specified
    if let Some(skills) = settings.skills {
        agent_config["skills"] = serde_json::json!(skills);
    }

    // Add MCP servers if specified
    if let Some(mcp_servers) = settings.mcp_servers {
        agent_config["mcpServers"] = mcp_servers;
    }

    // Add hooks if specified
    let has_hooks = hooks.pre_tool_use.as_ref().map_or(false, |v| !v.is_empty())
        || hooks.post_tool_use.as_ref().map_or(false, |v| !v.is_empty())
        || hooks.stop.is_some();

    if has_hooks {
        let mut hooks_config = serde_json::json!({});
        let mut has_any_hook = false;

        if let Some(pre_tool_use) = hooks.pre_tool_use {
            if !pre_tool_use.is_empty() {
                hooks_config["PreToolUse"] = serde_json::json!(pre_tool_use);
                has_any_hook = true;
            }
        }
        if let Some(post_tool_use) = hooks.post_tool_use {
            if !post_tool_use.is_empty() {
                hooks_config["PostToolUse"] = serde_json::json!(post_tool_use);
                has_any_hook = true;
            }
        }
        if let Some(stop) = hooks.stop {
            if !stop.is_empty() {
                hooks_config["Stop"] = serde_json::json!(stop);
                has_any_hook = true;
            }
        }

        if has_any_hook {
            agent_config["hooks"] = hooks_config;
        }
    }

    let agents_json = serde_json::json!({
        &agent_id: agent_config
    });

    // Find Claude binary
    let claude_path = match find_claude_bin(&app) {
        Ok(path) => path,
        Err(e) => {
            error!("Failed to find claude binary: {}", e);
            return Err(e);
        }
    };

    // Build command arguments
    // Note: We don't use -p (task) because we want to wait for stdin input
    let mut args = vec![
        "--agents".to_string(),
        agents_json.to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--verbose".to_string(),
    ];

    // Only add skip-permissions flag if permissionMode is bypassPermissions
    if permission_mode == "bypassPermissions" {
        args.push("--dangerously-skip-permissions".to_string());
    }

    info!("Claude args: {:?}", args);

    // Create and spawn the process
    let mut cmd = create_command_with_env(&claude_path);

    // Add environment variables for Claude Code
    if std::env::var("API_TIMEOUT_MS").is_err() {
        cmd.env("API_TIMEOUT_MS", "600000");
    }
    if std::env::var("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC").is_err() {
        cmd.env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1");
    }
    if std::env::var("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS").is_err() {
        cmd.env("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS", "1");
    }

    // Configure process
    cmd.args(&args)
        .current_dir(&project_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // Spawn the process
    info!("Spawning teammate agent process...");
    let mut child = cmd.spawn().map_err(|e| {
        error!("Failed to spawn Claude process: {}", e);
        format!("Failed to spawn Claude: {}", e)
    })?;

    // Get stdout, stderr, stdin
    let stdout = child.stdout.take().ok_or("Failed to get stdout")?;
    let stderr = child.stderr.take().ok_or("Failed to get stderr")?;
    let stdin = child.stdin.take().ok_or("Failed to get stdin")?;

    // Get PID
    let pid = child.id().unwrap_or(0);
    info!("Teammate agent process spawned with PID: {}", pid);

    // Clone variables for async tasks
    let app_handle = app.clone();
    let registry_clone = registry.0.clone();
    let run_id_clone = run_id.clone();
    let project_path_clone = project_path.clone();
    let agent_id_clone = agent_id.clone();
    let agent_name_clone = agent.name.clone();
    let model_clone = execution_model.clone();

    // Clone registry for middleware
    let registry_arc = registry.0.clone();
    let project_path_for_middleware = project_path.clone();
    let agent_id_for_middleware = agent_id.clone();
    let project_id_for_middleware = project_id.clone();

    // Spawn stdout reader
    let stdout_task = tokio::spawn(async move {
        let stdout_reader = TokioBufReader::new(stdout);
        let mut lines = stdout_reader.lines();

        while let Ok(Some(line)) = lines.next_line().await {
            info!("Teammate stdout: {}", line);

            // Store live output
            let _ = registry_clone.append_live_output(run_id_clone.clone(), &line);

            // Emit to frontend
            let _ = app_handle.emit(&format!("teammate-output:{}", run_id_clone), &line);
            let _ = app_handle.emit("teammate-output", &line);

            // Process teammate output for @mentions and forward
            let registry_for_middleware = registry_arc.clone();
            let project_path_clone = project_path_for_middleware.clone();
            let run_id_for_middleware = run_id_clone.clone();
            let agent_id_for_mw = agent_id_for_middleware.clone();
            let project_id_for_mw = project_id_for_middleware.clone();

            tokio::spawn(async move {
                match crate::commands::message_middleware::process_teammate_output_by_path(
                    &run_id_for_middleware,
                    &project_path_clone,
                    &agent_id_for_mw,
                    &project_id_for_mw,
                    &line,
                    registry_for_middleware,
                ).await {
                    Ok(_) => {}
                    Err(e) => {
                        log::debug!("Message middleware error (non-fatal): {}", e);
                    }
                }
            });
        }

        info!("Teammate stdout reader finished");
    });

    // Spawn stderr reader
    let app_handle_stderr = app.clone();
    let run_id_stderr = run_id.clone();
    let stderr_task = tokio::spawn(async move {
        let stderr_reader = TokioBufReader::new(stderr);
        let mut lines = stderr_reader.lines();

        while let Ok(Some(line)) = lines.next_line().await {
            error!("Teammate stderr: {}", line);

            // Emit error to frontend
            let _ = app_handle_stderr.emit(&format!("teammate-error:{}", run_id_stderr), &line);
            let _ = app_handle_stderr.emit("teammate-error", &line);
        }

        info!("Teammate stderr reader finished");
    });

    // Register in process registry
    registry
        .0
        .register_teammate_agent(
            run_id.clone(),
            agent_id_clone,
            agent_name_clone,
            pid,
            project_path_clone,
            String::new(), // No initial task
            model_clone,
            child,
            stdin,
        )
        .map_err(|e| format!("Failed to register teammate agent: {}", e))?;

    info!("Registered teammate agent with run_id: {}", run_id);

    // Clone db for saving messages (Arc<Mutex<Connection>> can be cloned)
    let db_arc = db.0.clone();

    // Spawn monitoring task
    let app_handle_monitor = app.clone();
    let registry_monitor = registry.0.clone();
    let run_id_monitor = run_id.clone();
    let project_id_for_event = project_id.clone();
    tokio::spawn(async move {
        // Wait for stdout/stderr tasks to complete
        let _ = stdout_task.await;
        let _ = stderr_task.await;

        info!("Teammate agent {} finished", run_id_monitor);

        // Get live output and save response to database
        if let Ok(live_output) = registry_monitor.get_live_output(run_id_monitor.clone()) {
            info!("Agent output length: {} chars", live_output.len());

            // Extract response content
            if let Some(response_content) = extract_result_from_output(&live_output) {
                if !response_content.trim().is_empty() {
                    info!("Saving agent response to database: {} chars", response_content.len());

                    // Clone values needed for spawn_blocking
                    let db_clone = db_arc.clone();
                    let project_id_for_db = project_id_for_event.clone();
                    let run_id_for_db = run_id_monitor.clone();
                    let app_clone = app_handle_monitor.clone();
                    let project_id_for_emit = project_id_for_event.clone();
                    let json_content = live_output.clone(); // Save raw json output

                    let result = tokio::task::spawn_blocking(move || {
                        save_message_response_internal(
                            &db_clone,
                            &project_id_for_db,
                            &run_id_for_db,
                            &response_content,
                            &json_content,
                            "response",
                        )
                    }).await;

                    match result {
                        Ok(Ok(msg)) => {
                            info!("Agent response saved: {}", msg.id);
                            // Emit message update event
                            let _ = app_clone.emit("project-message-update", &project_id_for_emit);
                        }
                        Ok(Err(e)) => {
                            error!("Failed to save agent response: {}", e);
                        }
                        Err(e) => {
                            error!("Failed to spawn blocking task: {}", e);
                        }
                    }
                }
            }
        }

        // Emit completion event with project_id for frontend to refresh messages
        let _ = app_handle_monitor.emit(&format!("teammate-complete:{}", run_id_monitor), true);
        let _ = app_handle_monitor.emit("teammate-complete", true);
        let _ = app_handle_monitor.emit("project-message-update", &project_id_for_event);

        // Unregister from registry
        let _ = registry_monitor.unregister_process(run_id_monitor);
    });

    Ok(run_id)
}

/// Send a message to a running teammate agent
#[tauri::command]
pub async fn send_to_teammate(
    run_id: String,
    message: String,
    registry: State<'_, ProcessRegistryState>,
) -> Result<(), String> {
    info!("Sending message to teammate agent: {}", run_id);

    registry
        .0
        .send_to_process_async(&run_id, &message)
        .await
}

/// Stop a running teammate agent
#[tauri::command]
pub async fn stop_teammate_agent(
    run_id: String,
    registry: State<'_, ProcessRegistryState>,
) -> Result<bool, String> {
    info!("Stopping teammate agent: {}", run_id);

    let result = registry.0.kill_process(run_id.clone()).await?;

    if result {
        info!("Successfully stopped teammate agent: {}", run_id);
    } else {
        warn!("Failed to stop teammate agent: {}", run_id);
    }

    Ok(result)
}

/// Get the status of a teammate agent
#[tauri::command]
pub async fn get_teammate_status(
    run_id: String,
    registry: State<'_, ProcessRegistryState>,
) -> Result<Option<String>, String> {
    if registry.0.exists(&run_id)? {
        Ok(Some("running".to_string()))
    } else {
        Ok(None)
    }
}

/// Status of a project member's process
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberProcessStatus {
    pub agent_id: String,
    pub run_id: Option<String>,
    pub status: String, // "stopped", "running", "error"
}

/// Get the status of all project members' processes
#[tauri::command]
pub async fn get_project_member_statuses(
    project_id: String,
    db: State<'_, AgentDb>,
    registry: State<'_, ProcessRegistryState>,
) -> Result<Vec<MemberProcessStatus>, String> {
    // Get project path
    let project_path = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT working_dir FROM projects WHERE id = ?1")
            .map_err(|e| e.to_string())?;
        stmt.query_row([&project_id], |row| row.get::<_, String>(0))
            .map_err(|e| format!("Project not found: {}", e))?
    };

    // Get all project members with their run_ids (now uses pa.id)
    let member_statuses = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT pa.agent_id, pa.id
                 FROM project_agents pa
                 WHERE pa.project_id = ?1"
            )
            .map_err(|e| e.to_string())?;

        let members = stmt
            .query_map([&project_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        members
    };

    // Check each member's process status
    let mut result: Vec<MemberProcessStatus> = Vec::new();
    for (agent_id, stored_run_id) in member_statuses {
        // stored_run_id is now pa.id (the run_id)
        let run_id = stored_run_id;
        let status = if registry.0.exists(&run_id).unwrap_or(false) {
            // Check if there's an error state (process might have died)
            if let Ok(Some(_info)) = registry.0.get_process(run_id.clone()) {
                // Process is running
                MemberProcessStatus {
                    agent_id,
                    run_id: Some(run_id),
                    status: "running".to_string(),
                }
            } else {
                // Process info not found
                MemberProcessStatus {
                    agent_id,
                    run_id: Some(run_id),
                    status: "running".to_string(),
                }
            }
        } else {
            // Process was stored but not running - might have crashed
            // Still return the stored run_id (pa.id) so frontend can use it
            MemberProcessStatus {
                agent_id,
                run_id: Some(run_id),
                status: "stopped".to_string(),
            }
        };
        result.push(status);
    }

    Ok(result)
}
