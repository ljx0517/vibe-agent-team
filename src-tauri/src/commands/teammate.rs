use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use std::process::Stdio;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::io::{AsyncBufReadExt, BufReader as TokioBufReader};
use tokio::process::Command;
use uuid::Uuid;

use crate::claude_binary::find_claude_binary;
use crate::commands::agents::{get_agent, AgentDb};
use crate::process::ProcessRegistryState;

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
    model: Option<String>,
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

    // Generate run_id
    let run_id = Uuid::new_v4().to_string();

    // Build agents JSON config for --agents flag
    let agents_json = serde_json::json!({
        &agent_id: {
            "prompt": agent.system_prompt,
            "tools": agent.tools.as_ref().and_then(|t| serde_json::from_str::<Vec<String>>(t).ok()).unwrap_or_default(),
            "model": execution_model.clone()
        }
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
    let args = vec![
        "--agents".to_string(),
        agents_json.to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--verbose".to_string(),
        "--dangerously-skip-permissions".to_string(),
    ];

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

    // Spawn monitoring task
    let app_handle_monitor = app.clone();
    let registry_monitor = registry.0.clone();
    let run_id_monitor = run_id.clone();
    tokio::spawn(async move {
        // Wait for stdout/stderr tasks to complete
        let _ = stdout_task.await;
        let _ = stderr_task.await;

        info!("Teammate agent {} finished", run_id_monitor);

        // Emit completion event
        let _ = app_handle_monitor.emit(&format!("teammate-complete:{}", run_id_monitor), true);
        let _ = app_handle_monitor.emit("teammate-complete", true);

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
