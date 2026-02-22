use axum::extract::ws::{Message, WebSocket};
use axum::http::Method;
use axum::{
    extract::{Path, State as AxumState, WebSocketUpgrade},
    response::{Html, Json, Response},
    routing::get,
    Router,
};
use chrono;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use which;

use crate::commands::agents::AgentDb;
use crate::commands;
use crate::process;

// Find Claude binary for web mode - use bundled binary first
fn find_claude_binary_web() -> Result<String, String> {
    // First try the bundled binary (same location as Tauri app uses)
    let bundled_binary = "src-tauri/binaries/claude-code-x86_64-unknown-linux-gnu";
    if std::path::Path::new(bundled_binary).exists() {
        println!(
            "[find_claude_binary_web] Using bundled binary: {}",
            bundled_binary
        );
        return Ok(bundled_binary.to_string());
    }

    // Fall back to system installation paths
    let home_path = format!(
        "{}/.local/bin/claude",
        std::env::var("HOME").unwrap_or_default()
    );
    let candidates = vec![
        "claude",
        "claude-code",
        "/usr/local/bin/claude",
        "/usr/bin/claude",
        "/opt/homebrew/bin/claude",
        &home_path,
    ];

    for candidate in candidates {
        if which::which(candidate).is_ok() {
            println!(
                "[find_claude_binary_web] Using system binary: {}",
                candidate
            );
            return Ok(candidate.to_string());
        }
    }

    Err("Claude binary not found in bundled location or system paths".to_string())
}

#[derive(Clone)]
pub struct AppState {
    // Track active WebSocket sessions for Claude execution
    pub active_sessions:
        Arc<Mutex<std::collections::HashMap<String, tokio::sync::mpsc::Sender<String>>>>,
    // Process registry for managing Claude processes
    pub process_registry: Arc<process::ProcessRegistry>,
    // Database for agents (Arc-wrapped for sharing across requests)
    pub db: Arc<AgentDb>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ClaudeWebSocketMessage {
    /// 启动新进程
    Execute {
        project_path: String,
        prompt: String,
        model: Option<String>,
    },
    /// 发送输入到现有进程
    Send {
        content: String,
    },
    /// 终止进程
    Exit,
    /// 继续对话
    Continue {
        project_path: String,
        prompt: String,
        model: Option<String>,
    },
    /// 恢复会话
    Resume {
        project_path: String,
        session_id: String,
        prompt: String,
        model: Option<String>,
    },
}

#[derive(Deserialize)]
pub struct QueryParams {
    #[serde(default)]
    pub project_path: Option<String>,
}

#[derive(Serialize)]
pub struct ApiResponse<T> {
    pub success: bool,
    pub data: Option<T>,
    pub error: Option<String>,
}

impl<T> ApiResponse<T> {
    pub fn success(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn error(error: String) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(error),
        }
    }
}

/// Check if running in dev mode (Vite dev server is available)
/// Check if running in dev mode by detecting Vite dev server
fn is_dev_mode() -> bool {
    // Check environment variable first
    if std::env::var("TAURI_DEV").is_ok() || std::env::var("RUST_DEV").is_ok() {
        return true;
    }

    // Also auto-detect if Vite dev server is running on port 1420
    std::net::TcpStream::connect("localhost:1420").is_ok()
}

