//! Workspace 管理模块
//! 提供 Git Worktree 和多仓库工作空间的创建、清理、管理功能
//!
//! # 快速开始
//!
//! ```rust
//! use std::path::PathBuf;
//! use workspace::{WorkspaceManager, WorktreeManager, RepoInput, WorktreeCleanup};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // 1. 设置工作空间目录（可选）
//!     WorktreeManager::set_workspace_dir_override(PathBuf::from("/custom/path"));
//!
//!     // 2. 创建单仓库 worktree
//!     WorktreeManager::create_worktree(
//!         &PathBuf::from("/path/to/repo"),
//!         "feature-branch",
//!         &PathBuf::from("/tmp/worktrees/my-feature"),
//!         "main",
//!         true,
//!     ).await?;
//!
//!     // 3. 创建多仓库工作空间
//!     let repos = vec![
//!         RepoInput {
//!             id: "1".to_string(),
//!             name: "frontend".to_string(),
//!             path: PathBuf::from("/projects/frontend"),
//!             target_branch: "main".to_string(),
//!         },
//!         RepoInput {
//!             id: "2".to_string(),
//!             name: "backend".to_string(),
//!             path: PathBuf::from("/projects/backend"),
//!             target_branch: "main".to_string(),
//!         },
//!     ];
//!
//!     let workspace = WorkspaceManager::create_workspace(
//!         &PathBuf::from("/tmp/workspaces/ws-123"),
//!         &repos,
//!         "task-123",
//!     ).await?;
//!
//!     println!("Workspace created at: {}", workspace.workspace_dir.display());
//!
//!     // 4. 清理工作空间
//!     WorkspaceManager::cleanup_workspace(&workspace.worktrees).await?;
//!
//!     Ok(())
//! }
//! ```

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, LazyLock, Mutex, OnceLock},
};

// ============================================================================
// 类型定义
// ============================================================================

/// Worktree 清理数据结构
#[derive(Debug, Clone)]
pub struct WorktreeCleanup {
    /// Worktree 路径
    pub worktree_path: PathBuf,
    /// Git 仓库路径（可选）
    pub git_repo_path: Option<PathBuf>,
}

impl WorktreeCleanup {
    /// 创建新的清理数据
    pub fn new(worktree_path: PathBuf, git_repo_path: Option<PathBuf>) -> Self {
        Self {
            worktree_path,
            git_repo_path,
        }
    }
}

/// 单个仓库的工作空间信息
#[derive(Debug, Clone)]
pub struct RepoWorktree {
    /// 仓库 ID
    pub repo_id: String,
    /// 仓库名称
    pub repo_name: String,
    /// 源仓库路径
    pub source_repo_path: PathBuf,
    /// Worktree 路径
    pub worktree_path: PathBuf,
}

/// 仓库输入参数
#[derive(Debug, Clone)]
pub struct RepoInput {
    /// 仓库 ID
    pub id: String,
    /// 仓库名称
    pub name: String,
    /// 仓库本地路径
    pub path: PathBuf,
    /// 目标分支
    pub target_branch: String,
}

/// 工作空间容器
#[derive(Debug, Clone)]
pub struct WorktreeContainer {
    /// 工作空间根目录
    pub workspace_dir: PathBuf,
    /// 创建的 worktree 列表
    pub worktrees: Vec<RepoWorktree>,
}

// ============================================================================
// 错误类型
// ============================================================================

/// Workspace 模块错误类型
#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    /// Git 操作错误
    #[error("Git error: {0}")]
    Git(String),
    /// IO 错误
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    /// 无效路径
    #[error("Invalid path: {0}")]
    InvalidPath(String),
    /// 仓库错误
    #[error("Repository error: {0}")]
    Repository(String),
    /// 没有提供仓库
    #[error("No repositories provided")]
    NoRepositories,
    /// 部分创建失败
    #[error("Partial creation failed: {0}")]
    PartialCreation(String),
    /// 任务_join 错误
    #[error("Task join error: {0}")]
    TaskJoin(String),
}

impl From<git2::Error> for WorkspaceError {
    fn from(e: git2::Error) -> Self {
        WorkspaceError::Git(e.message().to_string())
    }
}

// ============================================================================
// 全局状态
// ============================================================================

