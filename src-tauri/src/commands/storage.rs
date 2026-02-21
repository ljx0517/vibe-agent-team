use super::agents::AgentDb;
use anyhow::Result;
use rusqlite::{params, types::ValueRef, Connection, Result as SqliteResult};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as JsonValue};
use std::collections::HashMap;
use tauri::{AppHandle, Manager, State};
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

/// Project creation input
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateProjectInput {
    pub name: String,
    pub project_code: Option<String>,
    pub description: String,
    pub work_dir: String,
}

/// Project creation result
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateProjectResult {
    pub project_id: String,
    pub workspace_id: String,
}

/// Create a new project with associated workspace
#[tauri::command]
pub async fn storage_create_project(
    db: State<'_, AgentDb>,
    app: AppHandle,
    input: CreateProjectInput,
) -> Result<CreateProjectResult, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;

    let project_id = Uuid::new_v4().to_string();
    let workspace_id = Uuid::new_v4().to_string();

    // Insert into projects table
    conn.execute(
        "INSERT INTO projects (id, name, project_code, working_dir, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, datetime('now'), datetime('now'))",
        params![project_id, input.name, input.project_code, input.work_dir],
    )
    .map_err(|e| format!("创建项目失败: {}", e))?;

    // Insert into workspaces table with the project path and name
    conn.execute(
        "INSERT INTO workspaces (id, name, path, created_at, updated_at)
         VALUES (?1, ?2, ?3, datetime('now'), datetime('now'))",
        params![workspace_id, input.name, input.work_dir],
    )
    .map_err(|e| format!("创建工作空间失败: {}", e))?;
    log::info!(
        "create workspace in: {}",
        input.work_dir
    );

    // Build the team formation prompt
    let prompt = format!(
        "我(产品经理)正在为一个项目组建一个开发团队(agent team)，项目名叫\"{}\"，你叫\"王大锤\"作为Team Lead还需要一个评审人叫\"李小锤\"，\
        李小锤在项目团队内充当devil's advocate的角色，李小锤是一个资深的IT专家，对一个项目从立项到最后的增长运维的各个环节都是专家，\
        目前团队(agent team)内只有两个人, 后续环节人员待定，现在只需要把团队(agent team)组建好就行了。无需确认。",
        input.name
    );

    // Call Claude to form the team in background (don't wait for completion)
    let app_clone = app.clone();
    let project_path = input.work_dir.clone();
    tokio::spawn(async move {
        // Import the claude module commands
        use crate::commands::claude::execute_claude_code;

        // Use default model "sonnet" if not specified
        let model = "sonnet".to_string();

        match execute_claude_code(app_clone, project_path, prompt, model).await {
            Ok(_) => {
                log::info!("Claude team formation completed successfully");
            }
            Err(e) => {
                log::error!("Failed to communicate with Claude: {}", e);
            }
        }
    });

    Ok(CreateProjectResult {
        project_id,
        workspace_id,
    })
}

/// Project with workspace info
#[derive(Debug, Serialize, Deserialize)]
pub struct ProjectWithWorkspace {
    pub project_id: String,
    pub project_name: String,
    pub project_code: Option<String>,
    pub workspace_id: String,
    pub workspace_path: String,
}

/// List all projects with their workspaces
#[tauri::command]
pub async fn storage_list_projects(
    db: State<'_, AgentDb>,
) -> Result<Vec<ProjectWithWorkspace>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;

    let mut stmt = conn
        .prepare(
            "SELECT p.id, p.name, p.project_code, w.id, w.path
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
                workspace_id: row.get(3)?,
                workspace_path: row.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    Ok(projects)
}
