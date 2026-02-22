use super::agents::AgentDb;
use anyhow::Result;
use rusqlite::{params, types::ValueRef, Connection, Result as SqliteResult};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as JsonValue};
use std::collections::HashMap;
use tauri::{AppHandle, Emitter, Manager, State};
use uuid::Uuid;

/// Represents metadata about a database table
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TableInfo {
    pub name: String,
    pub row_count: i64,
    pub columns: Vec<ColumnInfo>,
}

/// Represents metadata about a table column
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ColumnInfo {
    pub cid: i32,
    pub name: String,
    pub type_name: String,
    pub notnull: bool,
    pub dflt_value: Option<String>,
    pub pk: bool,
}

/// Represents a page of table data
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TableData {
    pub table_name: String,
    pub columns: Vec<ColumnInfo>,
    pub rows: Vec<Map<String, JsonValue>>,
    pub total_rows: i64,
    pub page: i64,
    pub page_size: i64,
    pub total_pages: i64,
}

/// SQL query result
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<JsonValue>>,
    pub rows_affected: Option<i64>,
    pub last_insert_rowid: Option<i64>,
}

/// List all tables in the database
#[tauri::command]
pub async fn storage_list_tables(db: State<'_, AgentDb>) -> Result<Vec<TableInfo>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;

    // Query for all tables
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .map_err(|e| e.to_string())?;

    let table_names: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .collect::<SqliteResult<Vec<_>>>()
        .map_err(|e| e.to_string())?;

    drop(stmt);

    let mut tables = Vec::new();

    for table_name in table_names {
        // Get row count
        let row_count: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM {}", table_name), [], |row| {
                row.get(0)
            })
            .unwrap_or(0);

        // Get column information
        let mut pragma_stmt = conn
            .prepare(&format!("PRAGMA table_info({})", table_name))
            .map_err(|e| e.to_string())?;

        let columns: Vec<ColumnInfo> = pragma_stmt
            .query_map([], |row| {
                Ok(ColumnInfo {
                    cid: row.get(0)?,
                    name: row.get(1)?,
                    type_name: row.get(2)?,
                    notnull: row.get::<_, i32>(3)? != 0,
                    dflt_value: row.get(4)?,
                    pk: row.get::<_, i32>(5)? != 0,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<SqliteResult<Vec<_>>>()
            .map_err(|e| e.to_string())?;

        tables.push(TableInfo {
            name: table_name,
            row_count,
            columns,
        });
    }

    Ok(tables)
}

/// Read table data with pagination
#[tauri::command]
#[allow(non_snake_case)]
pub async fn storage_read_table(
    db: State<'_, AgentDb>,
    tableName: String,
    page: i64,
    pageSize: i64,
    searchQuery: Option<String>,
) -> Result<TableData, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;

    // Validate table name to prevent SQL injection
    if !is_valid_table_name(&conn, &tableName)? {
        return Err("Invalid table name".to_string());
    }

    // Get column information
    let mut pragma_stmt = conn
        .prepare(&format!("PRAGMA table_info({})", tableName))
        .map_err(|e| e.to_string())?;

    let columns: Vec<ColumnInfo> = pragma_stmt
        .query_map([], |row| {
            Ok(ColumnInfo {
                cid: row.get(0)?,
                name: row.get(1)?,
                type_name: row.get(2)?,
                notnull: row.get::<_, i32>(3)? != 0,
                dflt_value: row.get(4)?,
                pk: row.get::<_, i32>(5)? != 0,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<SqliteResult<Vec<_>>>()
        .map_err(|e| e.to_string())?;

    drop(pragma_stmt);

    // Build query with optional search
    let (query, count_query) = if let Some(search) = &searchQuery {
        // Create search conditions for all text columns
        let search_conditions: Vec<String> = columns
            .iter()
            .filter(|col| col.type_name.contains("TEXT") || col.type_name.contains("VARCHAR"))
            .map(|col| format!("{} LIKE '%{}%'", col.name, search.replace("'", "''")))
            .collect();

        if search_conditions.is_empty() {
            (
                format!("SELECT * FROM {} LIMIT ? OFFSET ?", tableName),
                format!("SELECT COUNT(*) FROM {}", tableName),
            )
        } else {
            let where_clause = search_conditions.join(" OR ");
            (
                format!(
                    "SELECT * FROM {} WHERE {} LIMIT ? OFFSET ?",
                    tableName, where_clause
                ),
                format!("SELECT COUNT(*) FROM {} WHERE {}", tableName, where_clause),
            )
        }
    } else {
        (
            format!("SELECT * FROM {} LIMIT ? OFFSET ?", tableName),
            format!("SELECT COUNT(*) FROM {}", tableName),
        )
    };

    // Get total row count
    let total_rows: i64 = conn
        .query_row(&count_query, [], |row| row.get(0))
        .unwrap_or(0);

    // Calculate pagination
    let offset = (page - 1) * pageSize;
    let total_pages = (total_rows as f64 / pageSize as f64).ceil() as i64;

    // Query data
    let mut data_stmt = conn.prepare(&query).map_err(|e| e.to_string())?;

    let rows: Vec<Map<String, JsonValue>> = data_stmt
        .query_map(params![pageSize, offset], |row| {
            let mut row_map = Map::new();

            for (idx, col) in columns.iter().enumerate() {
                let value = match row.get_ref(idx)? {
                    ValueRef::Null => JsonValue::Null,
                    ValueRef::Integer(i) => JsonValue::Number(serde_json::Number::from(i)),
                    ValueRef::Real(f) => {
                        if let Some(n) = serde_json::Number::from_f64(f) {
                            JsonValue::Number(n)
                        } else {
                            JsonValue::String(f.to_string())
                        }
                    }
                    ValueRef::Text(s) => JsonValue::String(String::from_utf8_lossy(s).to_string()),
                    ValueRef::Blob(b) => JsonValue::String(base64::Engine::encode(
                        &base64::engine::general_purpose::STANDARD,
                        b,
                    )),
                };
                row_map.insert(col.name.clone(), value);
            }

            Ok(row_map)
        })
        .map_err(|e| e.to_string())?
        .collect::<SqliteResult<Vec<_>>>()
        .map_err(|e| e.to_string())?;

    Ok(TableData {
        table_name: tableName,
        columns,
        rows,
        total_rows,
        page,
        page_size: pageSize,
        total_pages,
    })
}

/// Update a row in a table
#[tauri::command]
#[allow(non_snake_case)]
pub async fn storage_update_row(
    db: State<'_, AgentDb>,
    tableName: String,
    primaryKeyValues: HashMap<String, JsonValue>,
    updates: HashMap<String, JsonValue>,
) -> Result<(), String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;

    // Validate table name
    if !is_valid_table_name(&conn, &tableName)? {
        return Err("Invalid table name".to_string());
    }

    // Build UPDATE query
    let set_clauses: Vec<String> = updates
        .keys()
        .enumerate()
        .map(|(idx, key)| format!("{} = ?{}", key, idx + 1))
        .collect();

    let where_clauses: Vec<String> = primaryKeyValues
        .keys()
        .enumerate()
        .map(|(idx, key)| format!("{} = ?{}", key, idx + updates.len() + 1))
        .collect();

    let query = format!(
        "UPDATE {} SET {} WHERE {}",
        tableName,
        set_clauses.join(", "),
        where_clauses.join(" AND ")
    );

    // Prepare parameters
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    // Add update values
    for value in updates.values() {
        params.push(json_to_sql_value(value)?);
    }

    // Add where clause values
    for value in primaryKeyValues.values() {
        params.push(json_to_sql_value(value)?);
    }

    // Execute update
    conn.execute(
        &query,
        rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
    )
    .map_err(|e| format!("Failed to update row: {}", e))?;

    Ok(())
}

/// Delete a row from a table
#[tauri::command]
#[allow(non_snake_case)]
pub async fn storage_delete_row(
    db: State<'_, AgentDb>,
    tableName: String,
    primaryKeyValues: HashMap<String, JsonValue>,
) -> Result<(), String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;

    // Validate table name
    if !is_valid_table_name(&conn, &tableName)? {
        return Err("Invalid table name".to_string());
    }

    // Build DELETE query
    let where_clauses: Vec<String> = primaryKeyValues
        .keys()
        .enumerate()
        .map(|(idx, key)| format!("{} = ?{}", key, idx + 1))
        .collect();

    let query = format!(
        "DELETE FROM {} WHERE {}",
        tableName,
        where_clauses.join(" AND ")
    );

    // Prepare parameters
    let params: Vec<Box<dyn rusqlite::ToSql>> = primaryKeyValues
        .values()
        .map(json_to_sql_value)
        .collect::<Result<Vec<_>, _>>()?;

    // Execute delete
    conn.execute(
        &query,
        rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
    )
    .map_err(|e| format!("Failed to delete row: {}", e))?;

    Ok(())
}

/// Insert a new row into a table
#[tauri::command]
#[allow(non_snake_case)]
pub async fn storage_insert_row(
    db: State<'_, AgentDb>,
    tableName: String,
    values: HashMap<String, JsonValue>,
) -> Result<i64, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;

    // Validate table name
    if !is_valid_table_name(&conn, &tableName)? {
        return Err("Invalid table name".to_string());
    }

    // Build INSERT query
    let columns: Vec<&String> = values.keys().collect();
    let placeholders: Vec<String> = (1..=columns.len()).map(|i| format!("?{}", i)).collect();

    let query = format!(
        "INSERT INTO {} ({}) VALUES ({})",
        tableName,
        columns
            .iter()
            .map(|c| c.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        placeholders.join(", ")
    );

    // Prepare parameters
    let params: Vec<Box<dyn rusqlite::ToSql>> = values
        .values()
        .map(json_to_sql_value)
        .collect::<Result<Vec<_>, _>>()?;

    // Execute insert
    conn.execute(
        &query,
        rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
    )
    .map_err(|e| format!("Failed to insert row: {}", e))?;

    Ok(conn.last_insert_rowid())
}

/// Execute a raw SQL query
#[tauri::command]
pub async fn storage_execute_sql(
    db: State<'_, AgentDb>,
    query: String,
) -> Result<QueryResult, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;

    // Check if it's a SELECT query
    let is_select = query.trim().to_uppercase().starts_with("SELECT");

    if is_select {
        // Handle SELECT queries
        let mut stmt = conn.prepare(&query).map_err(|e| e.to_string())?;
        let column_count = stmt.column_count();

        // Get column names
        let columns: Vec<String> = (0..column_count)
            .map(|i| stmt.column_name(i).unwrap_or("").to_string())
            .collect();

        // Execute query and collect results
        let rows: Vec<Vec<JsonValue>> = stmt
            .query_map([], |row| {
                let mut row_values = Vec::new();
                for i in 0..column_count {
                    let value = match row.get_ref(i)? {
                        ValueRef::Null => JsonValue::Null,
                        ValueRef::Integer(n) => JsonValue::Number(serde_json::Number::from(n)),
                        ValueRef::Real(f) => {
                            if let Some(n) = serde_json::Number::from_f64(f) {
                                JsonValue::Number(n)
                            } else {
                                JsonValue::String(f.to_string())
                            }
                        }
                        ValueRef::Text(s) => {
                            JsonValue::String(String::from_utf8_lossy(s).to_string())
                        }
                        ValueRef::Blob(b) => JsonValue::String(base64::Engine::encode(
                            &base64::engine::general_purpose::STANDARD,
                            b,
                        )),
                    };
                    row_values.push(value);
                }
                Ok(row_values)
            })
            .map_err(|e| e.to_string())?
            .collect::<SqliteResult<Vec<_>>>()
            .map_err(|e| e.to_string())?;

        Ok(QueryResult {
            columns,
            rows,
            rows_affected: None,
            last_insert_rowid: None,
        })
    } else {
        // Handle non-SELECT queries (INSERT, UPDATE, DELETE, etc.)
        let rows_affected = conn.execute(&query, []).map_err(|e| e.to_string())?;

        Ok(QueryResult {
            columns: vec![],
            rows: vec![],
            rows_affected: Some(rows_affected as i64),
            last_insert_rowid: Some(conn.last_insert_rowid()),
        })
    }
}

/// Reset the entire database (with confirmation)
#[tauri::command]
pub async fn storage_reset_database(app: AppHandle) -> Result<(), String> {
    {
        // Drop all existing tables within a scoped block
        let db_state = app.state::<AgentDb>();
        let conn = db_state.0.lock().map_err(|e| e.to_string())?;

        // Disable foreign key constraints temporarily to allow dropping tables
        conn.execute("PRAGMA foreign_keys = OFF", [])
            .map_err(|e| format!("Failed to disable foreign keys: {}", e))?;

        // Drop tables - order doesn't matter with foreign keys disabled
        conn.execute("DROP TABLE IF EXISTS agent_runs", [])
            .map_err(|e| format!("Failed to drop agent_runs table: {}", e))?;
        conn.execute("DROP TABLE IF EXISTS agents", [])
            .map_err(|e| format!("Failed to drop agents table: {}", e))?;
        conn.execute("DROP TABLE IF EXISTS app_settings", [])
            .map_err(|e| format!("Failed to drop app_settings table: {}", e))?;

        // Re-enable foreign key constraints
        conn.execute("PRAGMA foreign_keys = ON", [])
            .map_err(|e| format!("Failed to re-enable foreign keys: {}", e))?;

        // Connection is automatically dropped at end of scope
    }

    // Re-initialize the database which will recreate all tables empty
    let new_conn = init_database(&app).map_err(|e| format!("Failed to reset database: {}", e))?;

    // Update the managed state with the new connection
    {
        let db_state = app.state::<AgentDb>();
        let mut conn_guard = db_state.0.lock().map_err(|e| e.to_string())?;
        *conn_guard = new_conn;
    }

    // Run VACUUM to optimize the database
    {
        let db_state = app.state::<AgentDb>();
        let conn = db_state.0.lock().map_err(|e| e.to_string())?;
        conn.execute("VACUUM", []).map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// Helper function to validate table name exists
fn is_valid_table_name(conn: &Connection, table_name: &str) -> Result<bool, String> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?",
            params![table_name],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;

    Ok(count > 0)
}

/// Helper function to convert JSON value to SQL value
fn json_to_sql_value(value: &JsonValue) -> Result<Box<dyn rusqlite::ToSql>, String> {
    match value {
        JsonValue::Null => Ok(Box::new(rusqlite::types::Null)),
        JsonValue::Bool(b) => Ok(Box::new(*b)),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Box::new(i))
            } else if let Some(f) = n.as_f64() {
                Ok(Box::new(f))
            } else {
                Err("Invalid number value".to_string())
            }
        }
        JsonValue::String(s) => Ok(Box::new(s.clone())),
        _ => Err("Unsupported value type".to_string()),
    }
}