static WORKSPACE_DIR_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();
static WORKTREE_CREATION_LOCKS: LazyLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

// ============================================================================
// 核心实现
// ============================================================================

/// Worktree 管理器 - 用于管理单个 Git worktree
pub struct WorktreeManager;

impl WorktreeManager {
    /// 设置工作空间目录覆盖（可选，用于自定义路径）
    ///
    /// # 示例
    /// ```
    /// WorktreeManager::set_workspace_dir_override(PathBuf::from("/custom/path"));
    /// ```
    pub fn set_workspace_dir_override(path: PathBuf) {
        let _ = WORKSPACE_DIR_OVERRIDE.set(path);
    }

    /// 获取工作空间基础目录
    ///
    /// 默认路径：
    /// - macOS: `/tmp/vibe-kanban/worktrees`
    /// - Linux: `/var/tmp/vibe-kanban/worktrees`
    /// - 其他: `/tmp/vibe-kanban/worktrees`
    pub fn get_worktree_base_dir() -> PathBuf {
        if let Some(override_path) = WORKSPACE_DIR_OVERRIDE.get() {
            return override_path.join(".vibe-kanban-workspaces");
        }
        Self::get_default_worktree_base_dir()
    }

    /// 获取默认工作空间基础目录
    pub fn get_default_worktree_base_dir() -> PathBuf {
        let dir_name = if cfg!(debug_assertions) {
            "vibe-kanban-dev"
        } else {
            "vibe-kanban"
        };

        if cfg!(target_os = "macos") {
            std::env::temp_dir().join(dir_name).join("worktrees")
        } else if cfg!(target_os = "linux") {
            PathBuf::from("/var/tmp").join(dir_name).join("worktrees")
        } else {
            std::env::temp_dir().join(dir_name).join("worktrees")
        }
    }

    /// 创建 worktree（可选创建新分支）
    ///
    /// # 参数
    /// - `repo_path`: 源 Git 仓库路径
    /// - `branch_name`: 要创建/检出的分支名
    /// - `worktree_path`: worktree 将被创建的位置
    /// - `base_branch`: 创建分支时的基础分支
    /// - `create_branch`: 是否创建新分支
    ///
    /// # 示例
    /// ```ignore
    /// WorktreeManager::create_worktree(
    ///     &PathBuf::from("/path/to/repo"),
    ///     "feature-branch",
    ///     &PathBuf::from("/tmp/worktrees/feature"),
    ///     "main",
    ///     true,  // 创建新分支
    /// ).await?;
    /// ```
    pub async fn create_worktree(
        repo_path: &Path,
        branch_name: &str,
        worktree_path: &Path,
        base_branch: &str,
        create_branch: bool,
    ) -> Result<(), WorkspaceError> {
        // 如果需要创建分支
        if create_branch {
            let repo_path_owned = repo_path.to_path_buf();
            let branch_name_owned = branch_name.to_string();
            let base_branch_owned = base_branch.to_string();

            tokio::task::spawn_blocking(move || {
                let repo = git2::Repository::open(&repo_path_owned)?;
                let base_branch_ref =
                    repo.find_branch(&base_branch_owned, git2::BranchType::Local)?;
                repo.branch(
                    &branch_name_owned,
                    &base_branch_ref.get().peel_to_commit()?,
                    false,
                )?;
                Ok::<(), WorkspaceError>(())
            })
            .await
            .map_err(|e| WorkspaceError::TaskJoin(e.to_string()))??;
        }

        Self::ensure_worktree_exists(repo_path, branch_name, worktree_path).await
    }

    /// 确保 worktree 存在（不存在则创建）
    pub async fn ensure_worktree_exists(
        repo_path: &Path,
        branch_name: &str,
        worktree_path: &Path,
    ) -> Result<(), WorkspaceError> {
        let path_str = worktree_path.to_string_lossy().to_string();

        // 获取或创建路径锁
        let lock = {
            let mut locks = WORKTREE_CREATION_LOCKS.lock().unwrap();
            locks
                .entry(path_str.clone())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };

        let _guard = lock.lock().await;

        // 检查 worktree 是否已正确设置
        if Self::is_worktree_properly_set_up(repo_path, worktree_path).await? {
            tracing::trace!("Worktree already set up: {}", path_str);
            return Ok(());
        }

        // 重新创建 worktree
        tracing::info!("Creating worktree at: {}", path_str);
        Self::recreate_worktree_internal(repo_path, branch_name, worktree_path).await
    }

