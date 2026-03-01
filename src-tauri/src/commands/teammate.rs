use log::{error, info, warn};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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

/// Member status stored in memory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberStatus {
    pub agent_id: String,
    pub run_id: Option<String>,
    pub status: String, // "pending", "running", "completed", "stopped", "error"
}

/// Update member status in memory and emit event
fn update_member_status(
    app: &AppHandle,
    project_id: &str,
    agent_id: &str,
    run_id: Option<String>,
    status: &str,
) {
    let mut statuses = MEMBER_STATUS.lock().unwrap();
    let project_statuses = statuses.entry(project_id.to_string()).or_insert_with(HashMap::new);

    project_statuses.insert(
        agent_id.to_string(),
        MemberStatus {
            agent_id: agent_id.to_string(),
            run_id,
            status: status.to_string(),
        },
    );

    // Emit event to frontend
    let _ = app.emit(&format!("member-status-update:{}", project_id), ());
    let _ = app.emit("member-status-update", project_id);
}

// Global member status storage (lazily initialized)
lazy_static::lazy_static! {
    static ref MEMBER_STATUS: Arc<Mutex<HashMap<String, HashMap<String, MemberStatus>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    // Map run_id to (project_id, agent_id)
    static ref RUN_ID_MAPPING: Arc<Mutex<HashMap<String, (String, String)>>> =
        Arc::new(Mutex::new(HashMap::new()));
}

/// Save run_id mapping for later lookup
fn save_run_id_mapping(run_id: &str, project_id: &str, agent_id: &str) {
    let mut mapping = RUN_ID_MAPPING.lock().unwrap();
    mapping.insert(run_id.to_string(), (project_id.to_string(), agent_id.to_string()));
}

/// Get project_id and agent_id from run_id
fn get_run_id_mapping(run_id: &str) -> Option<(String, String)> {
    let mapping = RUN_ID_MAPPING.lock().unwrap();
    mapping.get(run_id).cloned()
}

/// Remove run_id mapping
fn remove_run_id_mapping(run_id: &str) {
    let mut mapping = RUN_ID_MAPPING.lock().unwrap();
    mapping.remove(run_id);
}

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