/// Initialize the agents database (re-exported from agents module)
use super::agents::init_database;

/// Directory status for workspace validation
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DirectoryStatus {
    pub is_empty: bool,
    pub has_workspace_marker: bool,
    pub is_valid_workspace: bool,
    pub error: Option<String>,
}

/// Check directory status for workspace validation
#[tauri::command]
pub async fn check_directory_status(path: String) -> Result<DirectoryStatus, String> {
    use std::fs;

    let path_obj = std::path::Path::new(&path);

    if !path_obj.exists() {
        return Ok(DirectoryStatus {
            is_empty: true,
            has_workspace_marker: false,
            is_valid_workspace: false,
            error: Some("目录不存在".to_string()),
        });
    }

    if !path_obj.is_dir() {
        return Ok(DirectoryStatus {
            is_empty: true,
            has_workspace_marker: false,
            is_valid_workspace: false,
            error: Some("所选路径不是目录".to_string()),
        });
    }

    // Check for .vibe-team-workspace marker file
    let workspace_marker = path_obj.join(".vibe-team-workspace");
    let has_workspace_marker = workspace_marker.exists();

    // Read directory contents
    let entries = fs::read_dir(path_obj).map_err(|e| e.to_string())?;

    // Filter out hidden files/directories (starting with .)
    let visible_entries: Vec<_> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            !name_str.starts_with('.')
        })
        .collect();

    let is_empty = visible_entries.is_empty();

    Ok(DirectoryStatus {
        is_empty,
        has_workspace_marker,
        is_valid_workspace: is_empty || has_workspace_marker,
        error: None,
    })
}