    /// 检查 worktree 是否正确设置
    async fn is_worktree_properly_set_up(
        repo_path: &Path,
        worktree_path: &Path,
    ) -> Result<bool, WorkspaceError> {
        let repo_path = repo_path.to_path_buf();
        let worktree_path = worktree_path.to_path_buf();

        tokio::task::spawn_blocking(move || -> Result<bool, WorkspaceError> {
            // 检查 1: 文件系统路径必须存在
            if !worktree_path.exists() {
                return Ok(false);
            }

            // 检查 2: 必须在 git 元数据中注册
            let repo = git2::Repository::open(&repo_path)?;
            let Some(worktree_name) =
                Self::find_worktree_git_internal_name(&repo_path, &worktree_path)?
            else {
                return Ok(false);
            };

            // 尝试查找 worktree
            match repo.find_worktree(&worktree_name) {
                Ok(_) => Ok(true),
                Err(_) => Ok(false),
            }
        })
        .await
        .map_err(|e| WorkspaceError::TaskJoin(e.to_string()))?
    }

    /// 查找 worktree 的内部名称
    fn find_worktree_git_internal_name(
        git_repo_path: &Path,
        worktree_path: &Path,
    ) -> Result<Option<String>, WorkspaceError> {
        fn canonicalize_for_compare(path: &Path) -> PathBuf {
            dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
        }

        let worktree_root = canonicalize_for_compare(worktree_path);
        let worktree_metadata_path = Self::get_worktree_metadata_path(git_repo_path)?;

        let worktree_metadata_folders = match fs::read_dir(&worktree_metadata_path) {
            Ok(read_dir) => read_dir
                .filter_map(|entry| entry.ok())
                .collect::<Vec<fs::DirEntry>>(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(WorkspaceError::Repository(format!(
                    "Failed to read worktree metadata: {}",
                    e
                )));
            }
        };

        for entry in worktree_metadata_folders {
            let gitdir_path = entry.path().join("gitdir");
            if gitdir_path.exists() {
                let gitdir_content = match fs::read_to_string(&gitdir_path) {
                    Ok(content) => content,
                    Err(_) => continue,
                };
                let linked_path = Path::new(gitdir_content.trim());
                if canonicalize_for_compare(linked_path.parent().unwrap_or(linked_path))
                    == worktree_root
                {
                    return Ok(Some(entry.file_name().to_string_lossy().to_string()));
                }
            }
        }
        Ok(None)
    }

    fn get_worktree_metadata_path(git_repo_path: &Path) -> Result<PathBuf, WorkspaceError> {
        let repo = git2::Repository::open(git_repo_path)?;
        Ok(repo.commondir().join("worktrees"))
    }

    /// 重新创建 worktree（内部使用）
    async fn recreate_worktree_internal(
        repo_path: &Path,
        branch_name: &str,
        worktree_path: &Path,
    ) -> Result<(), WorkspaceError> {
        let worktree_path_owned = worktree_path.to_path_buf();

        // 步骤 1: 清理现有 worktree
        Self::comprehensive_cleanup_async(repo_path, &worktree_path_owned).await?;

        // 步骤 2: 确保父目录存在
        if let Some(parent) = worktree_path_owned.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        // 步骤 3: 创建 worktree
        Self::create_worktree_with_retry(repo_path, branch_name, &worktree_path_owned).await
    }