/// Serve the React frontend - either from Vite dev server or embedded HTML
async fn serve_frontend() -> Html<String> {
    if is_dev_mode() {
        // In dev mode, fetch from Vite dev server
        match reqwest::get("http://localhost:1420").await {
            Ok(response) => {
                if let Ok(html) = response.text().await {
                    println!("[serve_frontend] Fetched from Vite dev server");
                    return Html(html);
                }
            }
            Err(e) => {
                println!("[serve_frontend] Failed to fetch from Vite: {}", e);
            }
        }
        // If Vite fetch fails, return a message to start Vite
        return Html(r#"<!DOCTYPE html>
<html>
<head><title>Dev Mode</title></head>
<body>
<h1>Start Vite dev server</h1>
<p>Run <code>bun run dev:front</code> to start the frontend dev server.</p>
</body>
</html>"#.to_string());
    }

    // Production mode: use embedded HTML (requires dist/index.html to exist)
    #[cfg(not(debug_assertions))]
    Html(include_str!("../../dist/index.html").to_string());

    // In debug builds, try to load from file if embedded not available
    #[cfg(debug_assertions)]
    {
        let dist_path = std::path::Path::new("../../dist/index.html");
        if dist_path.exists() {
            std::fs::read_to_string(dist_path)
                .map(Html)
                .unwrap_or_else(|_| Html("<h1>Error loading index.html</h1>".to_string()))
        } else {
            Html("<h1>Build frontend first</h1><p>Run bun run build</p>".to_string())
        }
    }
}

/// API endpoint to get projects (equivalent to Tauri command)
async fn get_projects() -> Json<ApiResponse<Vec<commands::claude::Project>>> {
    match commands::claude::list_projects().await {
        Ok(projects) => Json(ApiResponse::success(projects)),
        Err(e) => Json(ApiResponse::error(e.to_string())),
    }
}

/// API endpoint to get sessions for a project
async fn get_sessions(
    Path(project_id): Path<String>,
) -> Json<ApiResponse<Vec<commands::claude::Session>>> {
    match commands::claude::get_project_sessions(project_id).await {
        Ok(sessions) => Json(ApiResponse::success(sessions)),
        Err(e) => Json(ApiResponse::error(e.to_string())),
    }
}

/// Simple agents endpoint - return empty for now (needs DB state)
async fn get_agents() -> Json<ApiResponse<Vec<serde_json::Value>>> {
    Json(ApiResponse::success(vec![]))
}

/// Teamleads endpoint - fetch teamleads from database
async fn get_teamleads(AxumState(state): AxumState<AppState>) -> Json<ApiResponse<Vec<serde_json::Value>>> {
    let db = state.db;
    let conn = match db.0.lock() {
        Ok(c) => c,
        Err(e) => {
            return Json(ApiResponse::error(format!("Failed to lock database: {}", e)));
        }
    };

    let mut stmt = match conn.prepare(
        "SELECT id, project_id, name, icon, color, nickname, gender, agent_type, system_prompt, default_task, model, tools, enable_file_read, enable_file_write, enable_network, hooks, settings, role_type, created_at, updated_at FROM agents WHERE role_type = 'teamlead' ORDER BY created_at DESC"
    ) {
        Ok(s) => s,
        Err(e) => {
            return Json(ApiResponse::error(format!("Failed to prepare query: {}", e)));
        }
    };

    let agents = stmt.query_map([], |row| {
        Ok(serde_json::json!({
            "id": row.get::<_, Option<String>>(0)?,
            "project_id": row.get::<_, Option<String>>(1)?,
            "name": row.get::<_, String>(2)?,
            "icon": row.get::<_, String>(3)?,
            "color": row.get::<_, Option<String>>(4)?,
            "nickname": row.get::<_, Option<String>>(5)?,
            "gender": row.get::<_, Option<String>>(6)?,
            "agent_type": row.get::<_, String>(7).unwrap_or_else(|_| "general-purpose".to_string()),
            "system_prompt": row.get::<_, String>(8)?,
            "default_task": row.get::<_, Option<String>>(9)?,
            "model": row.get::<_, String>(10).unwrap_or_else(|_| "sonnet".to_string()),
            "tools": row.get::<_, Option<String>>(11)?,
            "enable_file_read": row.get::<_, bool>(12).unwrap_or(true),
            "enable_file_write": row.get::<_, bool>(13).unwrap_or(true),
            "enable_network": row.get::<_, bool>(14).unwrap_or(false),
            "hooks": row.get::<_, Option<String>>(15)?,
            "settings": row.get::<_, Option<String>>(16)?,
            "role_type": row.get::<_, Option<String>>(17)?,
            "created_at": row.get::<_, String>(18)?,
            "updated_at": row.get::<_, String>(19)?,
        }))
    });

    match agents {
        Ok(rows) => {
            let result: Vec<serde_json::Value> = rows.filter_map(|r| r.ok()).collect();
            Json(ApiResponse::success(result))
        }
        Err(e) => Json(ApiResponse::error(format!("Query failed: {}", e))),
    }
}

/// Simple usage endpoint - return empty for now
async fn get_usage() -> Json<ApiResponse<Vec<serde_json::Value>>> {
    Json(ApiResponse::success(vec![]))
}

/// Get Claude settings - return basic defaults for web mode
async fn get_claude_settings() -> Json<ApiResponse<serde_json::Value>> {
    let default_settings = serde_json::json!({
        "data": {
            "model": "claude-3-5-sonnet-20241022",
            "max_tokens": 8192,
            "temperature": 0.0,
            "auto_save": true,
            "theme": "dark"
        }
    });
    Json(ApiResponse::success(default_settings))
}

/// Check Claude version - return mock status for web mode
async fn check_claude_version() -> Json<ApiResponse<serde_json::Value>> {
    let version_status = serde_json::json!({
        "status": "ok",
        "version": "web-mode",
        "message": "Running in web server mode"
    });
    Json(ApiResponse::success(version_status))
}

/// List all available Claude installations on the system
async fn list_claude_installations(
) -> Json<ApiResponse<Vec<crate::claude_binary::ClaudeInstallation>>> {
    let installations = crate::claude_binary::discover_claude_installations();

    if installations.is_empty() {
        Json(ApiResponse::error(
            "No Claude Code installations found on the system".to_string(),
        ))
    } else {
        Json(ApiResponse::success(installations))
    }
}

/// Get system prompt - return default for web mode
async fn get_system_prompt() -> Json<ApiResponse<String>> {
    let default_prompt =
        "You are Claude, an AI assistant created by Anthropic. You are running in web server mode."
            .to_string();
    Json(ApiResponse::success(default_prompt))
}

/// Open new session - mock for web mode
async fn open_new_session() -> Json<ApiResponse<String>> {
    let session_id = format!("web-session-{}", chrono::Utc::now().timestamp());
    Json(ApiResponse::success(session_id))
}

/// List slash commands - return empty for web mode
async fn list_slash_commands() -> Json<ApiResponse<Vec<serde_json::Value>>> {
    Json(ApiResponse::success(vec![]))
}

/// MCP list servers - return empty for web mode
async fn mcp_list() -> Json<ApiResponse<Vec<serde_json::Value>>> {
    Json(ApiResponse::success(vec![]))
}

/// Load session history from JSONL file
async fn load_session_history(
    Path((session_id, project_id)): Path<(String, String)>,
) -> Json<ApiResponse<Vec<serde_json::Value>>> {
    match commands::claude::load_session_history(session_id, project_id).await {
        Ok(history) => Json(ApiResponse::success(history)),
        Err(e) => Json(ApiResponse::error(e.to_string())),
    }
}

/// List running Claude sessions
async fn list_running_claude_sessions() -> Json<ApiResponse<Vec<serde_json::Value>>> {
    // Return empty for web mode - no actual Claude processes in web mode
    Json(ApiResponse::success(vec![]))
}

/// Execute Claude code - mock for web mode
async fn execute_claude_code() -> Json<ApiResponse<serde_json::Value>> {
    Json(ApiResponse::error("Claude execution is not available in web mode. Please use the desktop app for running Claude commands.".to_string()))
}

/// Continue Claude code - mock for web mode
async fn continue_claude_code() -> Json<ApiResponse<serde_json::Value>> {
    Json(ApiResponse::error("Claude execution is not available in web mode. Please use the desktop app for running Claude commands.".to_string()))
}

/// Resume Claude code - mock for web mode  
async fn resume_claude_code() -> Json<ApiResponse<serde_json::Value>> {
    Json(ApiResponse::error("Claude execution is not available in web mode. Please use the desktop app for running Claude commands.".to_string()))
}

/// Cancel Claude execution
async fn cancel_claude_execution(Path(session_id): Path<String>) -> Json<ApiResponse<()>> {
    // In web mode, we don't have a way to cancel the subprocess cleanly
    // The WebSocket closing should handle cleanup
    println!("[TRACE] Cancel request for session: {}", session_id);
    Json(ApiResponse::success(()))
}

/// Get Claude session output
async fn get_claude_session_output(Path(session_id): Path<String>) -> Json<ApiResponse<String>> {
    // In web mode, output is streamed via WebSocket, not stored
    println!("[TRACE] Output request for session: {}", session_id);
    Json(ApiResponse::success(
        "Output available via WebSocket only".to_string(),
    ))
}

/// WebSocket handler for Claude execution with streaming output
async fn claude_websocket(ws: WebSocketUpgrade, AxumState(state): AxumState<AppState>) -> Response {
    ws.on_upgrade(move |socket| claude_websocket_handler(socket, state))
}

async fn claude_websocket_handler(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();
    let session_id = uuid::Uuid::new_v4().to_string();

    println!(
        "[TRACE] WebSocket handler started - session_id: {}",
        session_id
    );

    // Channel for sending output to WebSocket
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(100);

    // Store session in state
    {
        let mut sessions = state.active_sessions.lock().await;
        sessions.insert(session_id.clone(), tx);
        println!(
            "[TRACE] Session stored in state - active sessions count: {}",
            sessions.len()
        );
    }

    // Task to forward channel messages to WebSocket
    let session_id_for_forward = session_id.clone();
    let forward_task = tokio::spawn(async move {
        println!(
            "[TRACE] Forward task started for session {}",
            session_id_for_forward
        );
        while let Some(message) = rx.recv().await {
            println!("[TRACE] Forwarding message to WebSocket: {}", message);
            if sender.send(Message::Text(message.into())).await.is_err() {
                println!("[TRACE] Failed to send message to WebSocket - connection closed");
                break;
            }
        }
        println!(
            "[TRACE] Forward task ended for session {}",
            session_id_for_forward
        );
    });

    // Handle incoming messages from WebSocket
    println!("[TRACE] Starting to listen for WebSocket messages");
    while let Some(msg) = receiver.next().await {
        println!("[TRACE] Received WebSocket message: {:?}", msg);
        if let Ok(msg) = msg {
            if let Message::Text(text) = msg {
                println!(
                    "[TRACE] WebSocket text message received - length: {} chars",
                    text.len()
                );
                println!("[TRACE] WebSocket message content: {}", text);

                // 解析消息
                match serde_json::from_str::<ClaudeWebSocketMessage>(&text) {
                    Ok(message) => {
                        println!("[TRACE] Successfully parsed message: {:?}", message);

                        let session_id_clone = session_id.clone();
                        let state_clone = state.clone();

                        match message {
                            // 启动新进程
                            ClaudeWebSocketMessage::Execute { project_path, prompt, model } => {
                                println!("[TRACE] Executing new Claude process");
                                tokio::spawn(async move {
                                    let result = execute_claude_command(
                                        project_path,
                                        prompt,
                                        model.unwrap_or_default(),
                                        session_id_clone.clone(),
                                        state_clone.clone(),
                                    ).await;

                                    // 发送完成消息
                                    if let Some(sender) = state_clone.active_sessions.lock().await.get(&session_id_clone) {
                                        let completion_msg = match result {
                                            Ok(_) => json!({ "type": "complete", "success": true }),
                                            Err(e) => json!({ "type": "error", "message": e }),
                                        };
                                        let _ = sender.send(completion_msg.to_string()).await;
                                    }
                                });
                            }
                            // 发送输入到进程
                            ClaudeWebSocketMessage::Send { content } => {
                                println!("[TRACE] Sending input to Claude process");
                                tokio::spawn(async move {
                                    // 从 session_id 获取 registry 中的进程
                                    let registry = state_clone.process_registry.clone();
                                    let result = registry.send_to_process_async(&session_id_clone, &content).await;

                                    if let Some(sender) = state_clone.active_sessions.lock().await.get(&session_id_clone) {
                                        match result {
                                            Ok(_) => {
                                                let _ = sender.send(json!({ "type": "sent" }).to_string()).await;
                                            }
                                            Err(e) => {
                                                let _ = sender.send(json!({ "type": "error", "message": e }).to_string()).await;
                                            }
                                        }
                                    }
                                });
                            }
                            // 终止进程
                            ClaudeWebSocketMessage::Exit => {
                                println!("[TRACE] Exiting Claude process");
                                let session_id_for_exit = session_id_clone.clone();
                                let state_for_exit = state_clone.clone();
                                tokio::spawn(async move {
                                    let registry = state_for_exit.process_registry.clone();
                                    // 使用 kill_process 方法
                                    let _ = registry.kill_process(session_id_for_exit.clone()).await;

                                    if let Some(sender) = state_for_exit.active_sessions.lock().await.get(&session_id_for_exit) {
                                        let _ = sender.send(json!({ "type": "exited" }).to_string()).await;
                                    }
                                });
                            }
                            // 继续对话
                            ClaudeWebSocketMessage::Continue { project_path, prompt, model } => {
                                println!("[TRACE] Continue Claude process");
                                tokio::spawn(async move {
                                    let result = continue_claude_command(
                                        project_path,
                                        prompt,
                                        model.unwrap_or_default(),
                                        session_id_clone.clone(),
                                        state_clone.clone(),
                                    ).await;

                                    if let Some(sender) = state_clone.active_sessions.lock().await.get(&session_id_clone) {
                                        let completion_msg = match result {
                                            Ok(_) => json!({ "type": "complete", "success": true }),
                                            Err(e) => json!({ "type": "error", "message": e }),
                                        };
                                        let _ = sender.send(completion_msg.to_string()).await;
                                    }
                                });
                            }
                            // 恢复会话
                            ClaudeWebSocketMessage::Resume { project_path, session_id, prompt, model } => {
                                println!("[TRACE] Resume Claude session");
                                tokio::spawn(async move {
                                    let result = resume_claude_command(
                                        project_path,
                                        session_id,
                                        prompt,
                                        model.unwrap_or_default(),
                                        session_id_clone.clone(),
                                        state_clone.clone(),
                                    ).await;

                                    if let Some(sender) = state_clone.active_sessions.lock().await.get(&session_id_clone) {
                                        let completion_msg = match result {
                                            Ok(_) => json!({ "type": "complete", "success": true }),
                                            Err(e) => json!({ "type": "error", "message": e }),
                                        };
                                        let _ = sender.send(completion_msg.to_string()).await;
                                    }
                                });
                            }
                        }
                    }
                    Err(e) => {
                        println!("[TRACE] Failed to parse WebSocket request: {}", e);
                        println!("[TRACE] Raw message that failed to parse: {}", text);

                        // Send error back to client
                        let error_msg = json!({
                            "type": "error",
                            "message": format!("Failed to parse request: {}", e)
                        });
                        if let Some(sender_tx) = state.active_sessions.lock().await.get(&session_id)
                        {
                            let _ = sender_tx.send(error_msg.to_string()).await;
                        }
                    }
                }
            } else if let Message::Close(_) = msg {
                println!("[TRACE] WebSocket close message received");
                break;
            } else {
                println!("[TRACE] Non-text WebSocket message received: {:?}", msg);
            }
        } else {
            println!("[TRACE] Error receiving WebSocket message");
        }
    }

    println!("[TRACE] WebSocket message loop ended");

    // Clean up session
    {
        let mut sessions = state.active_sessions.lock().await;
        sessions.remove(&session_id);
        println!(
            "[TRACE] Session {} removed from state - remaining sessions: {}",
            session_id,
            sessions.len()
        );
    }

    forward_task.abort();
    println!("[TRACE] WebSocket handler ended for session {}", session_id);
}

// Claude command execution functions for WebSocket streaming
async fn execute_claude_command(
    project_path: String,
    prompt: String,
    model: String,
    session_id: String,
    state: AppState,
) -> Result<(), String> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::process::Command;

    println!("[TRACE] execute_claude_command called:");
    println!("[TRACE]   project_path: {}", project_path);
    println!("[TRACE]   prompt length: {} chars", prompt.len());
    println!("[TRACE]   model: {}", model);
    println!("[TRACE]   session_id: {}", session_id);

    // Send initial message
    println!("[TRACE] Sending initial start message");
    send_to_session(
        &state,
        &session_id,
        json!({
            "type": "start",
            "message": "Starting Claude execution..."
        })
        .to_string(),
    )
    .await;

    // Find Claude binary (simplified for web mode)
    println!("[TRACE] Finding Claude binary...");
    let claude_path = find_claude_binary_web().map_err(|e| {
        let error = format!("Claude binary not found: {}", e);
        println!("[TRACE] Error finding Claude binary: {}", error);
        error
    })?;
    println!("[TRACE] Found Claude binary: {}", claude_path);

    // Create Claude command
    println!("[TRACE] Creating Claude command...");
    let mut cmd = Command::new(&claude_path);
    let args = [
        "-p",
        &prompt,
        "--model",
        &model,
        "--output-format",
        "stream-json",
        "--input-format",
        "stream-json",
        "--verbose",
        "--dangerously-skip-permissions",
    ];
    cmd.args(args);
    cmd.current_dir(&project_path);
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    println!(
        "[TRACE] Command: {} {:?} (in dir: {})",
        claude_path, args, project_path
    );

    // Spawn Claude process
    println!("[TRACE] Spawning Claude process...");
    let mut child = cmd.spawn().map_err(|e| {
        let error = format!("Failed to spawn Claude: {}", e);
        println!("[TRACE] Spawn error: {}", error);
        error
    })?;
    println!("[TRACE] Claude process spawned successfully");

    // Get stdout for streaming
    let stdout = child.stdout.take().ok_or_else(|| {
        println!("[TRACE] Failed to get stdout from child process");
        "Failed to get stdout".to_string()
    })?;
    let stdout_reader = BufReader::new(stdout);

    println!("[TRACE] Starting to read Claude output...");
    // Stream output line by line
    let mut lines = stdout_reader.lines();
    let mut line_count = 0;
    while let Ok(Some(line)) = lines.next_line().await {
        line_count += 1;
        println!("[TRACE] Claude output line {}: {}", line_count, line);

        // Send each line to WebSocket
        let message = json!({
            "type": "output",
            "content": line
        })
        .to_string();
        println!("[TRACE] Sending output message to session: {}", message);
        send_to_session(&state, &session_id, message).await;
    }

    println!(
        "[TRACE] Finished reading Claude output ({} lines total)",
        line_count
    );

    // Wait for process to complete
    println!("[TRACE] Waiting for Claude process to complete...");
    let exit_status = child.wait().await.map_err(|e| {
        let error = format!("Failed to wait for Claude: {}", e);
        println!("[TRACE] Wait error: {}", error);
        error
    })?;

    println!(
        "[TRACE] Claude process completed with status: {:?}",
        exit_status
    );

    if !exit_status.success() {
        let error = format!(
            "Claude execution failed with exit code: {:?}",
            exit_status.code()
        );
        println!("[TRACE] Claude execution failed: {}", error);
        return Err(error);
    }

    println!("[TRACE] execute_claude_command completed successfully");
    Ok(())
}

