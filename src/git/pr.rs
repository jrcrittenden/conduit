//! Pull Request management utilities
//!
//! This module provides preflight checks and prompt generation for PR creation.
//! The actual git/gh commands are executed by Claude Sonnet.

use std::path::Path;
use std::process::Command;

use serde::Deserialize;

/// PR status for a branch
#[derive(Debug, Clone)]
pub struct PrStatus {
    pub exists: bool,
    pub number: Option<u32>,
    pub url: Option<String>,
}

/// Result of preflight checks before PR creation
#[derive(Debug, Clone)]
pub struct PrPreflightResult {
    pub gh_installed: bool,
    pub gh_authenticated: bool,
    pub on_main_branch: bool,
    pub branch_name: String,
    pub target_branch: String,
    pub uncommitted_count: usize,
    pub has_upstream: bool,
    pub existing_pr: Option<PrStatus>,
}

/// JSON structure returned by `gh pr view --json`
#[derive(Debug, Deserialize)]
struct GhPrView {
    number: u32,
    url: String,
}

/// PR Manager for preflight checks and utilities
pub struct PrManager;

impl PrManager {
    /// Check if GitHub CLI (gh) is installed
    pub fn is_gh_installed() -> bool {
        Command::new("gh")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Check if gh is authenticated
    pub fn is_gh_authenticated() -> bool {
        Command::new("gh")
            .args(["auth", "status"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Get current branch name
    pub fn get_current_branch(working_dir: &Path) -> Option<String> {
        let output = Command::new("git")
            .args(["branch", "--show-current"])
            .current_dir(working_dir)
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let branch = String::from_utf8_lossy(&output.stdout)
            .trim()
            .to_string();

        if branch.is_empty() {
            None // Detached HEAD
        } else {
            Some(branch)
        }
    }

    /// Check if branch is a main branch (main, master, develop)
    pub fn is_main_branch(branch: &str) -> bool {
        matches!(branch, "main" | "master" | "develop")
    }

    /// Get default branch from remote
    pub fn get_default_branch(working_dir: &Path) -> String {
        // Try to get the default branch from origin
        let output = Command::new("git")
            .args(["symbolic-ref", "refs/remotes/origin/HEAD", "--short"])
            .current_dir(working_dir)
            .output();

        if let Ok(output) = output {
            if output.status.success() {
                let branch = String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .to_string();
                // Remove "origin/" prefix if present
                return branch
                    .strip_prefix("origin/")
                    .unwrap_or(&branch)
                    .to_string();
            }
        }

        // Fallback: check if main or master exists
        let check_main = Command::new("git")
            .args(["rev-parse", "--verify", "origin/main"])
            .current_dir(working_dir)
            .output();

        if check_main.map(|o| o.status.success()).unwrap_or(false) {
            return "main".to_string();
        }

        "master".to_string()
    }

    /// Check if current branch has an upstream
    pub fn has_upstream(working_dir: &Path) -> bool {
        Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "@{u}"])
            .current_dir(working_dir)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Count uncommitted changes (staged + unstaged + untracked)
    pub fn count_uncommitted_changes(working_dir: &Path) -> usize {
        let output = Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(working_dir)
            .output();

        match output {
            Ok(o) if o.status.success() => {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .filter(|line| !line.is_empty())
                    .count()
            }
            _ => 0,
        }
    }

    /// Check if a PR exists for the current branch
    pub fn get_existing_pr(working_dir: &Path) -> Option<PrStatus> {
        let output = Command::new("gh")
            .args(["pr", "view", "--json", "number,url"])
            .current_dir(working_dir)
            .output()
            .ok()?;

        if !output.status.success() {
            // No PR exists for this branch
            return Some(PrStatus {
                exists: false,
                number: None,
                url: None,
            });
        }

        let json_str = String::from_utf8_lossy(&output.stdout);
        if let Ok(pr) = serde_json::from_str::<GhPrView>(&json_str) {
            Some(PrStatus {
                exists: true,
                number: Some(pr.number),
                url: Some(pr.url),
            })
        } else {
            Some(PrStatus {
                exists: false,
                number: None,
                url: None,
            })
        }
    }

    /// Run all preflight checks
    pub fn preflight_check(working_dir: &Path) -> PrPreflightResult {
        let gh_installed = Self::is_gh_installed();
        let gh_authenticated = if gh_installed {
            Self::is_gh_authenticated()
        } else {
            false
        };

        let branch_name = Self::get_current_branch(working_dir).unwrap_or_default();
        let on_main_branch = Self::is_main_branch(&branch_name);
        let target_branch = format!("origin/{}", Self::get_default_branch(working_dir));
        let uncommitted_count = Self::count_uncommitted_changes(working_dir);
        let has_upstream = Self::has_upstream(working_dir);

        let existing_pr = if gh_installed && gh_authenticated && !on_main_branch {
            Self::get_existing_pr(working_dir)
        } else {
            None
        };

        PrPreflightResult {
            gh_installed,
            gh_authenticated,
            on_main_branch,
            branch_name,
            target_branch,
            uncommitted_count,
            has_upstream,
            existing_pr,
        }
    }

    /// Open existing PR in browser
    pub fn open_pr_in_browser(working_dir: &Path) -> std::io::Result<()> {
        use std::process::Stdio;
        // Suppress stdout/stderr to prevent TUI corruption
        Command::new("gh")
            .args(["pr", "view", "--web"])
            .current_dir(working_dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        Ok(())
    }

    /// Generate the prompt for Claude Sonnet to create a PR
    pub fn generate_pr_prompt(preflight: &PrPreflightResult) -> String {
        let upstream_note = if preflight.has_upstream {
            String::new()
        } else {
            "There is no upstream branch yet.\n".to_string()
        };

        let base_branch = preflight
            .target_branch
            .strip_prefix("origin/")
            .unwrap_or(&preflight.target_branch);

        format!(
            r#"The user likes the state of the code.

There are {} uncommitted changes.
The current branch is {}.
The target branch is {}.

{}The user requested a PR.

Follow these exact steps to create a PR:

1. Run git diff to review uncommitted changes
2. Commit them with a clear, descriptive commit message
3. Push to origin
4. Use git diff {}... to review the PR diff
5. Use gh pr create --base {} to create a PR. Keep the title under 80 characters and the description under five sentences.
6. If any of these steps fail, explain what went wrong."#,
            preflight.uncommitted_count,
            preflight.branch_name,
            preflight.target_branch,
            upstream_note,
            base_branch,
            base_branch,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_main_branch() {
        assert!(PrManager::is_main_branch("main"));
        assert!(PrManager::is_main_branch("master"));
        assert!(PrManager::is_main_branch("develop"));
        assert!(!PrManager::is_main_branch("feature/foo"));
        assert!(!PrManager::is_main_branch("fix/bar"));
    }

    #[test]
    fn test_generate_pr_prompt() {
        let preflight = PrPreflightResult {
            gh_installed: true,
            gh_authenticated: true,
            on_main_branch: false,
            branch_name: "feature/add-pr-support".to_string(),
            target_branch: "origin/main".to_string(),
            uncommitted_count: 5,
            has_upstream: false,
            existing_pr: None,
        };

        let prompt = PrManager::generate_pr_prompt(&preflight);
        assert!(prompt.contains("5 uncommitted changes"));
        assert!(prompt.contains("feature/add-pr-support"));
        assert!(prompt.contains("origin/main"));
        assert!(prompt.contains("no upstream branch"));
        assert!(prompt.contains("gh pr create --base main"));
    }
}
