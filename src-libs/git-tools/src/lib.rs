//! Vibe Git - Git operations library
//!
//! A hybrid Git library combining CLI (for safe working-tree operations)
//! with libgit2 (for read-only graph queries).

use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    io::Write as _,
    path::Path,
    process::{Command, Stdio},
};

use chrono::{DateTime, Utc};
use git2::{BranchType, DiffOptions, Repository, Sort};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ============================================================================
// 常量
// ============================================================================

const ALWAYS_SKIP_DIRS: &[&str] = &[];
const MAX_INLINE_DIFF_BYTES: usize = 2 * 1024 * 1024;

// ============================================================================
// 错误类型
// ============================================================================

#[derive(Debug, Error)]
pub enum GitCliError {
    #[error("git executable not found or not runnable")]
    NotAvailable,
    #[error("git command failed: {0}")]
    CommandFailed(String),
    #[error("authentication failed")]
    AuthFailed,
    #[error("push rejected")]
    PushRejected,
    #[error("rebase in progress in this worktree")]
    RebaseInProgress,
}

#[derive(Debug, Error)]
pub enum GitServiceError {
    #[error(transparent)]
    Git(#[from] git2::Error),
    #[error(transparent)]
    GitCli(#[from] GitCliError),
    #[error(transparent)]
    IoError(#[from] std::io::Error),
    #[error("Invalid repository: {0}")]
    InvalidRepository(String),
    #[error("Branch not found: {0}")]
    BranchNotFound(String),
    #[error("Merge conflicts: {0}")]
    MergeConflicts(String),
    #[error("Branches diverged: {0}")]
    BranchesDiverged(String),
    #[error("{0} has uncommitted changes: {1}")]
    WorktreeDirty(String, String),
    #[error("Rebase in progress")]
    RebaseInProgress,
}

// ============================================================================
// 数据类型
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diff {
    pub change: DiffChangeKind,
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub old_content: Option<String>,
    pub new_content: Option<String>,
    pub content_omitted: bool,
    pub additions: Option<usize>,
    pub deletions: Option<usize>,
    pub repo_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DiffChangeKind {
    Added,
    Deleted,
    Modified,
    Renamed,
    Copied,
    PermissionChange,
}

#[derive(Debug, Clone)]
pub struct Commit(git2::Oid);

#[derive(Debug, Clone)]
pub struct FileStat {
    pub last_index: usize,
    pub commit_count: u32,
    pub last_time: DateTime<Utc>,
}

impl Commit {
    pub fn new(id: git2::Oid) -> Self { Self(id) }
    pub fn as_oid(&self) -> git2::Oid { self.0 }
}

impl std::fmt::Display for Commit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GitBranch {
    pub name: String,
    pub is_current: bool,
    pub is_remote: bool,
    pub last_commit_date: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GitRemote {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct HeadInfo {
    pub branch: String,
    pub oid: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeType {
    Added, Modified, Deleted, Renamed, Copied, TypeChanged, Unmerged, Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusDiffEntry {
    pub change: ChangeType,
    pub path: String,
    pub old_path: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WorktreeEntry {
    pub path: String,
    pub branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusEntry {
    pub staged: char,
    pub unstaged: char,
    pub path: Vec<u8>,
    pub orig_path: Option<Vec<u8>>,
    pub is_untracked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeStatus {
    pub uncommitted_tracked: usize,
    pub untracked: usize,
    pub entries: Vec<StatusEntry>,
}

#[derive(Debug, Clone, Copy)]
pub struct WorktreeResetOptions {
    pub perform_reset: bool,
    pub force_when_dirty: bool,
    pub is_dirty: bool,
    pub log_skip_when_dirty: bool,
}

impl WorktreeResetOptions {
    pub fn new(perform_reset: bool, force_when_dirty: bool, is_dirty: bool, log_skip_when_dirty: bool) -> Self {
        Self { perform_reset, force_when_dirty, is_dirty, log_skip_when_dirty }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct WorktreeResetOutcome {
    pub needed: bool,
    pub applied: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictOp {
    Rebase, Merge, CherryPick, Revert,
}

pub struct StatusDiffOptions {
    pub path_filter: Option<Vec<String>>,
}

pub enum DiffTarget<'p> {
    Worktree {
        worktree_path: &'p Path,
        base_commit: &'p Commit,
    },
    Branch {
        repo_path: &'p Path,
        branch_name: &'p str,
        base_branch: &'p str,
    },
    Commit {
        repo_path: &'p Path,
        commit_sha: &'p str,
    },
}

// ============================================================================
// 辅助函数
// ============================================================================

fn compute_line_change_counts(old: &str, new: &str) -> (usize, usize) {
    let mut opts = DiffOptions::new();
    opts.context_lines(0);

    match git2::Patch::from_buffers(old.as_bytes(), None, new.as_bytes(), None, Some(&mut opts))
        .and_then(|patch| patch.line_stats())
    {
        Ok((_, adds, dels)) => (adds, dels),
        Err(_) => (0, 0)
    }
}

fn blob_to_string(blob: &git2::Blob) -> Option<String> {
    if blob.is_binary() { None } else { std::str::from_utf8(blob.content()).ok().map(|s| s.to_string()) }
}

// ============================================================================
// GitCli - CLI 封装
// ============================================================================

#[derive(Clone, Default)]
pub struct GitCli;

impl GitCli {
    pub fn new() -> Self { Self {} }

    fn ensure_available() -> Result<(), GitCliError> {
        let git = which::which("git").map_err(|_| GitCliError::NotAvailable)?;
        let out = Command::new(&git).arg("--version").output().map_err(|_| GitCliError::NotAvailable)?;
        if out.status.success() { Ok(()) } else { Err(GitCliError::NotAvailable) }
    }

    fn git_path() -> Result<std::path::PathBuf, GitCliError> {
        which::which("git").map_err(|_| GitCliError::NotAvailable)
    }

    fn git_impl<I, S>(repo_path: &Path, args: I, envs: Option<&[(OsString, OsString)]>, stdin: Option<&[u8]>) -> Result<Vec<u8>, GitCliError>
    where I: IntoIterator<Item = S>, S: AsRef<OsStr> {
        Self::ensure_available()?;
        let git = Self::git_path()?;
        let mut cmd = Command::new(&git);
        cmd.arg("-C").arg(repo_path);

        if let Some(envs) = envs { for (k, v) in envs { cmd.env(k, v); } }
        for a in args { cmd.arg(a); }

        if stdin.is_some() { cmd.stdin(Stdio::piped()); } else { cmd.stdin(Stdio::null()); }
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| GitCliError::CommandFailed(e.to_string()))?;
        if let Some(input) = stdin { if let Some(mut stdin) = child.stdin.take() { let _ = stdin.write_all(input); } }

        let out = child.wait_with_output().map_err(|e| GitCliError::CommandFailed(e.to_string()))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let combined = if stdout.is_empty() && stderr.is_empty() {
                "Command failed with no output".to_string()
            } else if stdout.is_empty() {
                stderr
            } else if stderr.is_empty() {
                stdout
            } else {
                format!("stderr: {}\nstdout: {}", stderr, stdout)
            };
            return Err(GitCliError::CommandFailed(combined));
        }
        Ok(out.stdout)
    }

    pub fn git<I, S>(&self, repo_path: &Path, args: I) -> Result<String, GitCliError>
    where I: IntoIterator<Item = S>, S: AsRef<OsStr> {
        let out = Self::git_impl(repo_path, args, None, None)?;
        Ok(String::from_utf8_lossy(&out).to_string())
    }

    fn git_with_env<I, S>(&self, repo_path: &Path, args: I, envs: &[(OsString, OsString)]) -> Result<String, GitCliError>
    where I: IntoIterator<Item = S>, S: AsRef<OsStr> {
        let out = Self::git_impl(repo_path, args, Some(envs), None)?;
        Ok(String::from_utf8_lossy(&out).to_string())
    }

    // --- Worktree ---
    pub fn worktree_add(&self, repo_path: &Path, worktree_path: &Path, branch: &str, create_branch: bool) -> Result<(), GitCliError> {
        Self::ensure_available()?;
        let mut args: Vec<OsString> = vec!["worktree".into(), "add".into()];
        if create_branch { args.push("-b".into()); args.push(OsString::from(branch)); }
        args.push(worktree_path.as_os_str().into());
        args.push(OsString::from(branch));
        self.git(repo_path, args)?;
        let _ = self.git(worktree_path, ["sparse-checkout", "reapply"]);
        Ok(())
    }

    pub fn worktree_remove(&self, repo_path: &Path, worktree_path: &Path, force: bool) -> Result<(), GitCliError> {
        Self::ensure_available()?;
        let mut args: Vec<OsString> = vec!["worktree".into(), "remove".into()];
        if force { args.push("--force".into()); }
        args.push(worktree_path.as_os_str().into());
        self.git(repo_path, args)?;
        Ok(())
    }

    pub fn worktree_move(&self, repo_path: &Path, old_path: &Path, new_path: &Path) -> Result<(), GitCliError> {
        Self::ensure_available()?;
        self.git(repo_path, ["worktree", "move", old_path.to_str().unwrap_or(""), new_path.to_str().unwrap_or("")])?;
        Ok(())
    }

    pub fn worktree_prune(&self, repo_path: &Path) -> Result<(), GitCliError> {
        self.git(repo_path, ["worktree", "prune"])?;
        Ok(())
    }

    pub fn list_worktrees(&self, repo_path: &Path) -> Result<Vec<WorktreeEntry>, GitCliError> {
        let out = self.git(repo_path, ["worktree", "list", "--porcelain"])?;
        let mut entries = Vec::new();
        let mut current_path: Option<String> = None;
        let mut current_head: Option<String> = None;
        let mut current_branch: Option<String> = None;

        for line in out.lines() {
            let line = line.trim();
            if line.is_empty() {
                if let (Some(path), Some(_head)) = (current_path.take(), current_head.take()) {
                    entries.push(WorktreeEntry { path, branch: current_branch.take() });
                }
            } else if let Some(p) = line.strip_prefix("worktree ") { current_path = Some(p.to_string()); }
            else if let Some(h) = line.strip_prefix("HEAD ") { current_head = Some(h.to_string()); }
            else if let Some(b) = line.strip_prefix("branch ") { current_branch = b.strip_prefix("refs/heads/").map(|s| s.to_string()); }
        }
        if let (Some(path), Some(_head)) = (current_path, current_head) {
            entries.push(WorktreeEntry { path, branch: current_branch });
        }
        Ok(entries)
    }

    // --- Status ---
    pub fn has_changes(&self, worktree_path: &Path) -> Result<bool, GitCliError> {
        let out = self.git(worktree_path, ["--no-optional-locks", "status", "--porcelain"])?;
        Ok(!out.is_empty())
    }

    pub fn get_worktree_status(&self, worktree_path: &Path) -> Result<WorktreeStatus, GitCliError> {
        let args: Vec<OsString> = vec!["--no-optional-locks".into(), "status".into(), "--porcelain".into(), "-z".into(), "--untracked-files=normal".into()];
        let out = Self::git_impl(worktree_path, args, None, None)?;
        let mut entries = Vec::new();
        let mut uncommitted_tracked = 0usize;
        let mut untracked = 0usize;
        let mut parts = out.split(|b| *b == 0);

        while let Some(part) = parts.next() {
            if part.is_empty() || part.len() < 4 { continue; }
            let staged = part[0] as char;
            let unstaged = part[1] as char;
            let path = part[3..].to_vec();
            let mut orig_path = None;
            if (staged == 'R' || staged == 'C' || unstaged == 'R' || unstaged == 'C')
                && let Some(old) = parts.next() { orig_path = Some(old.to_vec()); }

            if staged == '?' && unstaged == '?' {
                untracked += 1;
                entries.push(StatusEntry { staged, unstaged, path, orig_path, is_untracked: true });
            } else {
                if staged != ' ' || unstaged != ' ' { uncommitted_tracked += 1; }
                entries.push(StatusEntry { staged, unstaged, path, orig_path, is_untracked: false });
            }
        }
        Ok(WorktreeStatus { uncommitted_tracked, untracked, entries })
    }

    // --- Commit ---
    pub fn add_all(&self, worktree_path: &Path) -> Result<(), GitCliError> {
        self.git(worktree_path, ["add", "-A"])?;
        Ok(())
    }

    pub fn commit(&self, worktree_path: &Path, message: &str) -> Result<(), GitCliError> {
        self.git(worktree_path, ["commit", "-m", message])?;
        Ok(())
    }

    // --- Remote ---
    pub fn fetch_with_refspec(&self, repo_path: &Path, remote_url: &str, refspec: &str) -> Result<(), GitCliError> {
        let envs = vec![(OsString::from("GIT_TERMINAL_PROMPT"), OsString::from("0"))];
        let args: Vec<OsString> = vec!["fetch".into(), remote_url.into(), refspec.into()];
        match self.git_with_env(repo_path, args, &envs) {
            Ok(_) => Ok(()),
            Err(GitCliError::CommandFailed(_)) => Err(GitCliError::AuthFailed),
            Err(e) => Err(e),
        }
    }

    pub fn push(&self, repo_path: &Path, remote_url: &str, branch: &str, force: bool) -> Result<(), GitCliError> {
        let refspec = if force {
            format!("+refs/heads/{0}:refs/heads/{0}", branch)
        } else {
            format!("refs/heads/{0}:refs/heads/{0}", branch)
        };
        let envs = vec![(OsString::from("GIT_TERMINAL_PROMPT"), OsString::from("0"))];
        let args: Vec<OsString> = vec!["push".into(), remote_url.into(), refspec.into()];
        match self.git_with_env(repo_path, args, &envs) {
            Ok(_) => Ok(()),
            Err(GitCliError::CommandFailed(_)) => Err(GitCliError::PushRejected),
            Err(e) => Err(e),
        }
    }

    pub fn check_remote_branch_exists(&self, repo_path: &Path, remote_url: &str, branch_name: &str) -> Result<bool, GitCliError> {
        let envs = vec![(OsString::from("GIT_TERMINAL_PROMPT"), OsString::from("0"))];
        let refspec = format!("refs/heads/{0}", branch_name);
        let args: Vec<OsString> = vec!["ls-remote".into(), "--heads".into(), remote_url.into(), refspec.into()];
        match self.git_with_env(repo_path, args, &envs) {
            Ok(output) => Ok(!output.trim().is_empty()),
            Err(GitCliError::CommandFailed(_)) => Err(GitCliError::AuthFailed),
            Err(e) => Err(e),
        }
    }

    pub fn delete_branch(&self, repo_path: &Path, branch_name: &str) -> Result<(), GitCliError> {
        Self::ensure_available()?;
        self.git(repo_path, ["branch", "-D", branch_name])?;
        Ok(())
    }

    pub fn get_remote_url(&self, repo_path: &Path, remote_name: &str) -> Result<String, GitCliError> {
        let output = self.git(repo_path, ["remote", "get-url", remote_name])?;
        Ok(output.trim().to_string())
    }

    pub fn list_remotes(&self, repo_path: &Path) -> Result<Vec<(String, String)>, GitCliError> {
        let output = self.git(repo_path, ["remote", "-v"])?;
        let mut seen = std::collections::HashSet::new();
        let mut remotes = Vec::new();
        for line in output.lines() {
            let line = line.trim();
            if line.is_empty() { continue; }
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 2 {
                let name = parts[0].to_string();
                let url = parts[1].strip_suffix(" (fetch)").or_else(|| parts[1].strip_suffix(" (push)")).unwrap_or(parts[1]).to_string();
                if seen.insert(name.clone()) { remotes.push((name, url)); }
            }
        }
        Ok(remotes)
    }

    // --- Rebase ---
    pub fn merge_base(&self, worktree_path: &Path, a: &str, b: &str) -> Result<String, GitCliError> {
        let out = self.git(worktree_path, ["merge-base", "--fork-point", a, b]).unwrap_or(self.git(worktree_path, ["merge-base", a, b])?);
        Ok(out.trim().to_string())
    }

    pub fn rebase_onto(&self, worktree_path: &Path, new_base: &str, old_base: &str, task_branch: &str) -> Result<(), GitCliError> {
        if self.is_rebase_in_progress(worktree_path).unwrap_or(false) { return Err(GitCliError::RebaseInProgress); }
        let merge_base = self.merge_base(worktree_path, old_base, task_branch).unwrap_or(old_base.to_string());
        self.git(worktree_path, ["rebase", "--onto", new_base, &merge_base, task_branch])?;
        Ok(())
    }

    pub fn is_rebase_in_progress(&self, worktree_path: &Path) -> Result<bool, GitCliError> {
        let rebase_merge = self.git(worktree_path, ["rev-parse", "--git-path", "rebase-merge"])?;
        let rebase_apply = self.git(worktree_path, ["rev-parse", "--git-path", "rebase-apply"])?;
        Ok(std::path::Path::new(rebase_merge.trim()).exists() || std::path::Path::new(rebase_apply.trim()).exists())
    }

    pub fn is_merge_in_progress(&self, worktree_path: &Path) -> Result<bool, GitCliError> {
        match self.git(worktree_path, ["rev-parse", "--verify", "MERGE_HEAD"]) {
            Ok(_) => Ok(true),
            Err(GitCliError::CommandFailed(_)) => Ok(false),
            Err(e) => Err(e)
        }
    }

    pub fn is_cherry_pick_in_progress(&self, worktree_path: &Path) -> Result<bool, GitCliError> {
        match self.git(worktree_path, ["rev-parse", "--verify", "CHERRY_PICK_HEAD"]) {
            Ok(_) => Ok(true),
            Err(GitCliError::CommandFailed(_)) => Ok(false),
            Err(e) => Err(e)
        }
    }

    pub fn is_revert_in_progress(&self, worktree_path: &Path) -> Result<bool, GitCliError> {
        match self.git(worktree_path, ["rev-parse", "--verify", "REVERT_HEAD"]) {
            Ok(_) => Ok(true),
            Err(GitCliError::CommandFailed(_)) => Ok(false),
            Err(e) => Err(e)
        }
    }

    pub fn abort_rebase(&self, worktree_path: &Path) -> Result<(), GitCliError> {
        if !self.is_rebase_in_progress(worktree_path)? { return Ok(()); }
        self.git(worktree_path, ["rebase", "--abort"]).map(|_| ())
    }

    pub fn quit_rebase(&self, worktree_path: &Path) -> Result<(), GitCliError> {
        if !self.is_rebase_in_progress(worktree_path)? { return Ok(()); }
        self.git(worktree_path, ["rebase", "--quit"]).map(|_| ())
    }

    pub fn continue_rebase(&self, worktree_path: &Path) -> Result<(), GitCliError> {
        if !self.is_rebase_in_progress(worktree_path)? { return Err(GitCliError::CommandFailed("No rebase in progress".to_string())); }
        self.git(worktree_path, ["rebase", "--continue"]).map(|_| ())
    }

    // --- Other ---
    pub fn has_staged_changes(&self, repo_path: &Path) -> Result<bool, GitCliError> {
        let git = Self::git_path()?;
        let out = Command::new(&git).arg("-C").arg(repo_path).arg("diff").arg("--cached").arg("--quiet").output()
            .map_err(|e| GitCliError::CommandFailed(e.to_string()))?;
        match out.status.code() { Some(0) => Ok(false), Some(1) => Ok(true), _ => Err(GitCliError::CommandFailed("Unknown error".to_string())) }
    }

    pub fn merge_squash_commit(&self, repo_path: &Path, base_branch: &str, from_branch: &str, message: &str) -> Result<String, GitCliError> {
        self.git(repo_path, ["checkout", base_branch]).map(|_| ())?;
        self.git(repo_path, ["merge", "--squash", "--no-commit", from_branch]).map(|_| ())?;
        self.git(repo_path, ["commit", "-m", message]).map(|_| ())?;
        let sha = self.git(repo_path, ["rev-parse", "HEAD"])?.trim().to_string();
        Ok(sha)
    }

    pub fn update_ref(&self, repo_path: &Path, refname: &str, sha: &str) -> Result<(), GitCliError> {
        self.git(repo_path, ["update-ref", refname, sha]).map(|_| ())
    }

    pub fn abort_merge(&self, worktree_path: &Path) -> Result<(), GitCliError> {
        if !self.is_merge_in_progress(worktree_path)? { return Ok(()); }
        self.git(worktree_path, ["merge", "--abort"]).map(|_| ())
    }

    pub fn abort_cherry_pick(&self, worktree_path: &Path) -> Result<(), GitCliError> {
        if !self.is_cherry_pick_in_progress(worktree_path)? { return Ok(()); }
        self.git(worktree_path, ["cherry-pick", "--abort"]).map(|_| ())
    }

    pub fn abort_revert(&self, worktree_path: &Path) -> Result<(), GitCliError> {
        if !self.is_revert_in_progress(worktree_path)? { return Ok(()); }
        self.git(worktree_path, ["revert", "--abort"]).map(|_| ())
    }

    pub fn get_conflicted_files(&self, worktree_path: &Path) -> Result<Vec<String>, GitCliError> {
        let out = self.git(worktree_path, ["diff", "--name-only", "--diff-filter=U"])?;
        Ok(out.lines().filter(|l| !l.trim().is_empty()).map(|s| s.trim().to_string()).collect())
    }
}

// ============================================================================
// GitService - 高层封装
// ============================================================================

#[derive(Clone, Default)]
pub struct GitService;

impl GitService {
    pub fn new() -> Self { Self }

    pub fn open_repo(&self, repo_path: &Path) -> Result<Repository, GitServiceError> {
        Repository::open(repo_path).map_err(GitServiceError::from)
    }

    fn ensure_cli_commit_identity(&self, repo_path: &Path) -> Result<(), GitServiceError> {
        let repo = self.open_repo(repo_path)?;
        let cfg = repo.config()?;
        let has_name = cfg.get_string("user.name").is_ok();
        let has_email = cfg.get_string("user.email").is_ok();
        if !(has_name && has_email) {
            let mut cfg = repo.config()?;
            cfg.set_str("user.name", "Vibe Git")?;
            cfg.set_str("user.email", "noreply@vibegit.com")?;
        }
        Ok(())
    }

    fn signature_with_fallback<'a>(&self, repo: &'a Repository) -> Result<git2::Signature<'a>, GitServiceError> {
        match repo.signature() {
            Ok(sig) => Ok(sig),
            Err(_) => git2::Signature::now("Vibe Git", "noreply@vibegit.com").map_err(GitServiceError::from)
        }
    }

    fn default_remote(&self, repo: &Repository, repo_path: &Path) -> Result<GitRemote, GitServiceError> {
        let mut remotes = GitCli::new().list_remotes(repo_path)?;
        if let Ok(config) = repo.config() {
            if let Ok(default_name) = config.get_string("remote.pushDefault") {
                if let Some(idx) = remotes.iter().position(|(name, _)| name == &default_name) {
                    let (name, url) = remotes.swap_remove(idx);
                    return Ok(GitRemote { name, url });
                }
            }
        }
        remotes.into_iter().next()
            .map(|(name, url)| GitRemote { name, url })
            .ok_or_else(|| GitServiceError::InvalidRepository("No remotes configured".to_string()))
    }

    // --- Init ---
    pub fn initialize_repo_with_main_branch(&self, repo_path: &Path) -> Result<(), GitServiceError> {
        if !repo_path.exists() { std::fs::create_dir_all(repo_path)?; }
        let repo = Repository::init_opts(repo_path, git2::RepositoryInitOptions::new().initial_head("main").mkdir(true))?;
        self.create_initial_commit(&repo)?;
        Ok(())
    }

    pub fn ensure_main_branch_exists(&self, repo_path: &Path) -> Result<(), GitServiceError> {
        let repo = self.open_repo(repo_path)?;
        match repo.branches(None) {
            Ok(branches) => {
                let count = branches.count();
                if count == 0 { self.create_initial_commit(&repo)?; }
            }
            _ => {}
        }
        Ok(())
    }

    pub fn create_initial_commit(&self, repo: &Repository) -> Result<(), GitServiceError> {
        let signature = self.signature_with_fallback(repo)?;
        let tree_id = repo.treebuilder(None)?.write()?;
        let tree = repo.find_tree(tree_id)?;
        let _commit_id = repo.commit(Some("refs/heads/main"), &signature, &signature, "Initial commit", &tree, &[])?;
        repo.set_head("refs/heads/main")?;
        Ok(())
    }

    // --- Diff ---
    pub fn get_diffs(&self, target: DiffTarget, path_filter: Option<&[&str]>) -> Result<Vec<Diff>, GitServiceError> {
        match target {
            DiffTarget::Worktree { worktree_path, base_commit } => {
                // For worktree diffs, compare base commit tree with HEAD
                let repo = Repository::open(worktree_path)?;
                let base_tree = repo.find_commit(base_commit.as_oid())?.tree()
                    .map_err(|e| GitServiceError::InvalidRepository(format!("Failed to find base commit tree: {}", e)))?;
                let head_tree = repo.head()?.peel_to_commit()?.tree()?;
                let mut diff_opts = git2::DiffOptions::new();
                diff_opts.include_typechange(true);
                if let Some(paths) = path_filter { for path in paths { diff_opts.pathspec(path); } }
                let mut diff = repo.diff_tree_to_tree(Some(&base_tree), Some(&head_tree), Some(&mut diff_opts))?;
                let mut find_opts = git2::DiffFindOptions::new();
                diff.find_similar(Some(&mut find_opts))?;
                self.convert_diff_to_file_diffs(diff, &repo)
            }
            DiffTarget::Branch { repo_path, branch_name, base_branch } => {
                let repo = self.open_repo(repo_path)?;
                let base_tree = Self::find_branch(&repo, base_branch)?.get().peel_to_commit()?.tree()?;
                let branch_tree = Self::find_branch(&repo, branch_name)?.get().peel_to_commit()?.tree()?;
                let mut diff_opts = git2::DiffOptions::new();
                diff_opts.include_typechange(true);
                if let Some(paths) = path_filter { for path in paths { diff_opts.pathspec(path); } }
                let mut diff = repo.diff_tree_to_tree(Some(&base_tree), Some(&branch_tree), Some(&mut diff_opts))?;
                let mut find_opts = git2::DiffFindOptions::new();
                diff.find_similar(Some(&mut find_opts))?;
                self.convert_diff_to_file_diffs(diff, &repo)
            }
            DiffTarget::Commit { repo_path, commit_sha } => {
                let repo = self.open_repo(repo_path)?;
                let commit_oid = git2::Oid::from_str(commit_sha)
                    .map_err(|_| GitServiceError::InvalidRepository(format!("Invalid commit SHA: {}", commit_sha)))?;
                let commit = repo.find_commit(commit_oid)?;
                let parent = commit.parent(0)
                    .map_err(|_| GitServiceError::InvalidRepository("Commit has no parent".to_string()))?;
                let parent_tree = parent.tree()?;
                let commit_tree = commit.tree()?;
                let mut diff_opts = git2::DiffOptions::new();
                diff_opts.include_typechange(true);
                if let Some(paths) = path_filter { for path in paths { diff_opts.pathspec(path); } }
                let mut diff = repo.diff_tree_to_tree(Some(&parent_tree), Some(&commit_tree), Some(&mut diff_opts))?;
                let mut find_opts = git2::DiffFindOptions::new();
                diff.find_similar(Some(&mut find_opts))?;
                self.convert_diff_to_file_diffs(diff, &repo)
            }
        }
    }

    fn convert_diff_to_file_diffs(&self, diff: git2::Diff, repo: &Repository) -> Result<Vec<Diff>, GitServiceError> {
        use git2::Delta;
        let mut file_diffs = Vec::new();
        let mut delta_index: usize = 0;
        diff.foreach(&mut |delta, _| {
            if delta.status() == Delta::Unreadable { return true; }
            let status = delta.status();
            let mut content_omitted = false;
            if !matches!(status, Delta::Added) {
                let oid = delta.old_file().id();
                if !oid.is_zero() && let Ok(blob) = repo.find_blob(oid) && !blob.is_binary() && blob.size() > MAX_INLINE_DIFF_BYTES { content_omitted = true; }
            }
            if !matches!(status, Delta::Deleted) {
                let oid = delta.new_file().id();
                if !oid.is_zero() && let Ok(blob) = repo.find_blob(oid) && !blob.is_binary() && blob.size() > MAX_INLINE_DIFF_BYTES { content_omitted = true; }
            }
            let old_path = if matches!(status, Delta::Added) { None } else { delta.old_file().path().map(|p| p.to_string_lossy().to_string()) };
            let new_path = if matches!(status, Delta::Deleted) { None } else { delta.new_file().path().map(|p| p.to_string_lossy().to_string()) };
            let old_content = if content_omitted || matches!(status, Delta::Added) { None } else { self.read_blob_content(repo, &delta.old_file().id()).ok() };
            let new_content = if content_omitted || matches!(status, Delta::Deleted) { None } else { self.read_blob_content(repo, &delta.new_file().id()).ok() };
            let change = match status {
                Delta::Added => DiffChangeKind::Added, Delta::Deleted => DiffChangeKind::Deleted,
                Delta::Modified => DiffChangeKind::Modified, Delta::Renamed => DiffChangeKind::Renamed,
                Delta::Copied => DiffChangeKind::Copied, Delta::Untracked => DiffChangeKind::Added, _ => DiffChangeKind::Modified,
            };
            let (additions, deletions) = if let Ok(Some(patch)) = git2::Patch::from_diff(&diff, delta_index) && let Ok((_, adds, dels)) = patch.line_stats() { (Some(adds), Some(dels)) } else { (None, None) };
            file_diffs.push(Diff { change, old_path, new_path, old_content, new_content, content_omitted, additions, deletions, repo_id: None });
            delta_index += 1;
            true
        }, None, None, None)?;
        Ok(file_diffs)
    }

    fn read_blob_content(&self, repo: &Repository, oid: &git2::Oid) -> Result<String, GitServiceError> {
        if oid.is_zero() { return Err(GitServiceError::InvalidRepository("Zero OID".to_string())); }
        let blob = repo.find_blob(*oid)?;
        if blob.is_binary() { return Err(GitServiceError::InvalidRepository("Binary blob".to_string())); }
        std::str::from_utf8(blob.content()).map_err(|e| GitServiceError::InvalidRepository(format!("Invalid UTF-8: {}", e))).map(|s| s.to_string())
    }

    fn status_entry_to_diff(&self, repo: &Repository, base_tree: &git2::Tree, e: StatusDiffEntry) -> Diff {
        let change = match e.change {
            ChangeType::Added => DiffChangeKind::Added, ChangeType::Deleted => DiffChangeKind::Deleted,
            ChangeType::Modified => DiffChangeKind::Modified, ChangeType::Renamed => DiffChangeKind::Renamed,
            ChangeType::Copied => DiffChangeKind::Copied, _ => DiffChangeKind::Modified,
        };
        let (old_path, new_path) = match e.change {
            ChangeType::Added => (None, Some(e.path.clone())),
            ChangeType::Deleted => (Some(e.old_path.unwrap_or(e.path.clone())), None),
            ChangeType::Modified | ChangeType::TypeChanged | ChangeType::Unmerged => (Some(e.old_path.clone().unwrap_or(e.path.clone())), Some(e.path.clone())),
            ChangeType::Renamed | ChangeType::Copied => (e.old_path.clone(), Some(e.path.clone())),
            ChangeType::Unknown(_) => (e.old_path.clone(), Some(e.path.clone())),
        };
        Diff { change, old_path, new_path, old_content: None, new_content: None, content_omitted: false, additions: None, deletions: None, repo_id: None }
    }

    // --- Commit ---
    pub fn commit(&self, path: &Path, message: &str) -> Result<bool, GitServiceError> {
        let git = GitCli::new();
        let has_changes = git.has_changes(path).map_err(|e| GitServiceError::InvalidRepository(format!("git status failed: {}", e)))?;
        if !has_changes { return Ok(false); }
        git.add_all(path).map_err(|e| GitServiceError::InvalidRepository(format!("git add failed: {}", e)))?;
        self.ensure_cli_commit_identity(path)?;
        git.commit(path, message).map_err(|e| GitServiceError::InvalidRepository(format!("git commit failed: {}", e)))?;
        Ok(true)
    }

    // --- Branches ---
    pub fn get_all_branches(&self, repo_path: &Path) -> Result<Vec<GitBranch>, git2::Error> {
        let repo = Repository::open(repo_path)?;
        let current_branch = self.get_current_branch(repo_path).unwrap_or_default();
        let mut branches = Vec::new();

        let get_date = |branch: &git2::Branch| -> Result<DateTime<Utc>, git2::Error> {
            Ok(branch.get().target()
                .and_then(|t| repo.find_commit(t).ok())
                .map(|c| DateTime::from_timestamp(c.time().seconds(), 0))
                .flatten()
                .unwrap_or_else(Utc::now))
        };

        for branch_result in repo.branches(Some(BranchType::Local))? {
            let (branch, _) = branch_result?;
            if let Some(name) = branch.name()?.map(|s| s.to_string()) {
                branches.push(GitBranch {
                    name: name.clone(),
                    is_current: name == current_branch,
                    is_remote: false,
                    last_commit_date: get_date(&branch)?
                });
            }
        }
        for branch_result in repo.branches(Some(BranchType::Remote))? {
            let (branch, _) = branch_result?;
            if let Some(name) = branch.name()?.filter(|n| !n.ends_with("/HEAD")).map(|s| s.to_string()) {
                branches.push(GitBranch {
                    name: name.clone(),
                    is_current: false,
                    is_remote: true,
                    last_commit_date: get_date(&branch)?
                });
            }
        }

        branches.sort_by(|a, b| {
            if a.is_current != b.is_current {
                if a.is_current { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater }
            } else {
                b.last_commit_date.cmp(&a.last_commit_date)
            }
        });
        Ok(branches)
    }

    pub fn is_branch_name_valid(&self, name: &str) -> bool {
        git2::Branch::name_is_valid(name).unwrap_or(false)
    }

    pub fn rename_local_branch(&self, worktree_path: &Path, old_branch_name: &str, new_branch_name: &str) -> Result<(), GitServiceError> {
        let repo = self.open_repo(worktree_path)?;
        let mut branch = repo.find_branch(old_branch_name, BranchType::Local)
            .map_err(|_| GitServiceError::BranchNotFound(old_branch_name.to_string()))?;
        branch.rename(new_branch_name, false)?;
        repo.set_head(&format!("refs/heads/{new_branch_name}"))?;
        Ok(())
    }

    pub fn get_current_branch(&self, repo_path: &Path) -> Result<String, git2::Error> {
        Ok(self.get_head_info(repo_path).ok().map(|h| h.branch).unwrap_or_default())
    }

    pub fn get_head_info(&self, repo_path: &Path) -> Result<HeadInfo, GitServiceError> {
        let repo = self.open_repo(repo_path)?;
        let head = repo.head()?;
        let branch = head.shorthand().map(|s| s.to_string()).unwrap_or_else(|| "HEAD".to_string());
        let oid = head.target().map(|t| t.to_string()).ok_or_else(|| GitServiceError::InvalidRepository("Repository HEAD has no target".to_string()))?;
        Ok(HeadInfo { branch, oid })
    }

    pub fn check_branch_exists(&self, repo_path: &Path, branch_name: &str) -> Result<bool, GitServiceError> {
        let repo = self.open_repo(repo_path)?;
        Ok(repo.find_branch(branch_name, BranchType::Local).is_ok() || repo.find_branch(branch_name, BranchType::Remote).is_ok())
    }

    pub fn find_branch_type(&self, repo_path: &Path, branch_name: &str) -> Result<BranchType, GitServiceError> {
        let repo = self.open_repo(repo_path)?;
        Ok(if repo.find_branch(branch_name, BranchType::Local).is_ok() {
            BranchType::Local
        } else if repo.find_branch(branch_name, BranchType::Remote).is_ok() {
            BranchType::Remote
        } else {
            return Err(GitServiceError::BranchNotFound(branch_name.to_string()));
        })
    }

    pub fn find_branch<'a>(repo: &'a Repository, branch_name: &str) -> Result<git2::Branch<'a>, GitServiceError> {
        Ok(repo.find_branch(branch_name, BranchType::Local).or_else(|_| repo.find_branch(branch_name, BranchType::Remote))?)
    }

    pub fn delete_branch(&self, repo_path: &Path, branch_name: &str) -> Result<(), GitServiceError> {
        GitCli::new().delete_branch(repo_path, branch_name).map_err(|e| GitServiceError::InvalidRepository(e.to_string()))
    }

    pub fn get_branch_status(&self, repo_path: &Path, branch_name: &str, base_branch_name: &str) -> Result<(usize, usize), GitServiceError> {
        let repo = Repository::open(repo_path)?;
        let branch = Self::find_branch(&repo, branch_name)?.into_reference();
        let base_branch = Self::find_branch(&repo, base_branch_name)?.into_reference();
        let (a, b) = repo.graph_ahead_behind(
            branch.target().ok_or(GitServiceError::BranchNotFound("Branch not found".to_string()))?,
            base_branch.target().ok_or(GitServiceError::BranchNotFound("Base branch not found".to_string()))?
        )?;
        Ok((a, b))
    }

    pub fn get_base_commit(&self, repo_path: &Path, branch_name: &str, base_branch_name: &str) -> Result<Commit, GitServiceError> {
        let repo = self.open_repo(repo_path)?;
        let oid = repo.merge_base(
            Self::find_branch(&repo, branch_name)?.get().peel_to_commit()?.id(),
            Self::find_branch(&repo, base_branch_name)?.get().peel_to_commit()?.id()
        ).map_err(GitServiceError::from)?;
        Ok(Commit::new(oid))
    }

    pub fn get_fork_point(&self, worktree_path: &Path, target_branch: &str, task_branch: &str) -> Result<String, GitServiceError> {
        Ok(GitCli::new().merge_base(worktree_path, target_branch, task_branch)?)
    }

    // --- Worktree ---
    pub fn add_worktree(&self, repo_path: &Path, worktree_path: &Path, branch: &str, create_branch: bool) -> Result<(), GitServiceError> {
        GitCli::new().worktree_add(repo_path, worktree_path, branch, create_branch).map_err(|e| GitServiceError::InvalidRepository(e.to_string()))
    }

    pub fn remove_worktree(&self, repo_path: &Path, worktree_path: &Path, force: bool) -> Result<(), GitServiceError> {
        GitCli::new().worktree_remove(repo_path, worktree_path, force).map_err(|e| GitServiceError::InvalidRepository(e.to_string()))
    }

    pub fn prune_worktrees(&self, repo_path: &Path) -> Result<(), GitServiceError> {
        GitCli::new().worktree_prune(repo_path).map_err(|e| GitServiceError::InvalidRepository(e.to_string()))
    }

    pub fn move_worktree(&self, repo_path: &Path, old_path: &Path, new_path: &Path) -> Result<(), GitServiceError> {
        let git = GitCli::new();
        git.worktree_move(repo_path, old_path, new_path).map_err(|e| GitServiceError::InvalidRepository(e.to_string()))
    }

    pub fn reset_worktree_to_commit(&self, worktree_path: &Path, commit_sha: &str, force: bool) -> Result<(), GitServiceError> {
        let repo = self.open_repo(worktree_path)?;
        if !force { self.check_worktree_clean(&repo)?; }
        GitCli::new().git(worktree_path, ["reset", "--hard", commit_sha])
            .map_err(|e| GitServiceError::InvalidRepository(format!("git reset --hard failed: {}", e)))?;
        Ok(())
    }

    pub fn reconcile_worktree_to_commit(&self, worktree_path: &Path, target_commit_oid: &str, options: WorktreeResetOptions) -> WorktreeResetOutcome {
        let WorktreeResetOptions { perform_reset, force_when_dirty, is_dirty, log_skip_when_dirty } = options;
        let head_oid = self.get_head_info(worktree_path).ok().map(|h| h.oid);
        let mut outcome = WorktreeResetOutcome::default();

        if head_oid.as_deref() != Some(target_commit_oid) || is_dirty {
            outcome.needed = true;
            if perform_reset {
                if is_dirty && !force_when_dirty {
                    if log_skip_when_dirty { tracing::warn!("Worktree dirty; skipping reset as not forced"); }
                } else if let Err(e) = self.reset_worktree_to_commit(worktree_path, target_commit_oid, force_when_dirty) {
                    tracing::error!("Failed to reset worktree: {}", e);
                } else {
                    outcome.applied = true;
                }
            }
        }
        outcome
    }

    fn check_worktree_clean(&self, repo: &Repository) -> Result<(), GitServiceError> {
        let mut opts = git2::StatusOptions::new();
        opts.include_untracked(false).include_ignored(false);
        let statuses = repo.statuses(Some(&mut opts))?;
        if !statuses.is_empty() {
            let dirty: Vec<_> = statuses.iter().filter_map(|e| e.path().map(|s| s.to_string())).collect();
            if !dirty.is_empty() {
                return Err(GitServiceError::WorktreeDirty(
                    repo.head().ok().and_then(|h| h.shorthand().map(|s| s.to_string())).unwrap_or_default(),
                    dirty.join(", ")
                ));
            }
        }
        Ok(())
    }

    // --- Status ---
    pub fn is_worktree_clean(&self, worktree_path: &Path) -> Result<bool, GitServiceError> {
        Ok(self.open_repo(worktree_path).and_then(|r| self.check_worktree_clean(&r).map_err(GitServiceError::from)).is_ok())
    }

    pub fn get_worktree_status(&self, worktree_path: &Path) -> Result<WorktreeStatus, GitServiceError> {
        GitCli::new().get_worktree_status(worktree_path)
            .map_err(|e| GitServiceError::InvalidRepository(format!("git status failed: {}", e)))
    }

    pub fn get_worktree_change_counts(&self, worktree_path: &Path) -> Result<(usize, usize), GitServiceError> {
        let st = self.get_worktree_status(worktree_path)?;
        Ok((st.uncommitted_tracked, st.untracked))
    }

    // --- Remote ---
    pub fn get_default_remote(&self, repo_path: &Path) -> Result<GitRemote, GitServiceError> {
        self.default_remote(&self.open_repo(repo_path)?, repo_path)
    }

    pub fn list_remotes(&self, repo_path: &Path) -> Result<Vec<GitRemote>, GitServiceError> {
        Ok(GitCli::new().list_remotes(repo_path)?.into_iter().map(|(n, u)| GitRemote { name: n, url: u }).collect())
    }

    pub fn get_remote_url(&self, repo_path: &Path, remote_name: &str) -> Result<String, GitServiceError> {
        GitCli::new().get_remote_url(repo_path, remote_name).map_err(GitServiceError::from)
    }

    pub fn check_remote_branch_exists(&self, repo_path: &Path, remote_url: &str, branch_name: &str) -> Result<bool, GitServiceError> {
        GitCli::new().check_remote_branch_exists(repo_path, remote_url, branch_name).map_err(GitServiceError::from)
    }

    pub fn get_remote_branch_status(&self, repo_path: &Path, branch_name: &str, base_branch_name: Option<&str>) -> Result<(usize, usize), GitServiceError> {
        let repo = Repository::open(repo_path)?;
        let branch_ref = Self::find_branch(&repo, branch_name)?.into_reference();
        let base_branch_ref = if let Some(bn) = base_branch_name {
            Self::find_branch(&repo, bn)?.into_reference()
        } else {
            repo.find_branch(branch_name, BranchType::Local)?.upstream()?.into_reference()
        };
        // Get remote info
        let branch_name_str = base_branch_ref.name().ok_or_else(|| GitServiceError::InvalidRepository("Invalid branch ref".to_string()))?;
        let remote_name_buf = repo.branch_remote_name(branch_name_str)?;
        let remote_name = std::str::from_utf8(&remote_name_buf).map_err(|e| GitServiceError::InvalidRepository(format!("Invalid remote name: {}", e)))?.to_string();
        let remote = repo.find_remote(&remote_name).map_err(|_| GitServiceError::InvalidRepository(format!("Remote not found: {}", remote_name)))?;
        // Fetch
        let refspec = format!("+refs/heads/*:refs/remotes/{remote_name}/*");
        let remote_url = remote.url().ok_or_else(|| GitServiceError::InvalidRepository("Remote has no URL".to_string()))?;
        GitCli::new().fetch_with_refspec(repo_path, remote_url, &refspec).map_err(GitServiceError::from)?;
        let (ahead, behind) = repo.graph_ahead_behind(
            branch_ref.target().ok_or(GitServiceError::BranchNotFound("Branch not found".to_string()))?,
            base_branch_ref.target().ok_or(GitServiceError::BranchNotFound("Base branch not found".to_string()))?
        )?;
        Ok((ahead, behind))
    }

    pub fn resolve_remote_for_branch(&self, repo_path: &Path, branch_name: &str) -> Result<GitRemote, GitServiceError> {
        self.get_remote_from_branch_name(repo_path, branch_name)
            .or_else(|_| self.get_default_remote(repo_path))
    }

    fn get_remote_from_branch_name(&self, repo_path: &Path, branch_name: &str) -> Result<GitRemote, GitServiceError> {
        let repo = Repository::open(repo_path)?;
        let branch_ref = Self::find_branch(&repo, branch_name)?.into_reference();
        let remote = self.get_remote_from_branch_ref(&repo, &branch_ref)?;
        let name = remote.name().map(|s| s.to_string()).ok_or_else(|| GitServiceError::InvalidRepository("Remote has no name".to_string()))?;
        let url = remote.url().map(|s| s.to_string()).ok_or_else(|| GitServiceError::InvalidRepository("Remote has no URL".to_string()))?;
        Ok(GitRemote { name, url })
    }

    fn get_remote_from_branch_ref<'a>(&self, repo: &'a Repository, branch_ref: &git2::Reference) -> Result<git2::Remote<'a>, GitServiceError> {
        let branch_name = branch_ref.name().map(|s| s.to_string()).ok_or_else(|| GitServiceError::InvalidRepository("Invalid branch ref".to_string()))?;
        let remote_name_buf = repo.branch_remote_name(&branch_name)?;
        let remote_name = std::str::from_utf8(&remote_name_buf).map_err(|e| GitServiceError::InvalidRepository(format!("Invalid remote name: {}", e)))?.to_string();
        repo.find_remote(&remote_name).map_err(|_| GitServiceError::InvalidRepository(format!("Remote not found: {}", remote_name)))
    }

    fn fetch_all_from_remote(&self, repo: &Repository, remote: &git2::Remote) -> Result<(), GitServiceError> {
        let default_remote = self.default_remote(repo, repo.path())?;
        let remote_name = remote.name().unwrap_or(&default_remote.name);
        let refspec = format!("+refs/heads/*:refs/remotes/{remote_name}/*");
        GitCli::new().fetch_with_refspec(repo.path(), remote.url().ok_or(GitServiceError::InvalidRepository("Remote has no URL".to_string()))?, &refspec)
            .map_err(GitServiceError::from)
    }

    pub fn fetch_branch(&self, repo_path: &Path, remote_url: &str, branch_name: &str) -> Result<(), GitServiceError> {
        let refspec = format!("+refs/heads/{0}:refs/heads/{0}", branch_name);
        GitCli::new().fetch_with_refspec(repo_path, remote_url, &refspec).map_err(GitServiceError::from)
    }

    pub fn push_to_remote(&self, worktree_path: &Path, branch_name: &str, force: bool) -> Result<(), GitServiceError> {
        let repo = self.open_repo(worktree_path)?;
        self.check_worktree_clean(&repo)?;
        let remote = self.default_remote(&repo, worktree_path)?;
        if let Err(e) = GitCli::new().push(worktree_path, &remote.url, branch_name, force) {
            return Err(e.into());
        }
        let mut branch = Self::find_branch(&repo, branch_name)?;
        if !branch.get().is_remote() {
            if let Some(target) = branch.get().target() {
                let refname = format!("refs/remotes/{0}/{1}", remote.name, branch_name);
                repo.reference(&refname, target, true, "update remote-tracking")?;
            }
            branch.set_upstream(Some(&format!("{0}/{1}", remote.name, branch_name)))?;
        }
        Ok(())
    }

    // --- Conflict ---
    pub fn detect_conflict_op(&self, worktree_path: &Path) -> Result<Option<ConflictOp>, GitServiceError> {
        let git = GitCli::new();
        if git.is_rebase_in_progress(worktree_path).unwrap_or(false) { return Ok(Some(ConflictOp::Rebase)); }
        if git.is_merge_in_progress(worktree_path).unwrap_or(false) { return Ok(Some(ConflictOp::Merge)); }
        if git.is_cherry_pick_in_progress(worktree_path).unwrap_or(false) { return Ok(Some(ConflictOp::CherryPick)); }
        if git.is_revert_in_progress(worktree_path).unwrap_or(false) { return Ok(Some(ConflictOp::Revert)); }
        Ok(None)
    }

    pub fn get_conflicted_files(&self, worktree_path: &Path) -> Result<Vec<String>, GitServiceError> {
        GitCli::new().get_conflicted_files(worktree_path)
            .map_err(|e| GitServiceError::InvalidRepository(format!("git diff failed: {}", e)))
    }

    pub fn is_rebase_in_progress(&self, worktree_path: &Path) -> Result<bool, GitServiceError> {
        GitCli::new().is_rebase_in_progress(worktree_path)
            .map_err(|e| GitServiceError::InvalidRepository(format!("git rebase state check failed: {}", e)))
    }

    pub fn abort_rebase(&self, worktree_path: &Path) -> Result<(), GitServiceError> {
        GitCli::new().abort_rebase(worktree_path)
            .map_err(|e| GitServiceError::InvalidRepository(format!("git rebase --abort failed: {}", e)))
    }

    pub fn continue_rebase(&self, worktree_path: &Path) -> Result<(), GitServiceError> {
        GitCli::new().continue_rebase(worktree_path)
            .map_err(|e| GitServiceError::InvalidRepository(format!("git rebase --continue failed: {}", e)))
    }

    pub fn abort_conflicts(&self, worktree_path: &Path) -> Result<(), GitServiceError> {
        let git = GitCli::new();
        if git.is_rebase_in_progress(worktree_path).unwrap_or(false) {
            return if !self.get_conflicted_files(worktree_path)?.is_empty() {
                self.abort_rebase(worktree_path)
            } else {
                git.quit_rebase(worktree_path)
                    .map_err(|e| GitServiceError::InvalidRepository(format!("git rebase --quit failed: {}", e)))
            };
        }
        if git.is_merge_in_progress(worktree_path).unwrap_or(false) {
            return git.abort_merge(worktree_path)
                .map_err(|e| GitServiceError::InvalidRepository(format!("git merge --abort failed: {}", e)));
        }
        if git.is_cherry_pick_in_progress(worktree_path).unwrap_or(false) {
            return git.abort_cherry_pick(worktree_path)
                .map_err(|e| GitServiceError::InvalidRepository(format!("git cherry-pick --abort failed: {}", e)));
        }
        if git.is_revert_in_progress(worktree_path).unwrap_or(false) {
            return git.abort_revert(worktree_path)
                .map_err(|e| GitServiceError::InvalidRepository(format!("git revert --abort failed: {}", e)));
        }
        Ok(())
    }

    // --- Other ---
    pub fn get_commit_subject(&self, repo_path: &Path, commit_sha: &str) -> Result<String, GitServiceError> {
        let repo = self.open_repo(repo_path)?;
        Ok(repo.find_commit(git2::Oid::from_str(commit_sha)?)?.summary().unwrap_or("(no subject)").to_string())
    }

    pub fn ahead_behind_commits_by_oid(&self, repo_path: &Path, from_oid: &str, to_oid: &str) -> Result<(usize, usize), GitServiceError> {
        let repo = self.open_repo(repo_path)?;
        let (a, b) = repo.graph_ahead_behind(git2::Oid::from_str(from_oid)?, git2::Oid::from_str(to_oid)?)?;
        Ok((a, b))
    }

    pub fn collect_recent_file_stats(&self, repo_path: &Path, commit_limit: usize) -> Result<HashMap<String, FileStat>, GitServiceError> {
        let repo = self.open_repo(repo_path)?;
        let mut stats: HashMap<String, FileStat> = HashMap::new();
        let mut revwalk = repo.revwalk()?;
        revwalk.push_head()?;
        revwalk.set_sorting(Sort::TIME)?;
        for (commit_index, oid_result) in revwalk.take(commit_limit).enumerate() {
            let oid = oid_result?;
            let commit = repo.find_commit(oid)?;
            let commit_time = DateTime::from_timestamp(commit.time().seconds(), 0).unwrap_or_else(Utc::now);
            let commit_tree = commit.tree()?;
            let parent_tree = if commit.parent_count() == 0 { None } else { Some(commit.parent(0)?.tree()?) };
            let diff = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&commit_tree), None)?;
            diff.foreach(&mut |delta, _| {
                if let Some(path) = delta.new_file().path().or_else(|| delta.old_file().path()) {
                    let path_str = path.to_string_lossy().to_string();
                    let stat = stats.entry(path_str).or_insert(FileStat { last_index: commit_index, commit_count: 0, last_time: commit_time });
                    stat.commit_count += 1;
                    if commit_index < stat.last_index { stat.last_index = commit_index; stat.last_time = commit_time; }
                }
                true
            }, None, None, None)?;
        }
        Ok(stats)
    }

    pub fn get_branch_oid(&self, repo_path: &Path, branch_name: &str) -> Result<String, GitServiceError> {
        Ok(Self::find_branch(&self.open_repo(repo_path)?, branch_name)?.get().peel_to_commit()?.id().to_string())
    }

    // --- Merge/Rebase ---
    pub fn merge_changes(&self, base_worktree_path: &Path, task_worktree_path: &Path, task_branch_name: &str, base_branch_name: &str, commit_message: &str) -> Result<String, GitServiceError> {
        let (_, task_behind) = self.get_branch_status(base_worktree_path, task_branch_name, base_branch_name)?;
        if task_behind > 0 {
            let msg = format!("Cannot merge: base is {} commits ahead of task", task_behind);
            return Err(GitServiceError::BranchesDiverged(msg));
        }

        let git_cli = GitCli::new();
        let worktrees = git_cli.list_worktrees(base_worktree_path)?;
        for w in worktrees {
            if w.branch.as_deref() == Some(base_branch_name) {
                let path = std::path::PathBuf::from(w.path);
                if git_cli.has_staged_changes(&path)? {
                    return Err(GitServiceError::WorktreeDirty(base_branch_name.to_string(), "has staged changes".to_string()));
                }
                self.ensure_cli_commit_identity(&path)?;
                let sha = git_cli.merge_squash_commit(&path, base_branch_name, task_branch_name, commit_message)
                    .map_err(|e| GitServiceError::InvalidRepository(format!("CLI merge failed: {}", e)))?;
                let refname = format!("refs/heads/{0}", task_branch_name);
                git_cli.update_ref(base_worktree_path, &refname, &sha)
                    .map_err(|e| GitServiceError::InvalidRepository(format!("update-ref failed: {}", e)))?;
                return Ok(sha);
            }
        }
        Err(GitServiceError::InvalidRepository("Base branch not checked out".to_string()))
    }

    pub fn rebase_branch(&self, _repo_path: &Path, worktree_path: &Path, new_base_branch: &str, old_base_branch: &str, task_branch: &str) -> Result<String, GitServiceError> {
        let worktree_repo = self.open_repo(worktree_path)?;
        self.check_worktree_clean(&worktree_repo)?;
        let git = GitCli::new();
        if git.is_rebase_in_progress(worktree_path).unwrap_or(false) { return Err(GitServiceError::RebaseInProgress); }
        self.ensure_cli_commit_identity(worktree_path)?;
        match git.rebase_onto(worktree_path, new_base_branch, old_base_branch, task_branch) {
            Ok(()) => Ok(worktree_repo.head()?.peel_to_commit()?.id().to_string()),
            Err(GitCliError::RebaseInProgress) => Err(GitServiceError::RebaseInProgress),
            Err(GitCliError::CommandFailed(stderr)) if stderr.contains("CONFLICT") => {
                Err(GitServiceError::MergeConflicts("Rebase conflict".to_string()))
            }
            Err(e) => Err(GitServiceError::InvalidRepository(format!("git rebase failed: {}", e))),
        }
    }
}

// ============================================================================
// 验证
// ============================================================================

pub fn is_valid_branch_prefix(prefix: &str) -> bool {
    if prefix.is_empty() { return true; }
    if prefix.contains('/') { return false; }
    git2::Branch::name_is_valid(&format!("{}/x", prefix)).unwrap_or_default()
}

// ============================================================================
// Re-exports
// ============================================================================

// Re-exports
mod cli {}