/// Create workspace marker file in directory
#[tauri::command]
pub async fn create_workspace_marker(path: String) -> Result<(), String> {
    use std::fs;

    let path_obj = std::path::Path::new(&path);

    if !path_obj.exists() || !path_obj.is_dir() {
        return Err("无效的目录路径".to_string());
    }

    let marker_path = path_obj.join(".vibe-team-workspace");
    fs::write(&marker_path, "").map_err(|e| format!("创建工作空间标记失败: {}", e))?;

    Ok(())
}

/// Selected teamlead info when creating a project
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SelectedTeamlead {
    pub id: String,           // agent id
    pub project_agent_id: String, // project_agent_id from project_agents table
    pub name: String,
    pub nickname: Option<String>,
    pub gender: Option<String>,
    pub prompt: String,
    pub model: Option<String>,
    pub color: Option<String>,
}

/// Project creation input
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateProjectInput {
    pub name: String,
    pub project_code: Option<String>,
    pub description: String,
    pub work_dir: String,
    pub prompt: Option<String>,
    pub teamlead: Option<SelectedTeamlead>, // If provided, use existing teamlead instead of creating new one
}

/// Project creation result
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateProjectResult {
    pub project_id: String,
    pub workspace_id: String,
    pub members: Vec<TeamMember>,
}

