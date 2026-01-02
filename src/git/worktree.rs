//! Git worktree management

use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum WorktreeError {
    #[error("Git command failed: {0}")]
    CommandFailed(String),
    #[error("Not a git repository: {0}")]
    NotAGitRepo(PathBuf),
    #[error("Worktree already exists: {0}")]
    AlreadyExists(PathBuf),
    #[error("Worktree not found: {0}")]
    NotFound(PathBuf),
    #[error("Failed to parse git output: {0}")]
    ParseError(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Information about a git worktree
#[derive(Debug, Clone)]
pub struct WorktreeInfo {
    /// Path to the worktree
    pub path: PathBuf,
    /// Current HEAD commit
    pub head: String,
    /// Branch name (if on a branch)
    pub branch: Option<String>,
    /// Whether this is the main worktree
    pub is_main: bool,
}

/// Manager for git worktree operations
#[derive(Debug, Default)]
pub struct WorktreeManager {
    /// Base directory for managed worktrees
    managed_dir: Option<PathBuf>,
}

impl WorktreeManager {
    /// Create a new WorktreeManager
    pub fn new() -> Self {
        Self { managed_dir: None }
    }

    /// Create a WorktreeManager with a managed directory for worktrees
    pub fn with_managed_dir(dir: PathBuf) -> Self {
        Self {
            managed_dir: Some(dir),
        }
    }

    /// Create a new worktree for a branch
    ///
    /// # Arguments
    /// * `repo_path` - Path to the git repository
    /// * `branch` - Branch name to check out in the worktree
    /// * `name` - Name for the worktree directory
    ///
    /// # Returns
    /// Path to the created worktree
    pub fn create_worktree(
        &self,
        repo_path: &Path,
        branch: &str,
        name: &str,
    ) -> Result<PathBuf, WorktreeError> {
        self.validate_git_repo(repo_path)?;

        let worktree_path = self.worktree_path(repo_path, name);

        if worktree_path.exists() {
            return Err(WorktreeError::AlreadyExists(worktree_path));
        }

        // Ensure parent directory exists
        if let Some(parent) = worktree_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Try to add worktree for existing branch
        let output = Command::new("git")
            .args(["worktree", "add", worktree_path.to_str().unwrap(), branch])
            .current_dir(repo_path)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // If branch doesn't exist, create it
            if stderr.contains("not a valid reference") || stderr.contains("invalid reference") {
                let output = Command::new("git")
                    .args([
                        "worktree",
                        "add",
                        "-b",
                        branch,
                        worktree_path.to_str().unwrap(),
                    ])
                    .current_dir(repo_path)
                    .output()?;

                if !output.status.success() {
                    return Err(WorktreeError::CommandFailed(
                        String::from_utf8_lossy(&output.stderr).to_string(),
                    ));
                }
            } else {
                return Err(WorktreeError::CommandFailed(stderr.to_string()));
            }
        }

        Ok(worktree_path)
    }

    /// Remove a worktree
    pub fn remove_worktree(
        &self,
        repo_path: &Path,
        worktree_path: &Path,
    ) -> Result<(), WorktreeError> {
        self.validate_git_repo(repo_path)?;

        if !worktree_path.exists() {
            return Err(WorktreeError::NotFound(worktree_path.to_path_buf()));
        }

        let output = Command::new("git")
            .args(["worktree", "remove", worktree_path.to_str().unwrap()])
            .current_dir(repo_path)
            .output()?;

        if !output.status.success() {
            // Try force removal if there are changes
            let output = Command::new("git")
                .args([
                    "worktree",
                    "remove",
                    "--force",
                    worktree_path.to_str().unwrap(),
                ])
                .current_dir(repo_path)
                .output()?;

            if !output.status.success() {
                return Err(WorktreeError::CommandFailed(
                    String::from_utf8_lossy(&output.stderr).to_string(),
                ));
            }
        }

        Ok(())
    }

    /// List all worktrees for a repository
    pub fn list_worktrees(&self, repo_path: &Path) -> Result<Vec<WorktreeInfo>, WorktreeError> {
        self.validate_git_repo(repo_path)?;

        let output = Command::new("git")
            .args(["worktree", "list", "--porcelain"])
            .current_dir(repo_path)
            .output()?;

        if !output.status.success() {
            return Err(WorktreeError::CommandFailed(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        self.parse_worktree_list(&stdout)
    }

    /// Get the current branch name for a path
    pub fn get_current_branch(&self, path: &Path) -> Result<String, WorktreeError> {
        let output = Command::new("git")
            .args(["branch", "--show-current"])
            .current_dir(path)
            .output()?;

        if !output.status.success() {
            // Might be detached HEAD, try rev-parse
            let output = Command::new("git")
                .args(["rev-parse", "--short", "HEAD"])
                .current_dir(path)
                .output()?;

            if !output.status.success() {
                return Err(WorktreeError::CommandFailed(
                    String::from_utf8_lossy(&output.stderr).to_string(),
                ));
            }

            return Ok(format!(
                "detached@{}",
                String::from_utf8_lossy(&output.stdout).trim()
            ));
        }

        let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if branch.is_empty() {
            // Detached HEAD
            let output = Command::new("git")
                .args(["rev-parse", "--short", "HEAD"])
                .current_dir(path)
                .output()?;

            return Ok(format!(
                "detached@{}",
                String::from_utf8_lossy(&output.stdout).trim()
            ));
        }

        Ok(branch)
    }

    /// Check if a path is a git repository
    pub fn is_git_repo(&self, path: &Path) -> bool {
        path.join(".git").exists()
            || Command::new("git")
                .args(["rev-parse", "--git-dir"])
                .current_dir(path)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
    }

    /// Validate that a path is a git repository
    fn validate_git_repo(&self, path: &Path) -> Result<(), WorktreeError> {
        if !self.is_git_repo(path) {
            return Err(WorktreeError::NotAGitRepo(path.to_path_buf()));
        }
        Ok(())
    }

    /// Get the path for a worktree
    fn worktree_path(&self, repo_path: &Path, name: &str) -> PathBuf {
        if let Some(ref managed_dir) = self.managed_dir {
            // Use managed directory with repo name prefix
            let repo_name = repo_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("repo");
            managed_dir.join(repo_name).join(name)
        } else {
            // Create worktree next to repo in a worktrees subdirectory
            repo_path
                .parent()
                .unwrap_or(repo_path)
                .join("worktrees")
                .join(name)
        }
    }

    /// Parse the porcelain output of `git worktree list`
    fn parse_worktree_list(&self, output: &str) -> Result<Vec<WorktreeInfo>, WorktreeError> {
        let mut worktrees = Vec::new();
        let mut current_path: Option<PathBuf> = None;
        let mut current_head = String::new();
        let mut current_branch: Option<String> = None;
        let mut is_first = true;

        for line in output.lines() {
            if line.starts_with("worktree ") {
                // Save previous worktree if exists
                if let Some(path) = current_path.take() {
                    worktrees.push(WorktreeInfo {
                        path,
                        head: std::mem::take(&mut current_head),
                        branch: current_branch.take(),
                        is_main: is_first,
                    });
                    is_first = false;
                }
                current_path = Some(PathBuf::from(line.strip_prefix("worktree ").unwrap()));
            } else if line.starts_with("HEAD ") {
                current_head = line.strip_prefix("HEAD ").unwrap().to_string();
            } else if line.starts_with("branch ") {
                let branch = line.strip_prefix("branch ").unwrap();
                // Strip refs/heads/ prefix
                current_branch = Some(
                    branch
                        .strip_prefix("refs/heads/")
                        .unwrap_or(branch)
                        .to_string(),
                );
            }
        }

        // Don't forget the last worktree
        if let Some(path) = current_path {
            worktrees.push(WorktreeInfo {
                path,
                head: current_head,
                branch: current_branch,
                is_main: is_first,
            });
        }

        Ok(worktrees)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn init_git_repo(path: &Path) -> std::io::Result<()> {
        Command::new("git")
            .args(["init"])
            .current_dir(path)
            .output()?;

        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(path)
            .output()?;

        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(path)
            .output()?;

        // Create initial commit
        std::fs::write(path.join("README.md"), "# Test")?;
        Command::new("git")
            .args(["add", "."])
            .current_dir(path)
            .output()?;

        Command::new("git")
            .args(["commit", "-m", "Initial commit"])
            .current_dir(path)
            .output()?;

        Ok(())
    }

    #[test]
    fn test_is_git_repo() {
        let dir = tempdir().unwrap();
        let manager = WorktreeManager::new();

        assert!(!manager.is_git_repo(dir.path()));

        init_git_repo(dir.path()).unwrap();
        assert!(manager.is_git_repo(dir.path()));
    }

    #[test]
    fn test_get_current_branch() {
        let dir = tempdir().unwrap();
        init_git_repo(dir.path()).unwrap();

        let manager = WorktreeManager::new();
        let branch = manager.get_current_branch(dir.path()).unwrap();

        // Could be "main" or "master" depending on git config
        assert!(branch == "main" || branch == "master");
    }

    #[test]
    fn test_list_worktrees() {
        let dir = tempdir().unwrap();
        init_git_repo(dir.path()).unwrap();

        let manager = WorktreeManager::new();
        let worktrees = manager.list_worktrees(dir.path()).unwrap();

        assert_eq!(worktrees.len(), 1);
        assert!(worktrees[0].is_main);
    }

    #[test]
    fn test_create_and_remove_worktree() {
        let dir = tempdir().unwrap();
        let repo_path = dir.path().join("repo");
        std::fs::create_dir(&repo_path).unwrap();
        init_git_repo(&repo_path).unwrap();

        let manager = WorktreeManager::new();

        // Create a worktree
        let wt_path = manager
            .create_worktree(&repo_path, "feature-branch", "feature")
            .unwrap();

        assert!(wt_path.exists());

        // List worktrees
        let worktrees = manager.list_worktrees(&repo_path).unwrap();
        assert_eq!(worktrees.len(), 2);

        // Remove worktree
        manager.remove_worktree(&repo_path, &wt_path).unwrap();
        assert!(!wt_path.exists());

        let worktrees = manager.list_worktrees(&repo_path).unwrap();
        assert_eq!(worktrees.len(), 1);
    }
}