/// Extract all messages from output (thinking and response)
fn extract_all_messages(output: &str) -> Vec<(String, String)> {
    // message_type can be: "thinking", "result", "response"
    let mut messages = Vec::new();

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
            // Check for "type" field to determine message type
            let msg_type = json.get("type").and_then(|v| v.as_str()).unwrap_or("response");

            // Extract content based on message type
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
                    let message_type = if msg_type == "thinking" { "thinking" } else { "response" };
                    messages.push((message_type.to_string(), text));
                }
            }
        }
    }

    messages
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

    // Clone db for message middleware
    let db_for_output = db.0.clone();

    // Spawn stdout reader
    let stdout_task = tokio::spawn(async move {
        let stdout_reader = TokioBufReader::new(stdout);
        let mut lines = stdout_reader.lines();

        while let Ok(Some(line)) = lines.next_line().await {
            info!("Teammate stdout: {}", line);

            // Store live output
            let _ = registry_clone.append_live_output(run_id_clone.clone(), &line);

            // 只使用消息中间件传递消息，不直接emit， 也不需要过滤消息，是否过滤消息由消息中间件判断
            // Emit to frontend (skip init messages - they don't need to be displayed but are saved to DB)
            // let should_emit = if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) {
            //     !(msg["type"] == "system" && msg["subtype"] == "init")
            // } else {
            //     true
            // };
            //
            // if should_emit {
            //     let _ = app_handle.emit(&format!("teammate-output:{}", run_id_clone), &line);
            //     let _ = app_handle.emit("teammate-output", &line);
            // }

            // Process teammate output through MessageMiddleware
            // This handles: parsing, saving to DB, @mention forwarding, emitting to frontend
            let db_for_output_clone = db_for_output.clone();
            let registry_for_middleware = registry_arc.clone();
            let project_path_clone = project_path_for_middleware.clone();
            let run_id_for_middleware = run_id_clone.clone();
            let project_id_for_mw = project_id_for_middleware.clone();
            let app_handle_clone = app_handle.clone();

            tokio::spawn(async move {
                // Create middleware
                let middleware = crate::commands::message_middleware::MessageMiddleware::new(
                    db_for_output_clone,
                    registry_for_middleware,
                );

                // Handle outgoing message
                match middleware.handle_outgoing(
                    app_handle_clone,
                    run_id_for_middleware.clone(),
                    project_id_for_mw.clone(),
                    line,
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
            agent_id_clone.clone(),
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

    // Update member status to running and emit event
    update_member_status(&app, &project_id, &agent_id, Some(run_id.clone()), "running");

    // Save run_id mapping for later lookup (when stopping/completing)
    save_run_id_mapping(&run_id, &project_id, &agent_id);

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

        // [已注释] 消息保存已由 MessageMiddleware.handle_outgoing() 实时处理，不再需要批量保存
        // // Get live output and save all messages (thinking and response) to database
        // if let Ok(live_output) = registry_monitor.get_live_output(run_id_monitor.clone()) {
        //     info!("Agent output length: {} chars", live_output.len());
        //
        //     // Extract all messages (thinking and response)
        //     let all_messages = extract_all_messages(&live_output);
        //     if !all_messages.is_empty() {
        //         info!("Found {} messages to save", all_messages.len());
        //
        //         // Clone values needed for spawn_blocking
        //         let db_clone = db_arc.clone();
        //         let project_id_for_db = project_id_for_event.clone();
        //         let run_id_for_db = run_id_monitor.clone();
        //         let app_clone = app_handle_monitor.clone();
        //         let project_id_for_emit = project_id_for_event.clone();
        //         let json_content = live_output.clone(); // Save raw json output
        //
        //         let result: Result<Result<usize, String>, tokio::task::JoinError> = tokio::task::spawn_blocking(move || {
        //             let mut saved_count = 0;
        //             for (msg_type, content) in all_messages {
        //                 match save_message_response_internal(
        //                     &db_clone,
        //                     &project_id_for_db,
        //                     &run_id_for_db,
        //                     &content,
        //                     &json_content,
        //                     &msg_type,
        //                 ) {
        //                     Ok(msg) => {
        //                         info!("Saved {} message: {}", msg_type, msg.id);
        //                         saved_count += 1;
        //                     }
        //                     Err(e) => {
        //                         error!("Failed to save {} message: {}", msg_type, e);
        //                     }
        //                 }
        //             }
        //             Ok(saved_count)
        //         }).await;
        //
        //         match result {
        //             Ok(Ok(count)) => {
        //                 info!("Saved {} messages to database", count);
        //                 // Emit message update event
        //                 let _ = app_clone.emit("project-message-update", &project_id_for_emit);
        //             }
        //             Ok(Err(e)) => {
        //                 error!("Failed to save messages: {}", e);
        //             }
        //             Err(e) => {
        //                 error!("Failed to spawn blocking task: {}", e);
        //             }
        //         }
        //     }
        // }

        // Emit completion event with project_id for frontend to refresh messages
        let _ = app_handle_monitor.emit(&format!("teammate-complete:{}", run_id_monitor), true);
        let _ = app_handle_monitor.emit("teammate-complete", true);
        let _ = app_handle_monitor.emit("project-message-update", &project_id_for_event);

        // Update member status to completed and emit event
        if let Some((project_id, agent_id)) = get_run_id_mapping(&run_id_monitor) {
            update_member_status(&app_handle_monitor, &project_id, &agent_id, Some(run_id_monitor.clone()), "completed");
            // Remove mapping
            remove_run_id_mapping(&run_id_monitor);
        }

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
    app: AppHandle,
    registry: State<'_, ProcessRegistryState>,
) -> Result<bool, String> {
    info!("Stopping teammate agent: {}", run_id);

    let result = registry.0.kill_process(run_id.clone()).await?;

    if result {
        info!("Successfully stopped teammate agent: {}", run_id);
        // Update member status to stopped and emit event
        if let Some((project_id, agent_id)) = get_run_id_mapping(&run_id) {
            update_member_status(&app, &project_id, &agent_id, Some(run_id.clone()), "stopped");
            remove_run_id_mapping(&run_id);
        }
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
    pub status: String, // "pending", "running", "completed", "stopped", "error"
}

/// Get the status of all project members' processes
#[tauri::command]
pub async fn get_project_member_statuses(
    project_id: String,
    db: State<'_, AgentDb>,
    registry: State<'_, ProcessRegistryState>,
) -> Result<Vec<MemberProcessStatus>, String> {
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

    // Get statuses from memory
    let mem_statuses = MEMBER_STATUS.lock().unwrap();
    let project_mem_statuses = mem_statuses.get(&project_id);

    // Check each member's process status
    let mut result: Vec<MemberProcessStatus> = Vec::new();
    for (agent_id, stored_run_id) in member_statuses {
        let run_id = stored_run_id;

        // First check memory status
        if let Some(project_statuses) = project_mem_statuses {
            if let Some(mem_status) = project_statuses.get(&agent_id) {
                // If we have a valid run_id and the status is still valid (running exists in registry)
                if mem_status.run_id.as_ref() == Some(&run_id) {
                    if mem_status.status == "running" {
                        // Verify process is still actually running
                        if registry.0.exists(&run_id).unwrap_or(false) {
                            result.push(MemberProcessStatus {
                                agent_id,
                                run_id: Some(run_id),
                                status: "running".to_string(),
                            });
                            continue;
                        } else {
                            // Process died unexpectedly - mark as error
                            result.push(MemberProcessStatus {
                                agent_id,
                                run_id: Some(run_id),
                                status: "error".to_string(),
                            });
                            continue;
                        }
                    } else {
                        // Return the stored status (completed, stopped, error)
                        result.push(MemberProcessStatus {
                            agent_id,
                            run_id: Some(run_id),
                            status: mem_status.status.clone(),
                        });
                        continue;
                    }
                }
            }
        }

        // Default: not started yet
        result.push(MemberProcessStatus {
            agent_id,
            run_id: Some(run_id),
            status: "pending".to_string(),
        });
    }

    Ok(result)
}