async fn continue_claude_command(
    project_path: String,
    prompt: String,
    model: String,
    session_id: String,
    state: AppState,
) -> Result<(), String> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::process::Command;

    send_to_session(
        &state,
        &session_id,
        json!({
            "type": "start",
            "message": "Continuing Claude session..."
        })
        .to_string(),
    )
    .await;

    // Find Claude binary
    let claude_path =
        find_claude_binary_web().map_err(|e| format!("Claude binary not found: {}", e))?;

    // Create continue command
    let mut cmd = Command::new(&claude_path);
    cmd.args([
        "-c", // Continue flag
        "-p",
        &prompt,
        "--model",
        &model,
        "--output-format",
        "stream-json",
        "--verbose",
        "--dangerously-skip-permissions",
    ]);
    cmd.current_dir(&project_path);
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    // Spawn and stream output
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn Claude: {}", e))?;
    let stdout = child.stdout.take().ok_or("Failed to get stdout")?;
    let stdout_reader = BufReader::new(stdout);

    let mut lines = stdout_reader.lines();
    while let Ok(Some(line)) = lines.next_line().await {
        send_to_session(
            &state,
            &session_id,
            json!({
                "type": "output",
                "content": line
            })
            .to_string(),
        )
        .await;
    }

    let exit_status = child
        .wait()
        .await
        .map_err(|e| format!("Failed to wait for Claude: {}", e))?;
    if !exit_status.success() {
        return Err(format!(
            "Claude execution failed with exit code: {:?}",
            exit_status.code()
        ));
    }

    Ok(())
}