    /// 带重试的 worktree 创建
    async fn create_worktree_with_retry(
        git_repo_path: &Path,
        branch_name: &str,
        worktree_path: &Path,
    ) -> Result<(), WorkspaceError> {
        let git_repo_path = git_repo_path.to_path_buf();
        let branch_name = branch_name.to_string();
        let worktree_path = worktree_path.to_path_buf();

        tokio::task::spawn_blocking(move || -> Result<(), WorkspaceError> {
            // 尝试直接创建
            match Self::git_worktree_add(&git_repo_path, &worktree_path, &branch_name, false) {
                Ok(()) => {
                    if !worktree_path.exists() {
                        return Err(WorkspaceError::Repository(
                            "Worktree creation failed: path does not exist".to_string(),
                        ));
                    }
                    tracing::info!(
                        "Created worktree {} at {}",
                        branch_name,
                        worktree_path.display()
                    );
                    Ok(())
                }
                Err(e) => {
                    tracing::warn!("Worktree add failed, retrying: {}", e);
                    // 清理元数据后重试
                    let _ = Self::force_cleanup_metadata(&git_repo_path, &worktree_path);
                    if worktree_path.exists() {
                        fs::remove_dir_all(&worktree_path)?;
                    }
                    Self::git_worktree_add(&git_repo_path, &worktree_path, &branch_name, false)?;
                    tracing::info!("Created worktree {} after retry", branch_name);
                    Ok(())
                }
            }
        })
        .await
        .map_err(|e| WorkspaceError::TaskJoin(e.to_string()))?
    }