/// Create a new project with associated workspace (synchronous, returns immediately)
#[tauri::command]
pub async fn storage_create_project(
    db: State<'_, AgentDb>,
    app: AppHandle,
    input: CreateProjectInput,
) -> Result<CreateProjectResult, String> {
    log::info!("========== Starting project creation ==========");
    log::info!("Project name: {}", input.name);
    log::info!("Work directory: {}", input.work_dir);

    let project_id = Uuid::new_v4().to_string();
    let workspace_id = Uuid::new_v4().to_string();

    log::info!("Generated project_id: {}", project_id);
    log::info!("Generated workspace_id: {}", workspace_id);

    // Insert project and workspace with a scoped lock
    {
        let conn = db.0.lock().map_err(|e| e.to_string())?;

        // Insert into projects table (initializing = 1 means "initializing")
        conn.execute(
            "INSERT INTO projects (id, name, project_code, description, working_dir, prompt, initializing, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, datetime('now'), datetime('now'))",
            params![project_id, input.name, input.project_code, input.description, input.work_dir, input.prompt],
        )
        .map_err(|e| format!("创建项目失败: {}", e))?;

        log::info!("Project inserted into database with initializing=1");

        // Insert into workspaces table
        conn.execute(
            "INSERT INTO workspaces (id, name, path, created_at, updated_at)
             VALUES (?1, ?2, ?3, datetime('now'), datetime('now'))",
            params![workspace_id, input.name, input.work_dir],
        )
        .map_err(|e| format!("创建工作空间失败: {}", e))?;

        log::info!("create workspace in: {}", input.work_dir);
    }

    // Spawn background task to execute project team skill
    log::info!("Starting background task for project team skill...");

    // Emit initial progress AFTER returning to frontend (so frontend has the project in list)
    // Use tokio::spawn to defer the event until after this function returns
    let app_for_deferred = app.clone();
    let project_id_for_deferred = project_id.clone();
    tokio::spawn(async move {
        // Small delay to ensure frontend has updated its project list
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        let _ = app_for_deferred.emit("project-progress", serde_json::json!({
            "project_id": project_id_for_deferred,
            "step": "starting",
            "message": "启动后台任务..."
        }));
    });

    let app_clone = app.clone();
    let project_description = input.description.clone();
    let project_path = input.work_dir.clone();
    let project_name = input.name.clone();
    let project_id_clone = project_id.clone();
    let existing_teamlead = input.teamlead.clone();
    let existing_teamlead_id = existing_teamlead.as_ref().map(|t| t.id.clone());

    // Execute skill in background task (non-blocking)
    tokio::spawn(async move {
        log::info!("[Background Task] Starting project team skill for project: {}", project_name);
        log::info!("[Background Task] Project ID: {}", project_id_clone);
        log::info!("[Background Task] Working directory: {}", project_path);

        // Execute the skill (this is the耗时操作)
        let members = match execute_project_team_skill(
            &app_clone,
            project_id_clone.clone(),
            project_name.clone(),
            project_description.clone(),
            project_path.clone(),
            existing_teamlead.clone(),
        ).await {
            Ok(m) => m,
            Err(e) => {
                log::error!("Failed to execute project team skill: {}", e);
                Vec::new()
            }
        };

        // Emit progress: executing claude completed
        let _ = app_clone.emit("project-progress", serde_json::json!({
            "project_id": project_id_clone,
            "step": "executing_claude",
            "message": "调用 Claude Code 完成，正在保存团队成员..."
        }));

        // Save members to agents table
        let total_members = members.len();
        match init_database(&app_clone) {
            Ok(conn) => {
                // Emit progress: saving agents
                let _ = app_clone.emit("project-progress", serde_json::json!({
                    "project_id": project_id_clone,
                    "step": "saving_agents",
                    "message": format!("正在保存团队成员 ({}/{})...", 0, total_members)
                }));

                let mut saved_count = 0;
                for member in members {
                    // If this is a teamlead and we have an existing teamlead, use the existing agent_id
                    let agent_id = if member.role_type == "teamlead" && existing_teamlead_id.is_some() {
                        existing_teamlead_id.clone().unwrap()
                    } else {
                        Uuid::new_v4().to_string()
                    };
                    let color = member.color.clone();
                    let icon = color.clone().unwrap_or_else(|| "🤖".to_string());
                    let nickname = member.nickname.clone();
                    let gender = member.gender.clone();
                    let agent_type = member.agent_type.clone();
                    let system_prompt = member.prompt.clone().unwrap_or_default();
                    let model = member.model.clone().unwrap_or_else(|| "sonnet".to_string());
                    let role_type = member.role_type.clone();
                    let project_agent_id = member.agent_id.clone(); // 保存 agentId 字段

                    match conn.execute(
                        "INSERT INTO agents (id, project_id, name, icon, color, nickname, gender, agent_type, system_prompt, model, role_type, created_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, datetime('now'), datetime('now'))",
                        params![agent_id, project_id_clone, member.name, icon, color, nickname, gender, agent_type, system_prompt, model, role_type],
                    ) {
                        Ok(_) => {
                            // Also insert into project_agents table
                            let project_agent_uuid = Uuid::new_v4().to_string();
                            if let Err(e) = conn.execute(
                                "INSERT INTO project_agents (id, project_id, agent_id, project_agent_id, created_at, updated_at)
                                 VALUES (?1, ?2, ?3, ?4, datetime('now'), datetime('now'))",
                                params![project_agent_uuid, project_id_clone, agent_id, project_agent_id],
                            ) {
                                log::error!("Failed to create project_agent {}: {}", project_agent_id, e);
                            } else {
                                log::info!("Created project_agent: {} -> agent: {}", project_agent_id, agent_id);
                            }

                            saved_count += 1;
                            log::info!("Created agent: {} for project {}", member.name, project_id_clone);
                            // Emit progress for each saved agent
                            let _ = app_clone.emit("project-progress", serde_json::json!({
                                "project_id": project_id_clone,
                                "step": "saving_agents",
                                "message": format!("正在保存团队成员 ({}/{})...", saved_count, total_members)
                            }));
                        }
                        Err(e) => {
                            log::error!("Failed to create agent {}: {}", member.name, e);
                        }
                    }
                }

                log::info!("Saved {}/{} team members", saved_count, total_members);

                // If existing_teamlead is provided, link it to the new project (project_agents only)
                if let Some(ref teamlead) = existing_teamlead {
                    log::info!("Linking existing teamlead to project: {}", teamlead.name);

                    // Only insert into project_agents table (agent already exists in agents table)
                    let project_agent_uuid = Uuid::new_v4().to_string();
                    if let Err(e) = conn.execute(
                        "INSERT INTO project_agents (id, project_id, agent_id, project_agent_id, created_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, datetime('now'), datetime('now'))",
                        params![project_agent_uuid, project_id_clone, teamlead.id, teamlead.project_agent_id],
                    ) {
                        log::error!("Failed to link existing teamlead to project_agents: {}", e);
                    } else {
                        log::info!("Existing teamlead linked to project via project_agents table");
                    }
                }

                // Mark project as initialized
                if let Err(e) = conn.execute(
                            "UPDATE projects SET initializing = 0, updated_at = datetime('now') WHERE id = ?1",
                            params![project_id_clone],
        ) {
            log::error!("Failed to update project initialization status: {}", e);
        } else {
            // Emit completion progress
            let _ = app_clone.emit("project-progress", serde_json::json!({
                "project_id": project_id_clone,
                "step": "completed",
                "message": "完成！"
            }));

            // Emit event to frontend to refresh project list
            log::info!("Emitting project-initialized event for project: {}", project_id_clone);
            let _ = app_clone.emit("project-initialized", &project_id_clone);
        }
    }
    Err(e) => {
        log::error!("Failed to open database in background: {}", e);
    }
    }
    });

    log::info!("========== Project creation request completed ==========");
    log::info!("Returning to frontend immediately. Background task will complete asynchronously.");
    log::info!("Project ID: {}, Workspace ID: {}", project_id, workspace_id);

    // Return immediately with empty members - frontend will receive updates via events
    Ok(CreateProjectResult {
        project_id: project_id.clone(),
        workspace_id,
        members: Vec::new(), // Members will be populated by background task and sent via events
    })
}