async fn resume_claude_command(
    project_path: String,
    claude_session_id: String,
    prompt: String,
    model: String,
    session_id: String,
    state: AppState,
) -> Result<(), String> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::process::Command;

    println!("[resume_claude_command] Starting with project_path: {}, claude_session_id: {}, prompt: {}, model: {}", 
             project_path, claude_session_id, prompt, model);

    send_to_session(
        &state,
        &session_id,
        json!({
            "type": "start",
            "message": "Resuming Claude session..."
        })
        .to_string(),
    )
    .await;

    // Find Claude binary
    println!("[resume_claude_command] Finding Claude binary...");
    let claude_path =
        find_claude_binary_web().map_err(|e| format!("Claude binary not found: {}", e))?;
    println!(
        "[resume_claude_command] Found Claude binary: {}",
        claude_path
    );

    // Create resume command
    println!("[resume_claude_command] Creating command...");
    let mut cmd = Command::new(&claude_path);
    let args = [
        "--resume",
        &claude_session_id,
        "-p",
        &prompt,
        "--model",
        &model,
        "--output-format",
        "stream-json",
        "--input-format",
        "stream-json",
        "--verbose",
        "--dangerously-skip-permissions",
    ];
    cmd.args(args);
    cmd.current_dir(&project_path);
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    println!(
        "[resume_claude_command] Command: {} {:?} (in dir: {})",
        claude_path, args, project_path
    );

    // Spawn and stream output
    println!("[resume_claude_command] Spawning process...");
    let mut child = cmd.spawn().map_err(|e| {
        let error = format!("Failed to spawn Claude: {}", e);
        println!("[resume_claude_command] Spawn error: {}", error);
        error
    })?;
    println!("[resume_claude_command] Process spawned successfully");
    let stdout = child.stdout.take().ok_or("Failed to get stdout")?;
    let stdout_reader = BufReader::new(stdout);

    let mut lines = stdout_reader.lines();
    while let Ok(Some(line)) = lines.next_line().await {
        send_to_session(
            &state,
            &session_id,
            json!({
                "type": "output",
                "content": line
            })
            .to_string(),
        )
        .await;
    }

    let exit_status = child
        .wait()
        .await
        .map_err(|e| format!("Failed to wait for Claude: {}", e))?;
    if !exit_status.success() {
        return Err(format!(
            "Claude execution failed with exit code: {:?}",
            exit_status.code()
        ));
    }

    Ok(())
}