    /// Git CLI: 创建 worktree
    fn git_worktree_add(
        repo_path: &Path,
        worktree_path: &Path,
        branch: &str,
        create_branch: bool,
    ) -> Result<(), WorkspaceError> {
        let mut cmd = Command::new("git");
        cmd.args(["-C", &repo_path.to_string_lossy(), "worktree", "add"]);

        if create_branch {
            cmd.arg("-b");
            cmd.arg(branch);
        }

        cmd.arg(&worktree_path);
        cmd.arg(branch);

        let output = cmd.output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(WorkspaceError::Git(stderr.to_string()));
        }
        Ok(())
    }

    /// 强制清理 worktree 元数据
    fn force_cleanup_metadata(
        git_repo_path: &Path,
        worktree_path: &Path,
    ) -> Result<(), WorkspaceError> {
        if let Some(worktree_name) =
            Self::find_worktree_git_internal_name(git_repo_path, worktree_path)?
        {
            let metadata_path =
                Self::get_worktree_metadata_path(git_repo_path)?.join(&worktree_name);
            if metadata_path.exists() {
                fs::remove_dir_all(&metadata_path)?;
            }
        }
        Ok(())
    }

    /// 综合清理（异步）
    async fn comprehensive_cleanup_async(
        git_repo_path: &Path,
        worktree_path: &Path,
    ) -> Result<(), WorkspaceError> {
        let git_repo_path = git_repo_path.to_path_buf();
        let worktree_path = worktree_path.to_path_buf();

        // 尝试打开仓库
        let repo_result: Result<git2::Repository, git2::Error> =
            tokio::task::spawn_blocking(move || git2::Repository::open(&git_repo_path))
                .await
                .map_err(|e| WorkspaceError::TaskJoin(e.to_string()))?;

        match repo_result {
            Ok(repo) => tokio::task::spawn_blocking(move || {
                Self::comprehensive_cleanup(&repo, &worktree_path)
            })
            .await
            .map_err(|e| WorkspaceError::TaskJoin(e.to_string()))?,
            Err(_) => {
                // 仓库不存在，简单清理
                Self::simple_cleanup(&worktree_path).await
            }
        }
    }

    /// 综合清理（同步）
    fn comprehensive_cleanup(
        repo: &git2::Repository,
        worktree_path: &Path,
    ) -> Result<(), WorkspaceError> {
        let git_repo_path = repo
            .workdir()
            .ok_or_else(|| WorkspaceError::Repository("No working directory".into()))?
            .to_path_buf();

        // 1. 使用 git worktree remove
        let _ = Self::git_worktree_remove(&git_repo_path, worktree_path, true);

        // 2. 清理元数据
        let _ = Self::force_cleanup_metadata(&git_repo_path, worktree_path);

        // 3. 清理物理目录
        if worktree_path.exists() {
            fs::remove_dir_all(worktree_path)?;
        }

        // 4. 修剪孤立 worktree
        let _ = Self::git_worktree_prune(&git_repo_path);

        Ok(())
    }

    /// Git CLI: 移除 worktree
    fn git_worktree_remove(
        repo_path: &Path,
        worktree_path: &Path,
        force: bool,
    ) -> Result<(), WorkspaceError> {
        let mut cmd = Command::new("git");
        cmd.args(["-C", &repo_path.to_string_lossy(), "worktree", "remove"]);
        if force {
            cmd.arg("--force");
        }
        cmd.arg(&worktree_path);

        let output = cmd.output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::debug!("Worktree remove warning: {}", stderr);
        }
        Ok(())
    }

    /// Git CLI: 修剪 worktree
    fn git_worktree_prune(repo_path: &Path) -> Result<(), WorkspaceError> {
        let output = Command::new("git")
            .args(["-C", &repo_path.to_string_lossy(), "worktree", "prune"])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::debug!("Worktree prune warning: {}", stderr);
        }
        Ok(())
    }

    /// 简单清理（仅删除目录）
    async fn simple_cleanup(worktree_path: &Path) -> Result<(), WorkspaceError> {
        let worktree_path = worktree_path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            if worktree_path.exists() {
                fs::remove_dir_all(&worktree_path)?;
            }
            Ok::<(), WorkspaceError>(())
        })
        .await
        .map_err(|e| WorkspaceError::TaskJoin(e.to_string()))?
    }

    /// 清理 worktree
    ///
    /// # 示例
    /// ```ignore
    /// let cleanup = WorktreeCleanup::new(
    ///     PathBuf::from("/tmp/worktrees/feature"),
    ///     Some(PathBuf::from("/path/to/repo")),
    /// );
    /// WorktreeManager::cleanup_worktree(&cleanup).await?;
    /// ```
    pub async fn cleanup_worktree(cleanup: &WorktreeCleanup) -> Result<(), WorkspaceError> {
        let path_str = cleanup.worktree_path.to_string_lossy().to_string();

        // 获取锁
        let lock = {
            let mut locks = WORKTREE_CREATION_LOCKS.lock().unwrap();
            locks
                .entry(path_str.clone())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };

        let _guard = lock.lock().await;

        // 确定 git 仓库路径
        let resolved_repo_path = if let Some(ref repo_path) = cleanup.git_repo_path {
            Some(repo_path.to_path_buf())
        } else {
            Self::infer_git_repo_path(&cleanup.worktree_path).await
        };

        if let Some(repo_path) = resolved_repo_path {
            Self::comprehensive_cleanup_async(&repo_path, &cleanup.worktree_path).await?;
        } else {
            Self::simple_cleanup(&cleanup.worktree_path).await?;
        }

        Ok(())
    }

    /// 推断 git 仓库路径
    async fn infer_git_repo_path(worktree_path: &Path) -> Option<PathBuf> {
        let output = tokio::process::Command::new("git")
            .args(["rev-parse", "--git-common-dir"])
            .current_dir(worktree_path)
            .output()
            .await
            .ok()?;

        if output.status.success() {
            let git_common_dir = String::from_utf8(output.stdout).ok()?.trim().to_string();
            let git_dir_path = Path::new(&git_common_dir);
            if git_dir_path.file_name() == Some(std::ffi::OsStr::new(".git")) {
                git_dir_path.parent()?.to_str().map(PathBuf::from)
            } else {
                Some(PathBuf::from(git_common_dir))
            }
        } else {
            None
        }
    }

    /// 批量清理 worktree
    pub async fn batch_cleanup_worktrees(
        cleanups: &[WorktreeCleanup],
    ) -> Result<(), WorkspaceError> {
        for cleanup in cleanups {
            if let Err(e) = Self::cleanup_worktree(cleanup).await {
                tracing::error!("Failed to cleanup worktree: {}", e);
            }
        }
        Ok(())
    }

    /// 移动 worktree
    pub async fn move_worktree(
        repo_path: &Path,
        old_path: &Path,
        new_path: &Path,
    ) -> Result<(), WorkspaceError> {
        let repo_path = repo_path.to_path_buf();
        let old_path = old_path.to_path_buf();
        let new_path = new_path.to_path_buf();

        tokio::task::spawn_blocking(move || {
            let output = Command::new("git")
                .args(["-C", &repo_path.to_string_lossy(), "worktree", "move"])
                .arg(&old_path)
                .arg(&new_path)
                .output()?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(WorkspaceError::Git(stderr.to_string()));
            }
            Ok(())
        })
        .await
        .map_err(|e| WorkspaceError::TaskJoin(e.to_string()))?
    }
}

// ============================================================================
// WorkspaceManager - 多仓库工作空间管理
// ============================================================================

/// 工作空间管理器 - 用于管理包含多个仓库的工作空间
pub struct WorkspaceManager;