/// Complete project initialization (called after background skill execution)
#[tauri::command]
pub async fn complete_project_initialization(
    db: State<'_, AgentDb>,
    project_id: String,
) -> Result<(), String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;

    conn.execute(
        "UPDATE projects SET initializing = 0, updated_at = datetime('now') WHERE id = ?1",
        params![project_id],
    )
    .map_err(|e| format!("更新项目初始化状态失败: {}", e))?;

    log::info!("Project {} initialization completed", project_id);

    Ok(())
}

/// Project with workspace info
#[derive(Debug, Serialize, Deserialize)]
pub struct ProjectWithWorkspace {
    pub project_id: String,
    pub project_name: String,
    pub project_code: Option<String>,
    pub description: Option<String>,
    pub workspace_id: String,
    pub workspace_path: String,
    pub initializing: bool,
}

/// List all projects with their workspaces
#[tauri::command]
pub async fn storage_list_projects(
    db: State<'_, AgentDb>,
) -> Result<Vec<ProjectWithWorkspace>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;

    let mut stmt = conn
        .prepare(
            "SELECT p.id, p.name, p.project_code, p.description, w.id, w.path, COALESCE(p.initializing, 0) as initializing
             FROM projects p
             LEFT JOIN workspaces w ON w.name = p.name
             ORDER BY p.created_at DESC",
        )
        .map_err(|e| e.to_string())?;

    let projects = stmt
        .query_map([], |row| {
            Ok(ProjectWithWorkspace {
                project_id: row.get(0)?,
                project_name: row.get(1)?,
                project_code: row.get(2)?,
                description: row.get(3)?,
                workspace_id: row.get(4)?,
                workspace_path: row.get(5)?,
                initializing: row.get::<_, i32>(6)? != 0,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    Ok(projects)
}

/// Create project team skill and invoke it
/// This method:
/// 1. Creates the SKILL.md file in .claude/skills/create-project-team/
/// 2. Invokes the /create-project-team skill with project details
#[tauri::command]
pub async fn create_project_team_skill(
    app: AppHandle,
    project_name: String,
    project_description: String,
    workspace_path: String,
) -> Result<String, String> {
    use std::path::Path;
    use std::fs;

    log::info!("Creating project team skill for: {}", project_name);

    // 1. Create skill directory
    let skill_dir = Path::new(&workspace_path)
        .join(".claude")
        .join("skills")
        .join("create-project-team");

    fs::create_dir_all(&skill_dir)
        .map_err(|e| format!("Failed to create skill directory: {}", e))?;

    // 2. Write SKILL.md file from template
    let skill_content = include_str!("templates/create_project_team_skill.md");
    let skill_path = skill_dir.join("SKILL.md");
    fs::write(&skill_path, skill_content)
        .map_err(|e| format!("Failed to write SKILL.md: {}", e))?;

    log::info!("SKILL.md created at: {:?}", skill_path);

    // 3. Find Claude binary and invoke the skill
    let claude_path = crate::claude_binary::find_claude_binary(&app)?;

    // Build the skill invocation command
    let skill_invocation = format!(
        "/create-project-team \"{}\" \"{}\" {}",
        project_name, project_description, workspace_path
    );

    // Execute Claude Code with the skill invocation
    let mut cmd = std::process::Command::new(&claude_path);
    cmd.arg("--print")
        .arg("--init")
        .arg("--dangerously-skip-permissions")
        .arg(&skill_invocation)
        .current_dir(&workspace_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    log::info!("Executing: claude --print --init --dangerously-skip-permissions {}", skill_invocation);

    let output = cmd.output()
        .map_err(|e| format!("Failed to execute Claude Code: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        log::error!("Claude Code execution failed: {}", stderr);
        return Err(format!("Skill execution failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    log::info!("Skill execution output: {}", stdout);

    Ok(format!("Skill created and executed successfully. Output:\n{}", stdout))
}

/// Team member data from skill output
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TeamMember {
    pub agent_id: String,
    pub name: String,
    pub nickname: Option<String>,
    pub gender: Option<String>,
    pub agent_type: String,
    pub model: Option<String>,
    pub prompt: Option<String>,
    pub color: Option<String>,
    pub cwd: Option<String>,
    pub role_type: String, // "teamlead" or "teammate"
}

/// Execute project team skill and parse members from output
pub async fn execute_project_team_skill(
    app: &AppHandle,
    project_id: String,
    project_name: String,
    project_description: String,
    workspace_path: String,
    existing_teamlead: Option<SelectedTeamlead>,
) -> Result<Vec<TeamMember>, String> {
    use std::path::Path;
    use std::fs;

    log::info!("Executing project team skill for: {}", project_name);

    // Emit progress: preparing
    let _ = app.emit("project-progress", serde_json::json!({
        "project_id": project_id,
        "step": "preparing",
        "message": "准备环境..."
    }));

    // 1. Create skill directory
    let skill_dir = Path::new(&workspace_path)
        .join(".claude")
        .join("skills")
        .join("create-project-team");

    fs::create_dir_all(&skill_dir)
        .map_err(|e| format!("Failed to create skill directory: {}", e))?;

    // Emit progress: writing skill file
    let _ = app.emit("project-progress", serde_json::json!({
        "project_id": project_id,
        "step": "writing_skill",
        "message": "写入 Skill 文件..."
    }));

    // 2. Write SKILL.md file (same as create_project_team_skill)
    let skill_content = include_str!("templates/create_project_team_skill.md");
    let skill_path = skill_dir.join("SKILL.md");
    fs::write(&skill_path, skill_content)
        .map_err(|e| format!("Failed to write SKILL.md: {}", e))?;

    log::info!("SKILL.md created at: {:?}", skill_path);

    // Emit progress: finding claude
    let _ = app.emit("project-progress", serde_json::json!({
        "project_id": project_id,
        "step": "finding_claude",
        "message": "查找 Claude..."
    }));

    // 3. Find Claude binary and invoke the skill
    let claude_path = crate::claude_binary::find_claude_binary(app)?;

    // Build the skill invocation command
    let skill_invocation = format!(
        "/create-project-team \"{}\" \"{}\" {}",
        project_name, project_description, workspace_path
    );

    // Execute Claude Code with the skill invocation
    let mut cmd = std::process::Command::new(&claude_path);
    cmd.arg("--print")
        .arg("--init")
        .arg("--dangerously-skip-permissions")
        .arg(&skill_invocation)
        .current_dir(&workspace_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    log::info!("Executing Claude Code...");
    log::debug!("Claude command: claude --print --init --dangerously-skip-permissions {}", skill_invocation);
    log::info!("Working directory: {}", workspace_path);

    // Emit initial progress: executing_claude (40%)
    let app_for_progress = app.clone();
    let project_id_for_progress = project_id.clone();
    let _ = app_for_progress.emit("project-progress", serde_json::json!({
        "project_id": project_id_for_progress,
        "step": "executing_claude",
        "message": "正在调用 Claude Code... (40%)"
    }));

    // Spawn background task to send random progress updates while Claude is executing
    // Use tokio::task::spawn_blocking for better compatibility with async context
    let app_for_random_progress = app.clone();
    let project_id_for_random = project_id.clone();
    let handle = tokio::task::spawn_blocking(move || {
        use std::time::Duration;
        use rand::Rng;

        let mut rng = rand::thread_rng();

        // Random number of progress updates: 5-10
        let num_updates = rng.gen_range(5..=10);
        log::info!("[Random Progress] Will send {} progress updates", num_updates);

        for i in 1..=num_updates {
            // Wait for random duration (0.5s - 2s) between updates
            let wait_time = Duration::from_millis(rng.gen_range(500..2000));
            std::thread::sleep(wait_time);

            // Calculate progress: 40% -> 90%
            let progress = 40 + (i as f64 / num_updates as f64 * 50.0) as u32;
            let message = format!("正在调用 Claude Code... ({}%)", progress);

            log::info!("[Random Progress] Emitting event with project_id: {}", project_id_for_random);
            let _ = app_for_random_progress.emit("project-progress", serde_json::json!({
                "project_id": project_id_for_random,
                "step": "executing_claude",
                "message": message
            }));

            log::info!("[Random Progress] Sent progress update {}/{}: {}%", i, num_updates, progress);
        }

        log::info!("[Random Progress] Completed all {} progress updates", num_updates);
    });

    let output = cmd.output()
        .map_err(|e| format!("Failed to execute Claude Code: {}", e))?;

    // Wait for random progress task to finish
    let _ = handle.await;

    log::info!("Claude Code process finished, exit code: {:?}", output.status.code());

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        log::error!("Claude Code execution failed with non-zero exit code");
        log::error!("stderr: {}", stderr);
        return Err(format!("Skill execution failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    log::info!("Claude Code output length: {} chars", stdout.len());
    log::debug!("Claude Code output preview: {}", &stdout[..stdout.len().min(500)]);

    // Emit progress: parsing json
    let _ = app.emit("project-progress", serde_json::json!({
        "project_id": project_id,
        "step": "parsing_json",
        "message": "正在解析 JSON..."
    }));

    // 4. Parse the config.json from output
    log::info!("Parsing JSON from Claude output...");
    let mut members = parse_team_members_from_output(&stdout)?;
    log::info!("Successfully parsed {} team members", members.len());

    // 5. If existing_teamlead is provided, replace lead agent's name, nickname, gender
    if let Some(ref teamlead) = existing_teamlead {
        log::info!("Replacing lead agent with existing teamlead: {}", teamlead.name);
        for member in members.iter_mut() {
            if member.role_type == "teamlead" {
                member.name = teamlead.name.clone();
                member.nickname = teamlead.nickname.clone();
                member.gender = teamlead.gender.clone();
                // Also update agent_id to match the existing teamlead
                member.agent_id = format!("{}@{}", teamlead.name, project_name);
                log::info!("Lead agent replaced: name={}, nickname={}, gender={}",
                    member.name, member.nickname.as_deref().unwrap_or(""), member.gender.as_deref().unwrap_or(""));
                break;
            }
        }

        // 6. Update config.json file with replaced teamlead info
        // Extract configPath from stdout
        let config_path = find_json_in_output(&stdout)
            .and_then(|json_str| {
                serde_json::from_str::<serde_json::Value>(json_str)
                    .ok()
                    .and_then(|v| {
                        v.get("configPath")
                            .and_then(|p| p.as_str())
                            .map(|s| s.to_string())
                    })
            });

        if let Some(config_path_str) = config_path {
            let config_path = std::path::Path::new(&config_path_str);

            if config_path.exists() {
                log::info!("Updating config.json at: {:?}", config_path);

                // Re-read the existing config.json
                if let Ok(config_content) = fs::read_to_string(&config_path) {
                    if let Ok(mut config_json) = serde_json::from_str::<serde_json::Value>(&config_content) {
                        // Update leadAgentId
                        if let Some(lead_member) = members.iter().find(|m| m.role_type == "teamlead") {
                            if let Some(obj) = config_json.get_mut("leadAgentId") {
                                *obj = serde_json::Value::String(lead_member.agent_id.clone());
                            }

                            // Update members array
                            if let Some(members_arr) = config_json.get_mut("members").and_then(|m| m.as_array_mut()) {
                                for m in members_arr {
                                    if let Some(agent_id) = m.get("agentId").and_then(|v| v.as_str()) {
                                        if agent_id.contains(&project_name) || agent_id.contains("lead") {
                                            if let Some(obj) = m.as_object_mut() {
                                                obj.insert("agentId".to_string(), serde_json::Value::String(lead_member.agent_id.clone()));
                                                obj.insert("name".to_string(), serde_json::Value::String(lead_member.name.clone()));
                                                if let Some(nickname) = &lead_member.nickname {
                                                    obj.insert("nickname".to_string(), serde_json::Value::String(nickname.clone()));
                                                }
                                                if let Some(gender) = &lead_member.gender {
                                                    obj.insert("gender".to_string(), serde_json::Value::String(gender.clone()));
                                                }
                                                if let Some(prompt) = &lead_member.prompt {
                                                    obj.insert("prompt".to_string(), serde_json::Value::String(prompt.clone()));
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            // Write updated config.json
                            if let Ok(updated_content) = serde_json::to_string_pretty(&config_json) {
                                if let Err(e) = fs::write(&config_path, updated_content) {
                                    log::error!("Failed to write config.json: {}", e);
                                } else {
                                    log::info!("Successfully updated config.json with replaced teamlead info");
                                }
                            }
                        }
                    }
                }
            } else {
                log::warn!("Config.json not found at: {:?}", config_path);
            }
        }
    }

    // 7. If existing_teamlead is provided, filter out the teamlead from members
    // (the existing teamlead will be reused, no need to create a new one)
    if existing_teamlead.is_some() {
        let original_count = members.len();
        members.retain(|m| m.role_type != "teamlead");
        log::info!("Filtered out teamlead member: {} -> {} members", original_count, members.len());
    }

    Ok(members)
}

/// Parse team members from skill output
fn parse_team_members_from_output(output: &str) -> Result<Vec<TeamMember>, String> {
    log::debug!("Searching for JSON in output...");

    // Try to find JSON in the output
    let json_str = find_json_in_output(output)
        .ok_or("Could not find JSON in skill output")?;

    log::debug!("Found JSON, length: {} chars", json_str.len());

    // Parse the JSON
    let parsed: serde_json::Value = serde_json::from_str(json_str)
        .map_err(|e| format!("Failed to parse JSON: {}", e))?;

    log::debug!("JSON parsed successfully");

    // Get leadAgentId to determine role_type
    let lead_agent_id = parsed.get("leadAgentId")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    log::debug!("Lead agent ID: {}", lead_agent_id);

    let members_array = parsed.get("members")
        .and_then(|m| m.as_array())
        .ok_or("No members array in JSON")?;

    log::debug!("Found {} members in JSON", members_array.len());

    let mut members = Vec::new();
    for m in members_array {
        let agent_id = m.get("agentId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Determine role_type: teamlead if agentId == leadAgentId, else teammate
        let role_type = if agent_id == lead_agent_id {
            "teamlead"
        } else {
            "teammate"
        };
        let agent_id = m.get("agentId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let name = m.get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let nickname = m.get("nickname")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let gender = m.get("gender")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let agent_type = m.get("agentType")
            .and_then(|v| v.as_str())
            .unwrap_or("general-purpose")
            .to_string();
        let model = m.get("model")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let prompt = m.get("prompt")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let color = m.get("color")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let cwd = m.get("cwd")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        members.push(TeamMember {
            agent_id,
            name,
            nickname,
            gender,
            agent_type,
            model,
            prompt,
            color,
            cwd,
            role_type: role_type.to_string(),
        });
    }

    Ok(members)
}

/// Find JSON object in output string
fn find_json_in_output(output: &str) -> Option<&str> {
    // Find the first { and last }
    let start = output.find('{')?;
    let end = output.rfind('}')?;
    if end > start {
        Some(&output[start..=end])
    } else {
        None
    }
}