async fn send_to_session(state: &AppState, session_id: &str, message: String) {
    println!("[TRACE] send_to_session called for session: {}", session_id);
    println!("[TRACE] Message: {}", message);

    let sessions = state.active_sessions.lock().await;
    if let Some(sender) = sessions.get(session_id) {
        println!("[TRACE] Found session in active sessions, sending message...");
        match sender.send(message).await {
            Ok(_) => println!("[TRACE] Message sent successfully"),
            Err(e) => println!("[TRACE] Failed to send message: {}", e),
        }
    } else {
        println!(
            "[TRACE] Session {} not found in active sessions",
            session_id
        );
        println!(
            "[TRACE] Active sessions: {:?}",
            sessions.keys().collect::<Vec<_>>()
        );
    }
}

/// Create the web server
pub async fn create_web_server(port: u16, db: AgentDb) -> Result<(), Box<dyn std::error::Error>> {
    let state = AppState {
        active_sessions: Arc::new(Mutex::new(std::collections::HashMap::new())),
        process_registry: Arc::new(process::ProcessRegistry::new()),
        db: Arc::new(db),
    };

    // CORS layer to allow requests from phone browsers
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_headers(Any);

    // Create router with API endpoints
    let app = Router::new()
        // Frontend routes
        .route("/", get(serve_frontend))
        .route("/index.html", get(serve_frontend))
        // API routes (REST API equivalent of Tauri commands)
        .route("/api/projects", get(get_projects))
        .route("/api/projects/{project_id}/sessions", get(get_sessions))
        .route("/api/agents", get(get_agents))
        .route("/api/agents/teamleads", get(get_teamleads))
        .route("/api/usage", get(get_usage))
        // Settings and configuration
        .route("/api/settings/claude", get(get_claude_settings))
        .route("/api/settings/claude/version", get(check_claude_version))
        .route(
            "/api/settings/claude/installations",
            get(list_claude_installations),
        )
        .route("/api/settings/system-prompt", get(get_system_prompt))
        // Session management
        .route("/api/sessions/new", get(open_new_session))
        // Slash commands
        .route("/api/slash-commands", get(list_slash_commands))
        // MCP
        .route("/api/mcp/servers", get(mcp_list))
        // Session history
        .route(
            "/api/sessions/{session_id}/history/{project_id}",
            get(load_session_history),
        )
        .route("/api/sessions/running", get(list_running_claude_sessions))
        // Claude execution endpoints (read-only in web mode)
        .route("/api/sessions/execute", get(execute_claude_code))
        .route("/api/sessions/continue", get(continue_claude_code))
        .route("/api/sessions/resume", get(resume_claude_code))
        .route(
            "/api/sessions/{sessionId}/cancel",
            get(cancel_claude_execution),
        )
        .route(
            "/api/sessions/{sessionId}/output",
            get(get_claude_session_output),
        )
        // WebSocket endpoint for real-time Claude execution
        .route("/ws/claude", get(claude_websocket))
        // Serve static assets (in dev mode, these should be empty as Vite serves them)
        .nest_service("/assets", ServeDir::new("../dist/assets"))
        .nest_service("/vite.svg", ServeDir::new("../dist/vite.svg"))
        .layer(cors)
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    println!("🌐 Web server running on http://0.0.0.0:{}", port);
    if is_dev_mode() {
        println!("🔧 Dev mode: frontend served from Vite at http://localhost:1420");
    } else {
        println!("📱 Access from phone: http://YOUR_PC_IP:{}", port);
    }

    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// Start web server mode (alternative to Tauri GUI)
pub async fn start_web_mode(port: Option<u16>, db: AgentDb) -> Result<(), Box<dyn std::error::Error>> {
    let port = port.unwrap_or(8080);

    println!("🚀 Starting Vibe Agent Team in web server mode...");
    create_web_server(port, db).await
}