impl WorkspaceManager {
    /// 创建工作空间（包含多个仓库的 worktree）
    ///
    /// # 参数
    /// - `workspace_dir`: 工作空间根目录
    /// - `repos`: 仓库输入列表
    /// - `创建的工作分支名branch_name`: 要
    ///
    /// # 示例
    /// ```ignore
    /// let repos = vec![
    ///     RepoInput {
    ///         id: "1".to_string(),
    ///         name: "frontend".to_string(),
    ///         path: PathBuf::from("/projects/frontend"),
    ///         target_branch: "main".to_string(),
    ///     },
    /// ];
    ///
    /// let workspace = WorkspaceManager::create_workspace(
    ///     &PathBuf::from("/tmp/workspaces/ws-123"),
    ///     &repos,
    ///     "task-123",
    /// ).await?;
    /// ```
    pub async fn create_workspace(
        workspace_dir: &Path,
        repos: &[RepoInput],
        branch_name: &str,
    ) -> Result<WorktreeContainer, WorkspaceError> {
        if repos.is_empty() {
            return Err(WorkspaceError::NoRepositories);
        }

        tracing::info!(
            "Creating workspace at {} with {} repositories",
            workspace_dir.display(),
            repos.len()
        );

        tokio::fs::create_dir_all(workspace_dir).await?;

        let mut created_worktrees = Vec::new();

        for repo in repos {
            let worktree_path = workspace_dir.join(&repo.name);

            match WorktreeManager::create_worktree(
                &repo.path,
                branch_name,
                &worktree_path,
                &repo.target_branch,
                true,
            )
            .await
            {
                Ok(()) => {
                    created_worktrees.push(RepoWorktree {
                        repo_id: repo.id.clone(),
                        repo_name: repo.name.clone(),
                        source_repo_path: repo.path.clone(),
                        worktree_path,
                    });
                }
                Err(e) => {
                    tracing::error!("Failed to create worktree for repo '{}': {}", repo.name, e);
                    // 回滚已创建 worktree
                    Self::rollback_worktrees(&created_worktrees).await;
                    return Err(WorkspaceError::PartialCreation(format!(
                        "Failed to create worktree for repo '{}': {}",
                        repo.name, e
                    )));
                }
            }
        }

        Ok(WorktreeContainer {
            workspace_dir: workspace_dir.to_path_buf(),
            worktrees: created_worktrees,
        })
    }

    /// 回滚已创建 worktree
    async fn rollback_worktrees(worktrees: &[RepoWorktree]) {
        for wt in worktrees {
            let cleanup =
                WorktreeCleanup::new(wt.worktree_path.clone(), Some(wt.source_repo_path.clone()));
            if let Err(e) = WorktreeManager::cleanup_worktree(&cleanup).await {
                tracing::error!("Rollback failed for {}: {}", wt.repo_name, e);
            }
        }
    }

    /// 清理工作空间
    ///
    /// # 示例
    /// ```ignore
    /// WorkspaceManager::cleanup_workspace(&workspace.worktrees).await?;
    /// ```
    pub async fn cleanup_workspace(worktrees: &[RepoWorktree]) -> Result<(), WorkspaceError> {
        let cleanups: Vec<WorktreeCleanup> = worktrees
            .iter()
            .map(|wt| {
                WorktreeCleanup::new(wt.worktree_path.clone(), Some(wt.source_repo_path.clone()))
            })
            .collect();

        WorktreeManager::batch_cleanup_worktrees(&cleanups).await
    }

    /// 获取工作空间基础目录
    pub fn get_workspace_base_dir() -> PathBuf {
        WorktreeManager::get_worktree_base_dir()
    }

    /// 创建临时工作空间目录
    ///
    /// # 示例
    /// ```ignore
    /// let workspace_dir = WorkspaceManager::create_temp_workspace_dir("task-123")?;
    /// ```
    pub fn create_temp_workspace_dir(prefix: &str) -> Result<PathBuf, WorkspaceError> {
        let base_dir = Self::get_workspace_base_dir();
        let workspace_id = uuid::Uuid::new_v4();
        let workspace_dir = base_dir.join(format!("{}-{}", prefix, workspace_id));
        Ok(workspace_dir)
    }
}
