use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use uuid::Uuid;

use crate::data::{
    CodeRabbitCategory, CodeRabbitItem, CodeRabbitItemSource, CodeRabbitItemStore, CodeRabbitMode,
    CodeRabbitRound, CodeRabbitRoundStatus, CodeRabbitRoundStore, CodeRabbitSeverity,
    RepositorySettings, RepositorySettingsStore, RepositoryStore, WorkspaceStore,
};
use crate::git::{PrStatus, PrStatusCheck};

const CODERABBIT_CONTEXT: &str = "CodeRabbit";
const CODERABBIT_LOGINS: [&str; 2] = ["coderabbitai[bot]", "coderabbitai"];
const CODERABBIT_WINDOW_SLACK_MINUTES: i64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeRabbitCheckState {
    Pending,
    Success,
    Failure,
    Error,
    Cancelled,
    Skipped,
    Unknown,
}

impl CodeRabbitCheckState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            CodeRabbitCheckState::Success
                | CodeRabbitCheckState::Failure
                | CodeRabbitCheckState::Error
                | CodeRabbitCheckState::Cancelled
                | CodeRabbitCheckState::Skipped
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            CodeRabbitCheckState::Pending => "pending",
            CodeRabbitCheckState::Success => "success",
            CodeRabbitCheckState::Failure => "failure",
            CodeRabbitCheckState::Error => "error",
            CodeRabbitCheckState::Cancelled => "cancelled",
            CodeRabbitCheckState::Skipped => "skipped",
            CodeRabbitCheckState::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CodeRabbitCompletion {
    pub pr_number: u32,
    pub head_sha: String,
    pub check_state: CodeRabbitCheckState,
    pub check_started_at: Option<String>,
}

pub fn detect_completion(old: Option<&PrStatus>, new: &PrStatus) -> Option<CodeRabbitCompletion> {
    let pr_number = new.number?;
    let head_sha = new.head_sha.clone()?;
    let new_check = find_coderabbit_check(&new.status_checks)?;
    let new_state = check_state_from_status(new_check);
    if !new_state.is_terminal() {
        return None;
    }

    let old_state = old
        .and_then(|status| find_coderabbit_check(&status.status_checks))
        .map(check_state_from_status);
    let started_at = new_check.started_at.clone();

    if let (Some(old_status), Some(old_check)) = (
        old_state,
        old.and_then(|status| find_coderabbit_check(&status.status_checks)),
    ) {
        let same_started_at = started_at.as_deref() == old_check.started_at.as_deref();
        if old_status.is_terminal() && old_status == new_state && same_started_at {
            return None;
        }
    }

    Some(CodeRabbitCompletion {
        pr_number,
        head_sha,
        check_state: new_state,
        check_started_at: started_at,
    })
}

fn find_coderabbit_check(checks: &[PrStatusCheck]) -> Option<&PrStatusCheck> {
    checks.iter().find(|check| {
        check
            .context
            .as_deref()
            .map(|value| value == CODERABBIT_CONTEXT)
            .unwrap_or(false)
            || check
                .name
                .as_deref()
                .map(|value| value == CODERABBIT_CONTEXT)
                .unwrap_or(false)
    })
}

fn check_state_from_status(check: &PrStatusCheck) -> CodeRabbitCheckState {
    if let Some(state) = check.state.as_deref() {
        return match state.to_ascii_uppercase().as_str() {
            "SUCCESS" => CodeRabbitCheckState::Success,
            "FAILURE" => CodeRabbitCheckState::Failure,
            "ERROR" => CodeRabbitCheckState::Error,
            "PENDING" | "EXPECTED" => CodeRabbitCheckState::Pending,
            _ => CodeRabbitCheckState::Unknown,
        };
    }

    let status = check.status.as_deref().unwrap_or("").to_ascii_uppercase();
    if status != "COMPLETED" {
        return CodeRabbitCheckState::Pending;
    }
    match check
        .conclusion
        .as_deref()
        .unwrap_or("")
        .to_ascii_uppercase()
        .as_str()
    {
        "SUCCESS" | "NEUTRAL" => CodeRabbitCheckState::Success,
        "FAILURE" | "TIMED_OUT" | "ACTION_REQUIRED" => CodeRabbitCheckState::Failure,
        "CANCELLED" => CodeRabbitCheckState::Cancelled,
        "SKIPPED" => CodeRabbitCheckState::Skipped,
        _ => CodeRabbitCheckState::Unknown,
    }
}

#[derive(Debug, Clone)]
pub struct CodeRabbitItemDraft {
    pub comment_id: i64,
    pub commit_id: Option<String>,
    pub source: CodeRabbitItemSource,
    pub category: CodeRabbitCategory,
    pub severity: Option<CodeRabbitSeverity>,
    pub file_path: Option<String>,
    pub line: Option<i64>,
    pub original_line: Option<i64>,
    pub diff_hunk: Option<String>,
    pub html_url: String,
    pub body: String,
    pub agent_prompt: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct CodeRabbitProcessor {
    repo_store: RepositoryStore,
    workspace_store: WorkspaceStore,
    settings_store: RepositorySettingsStore,
    round_store: CodeRabbitRoundStore,
    item_store: CodeRabbitItemStore,
}

impl CodeRabbitProcessor {
    pub fn new(
        repo_store: RepositoryStore,
        workspace_store: WorkspaceStore,
        settings_store: RepositorySettingsStore,
        round_store: CodeRabbitRoundStore,
        item_store: CodeRabbitItemStore,
    ) -> Self {
        Self {
            repo_store,
            workspace_store,
            settings_store,
            round_store,
            item_store,
        }
    }

    pub fn handle_completion(
        &self,
        workspace_id: Uuid,
        completion: CodeRabbitCompletion,
    ) -> Result<()> {
        let workspace = self
            .workspace_store
            .get_by_id(workspace_id)
            .context("Load workspace for CodeRabbit completion")?
            .ok_or_else(|| anyhow!("Workspace not found for CodeRabbit completion"))?;
        let repo_id = workspace.repository_id;
        let settings = self
            .settings_store
            .get_or_default(repo_id)
            .context("Load repository settings for CodeRabbit")?;
        if settings.coderabbit_mode == CodeRabbitMode::Disabled {
            return Ok(());
        }

        let observed_at = Utc::now();
        let check_started_at =
            parse_timestamp(completion.check_started_at.as_deref()).unwrap_or(observed_at);
        let check_started_at_key = check_started_at.to_rfc3339();
        let pr_number = completion.pr_number as i64;
        let head_sha = completion.head_sha.clone();

        let existing = self
            .round_store
            .get_by_key(repo_id, pr_number, &head_sha, &check_started_at_key)
            .context("Check existing CodeRabbit round")?;

        let mut round = if let Some(round) = existing {
            round
        } else {
            let round = CodeRabbitRound::new(
                repo_id,
                Some(workspace_id),
                pr_number,
                head_sha.clone(),
                completion.check_state.as_str().to_string(),
                check_started_at,
                observed_at,
            );
            self.round_store
                .create(&round)
                .context("Create CodeRabbit round")?;
            round
        };

        if round.status == CodeRabbitRoundStatus::Complete {
            return Ok(());
        }

        round.check_state = completion.check_state.as_str().to_string();
        round.workspace_id = round.workspace_id.or(Some(workspace_id));
        round.next_fetch_at = Some(observed_at);
        round.updated_at = observed_at;
        self.round_store
            .update(&round)
            .context("Update CodeRabbit round scheduling")?;

        let working_dir = self.resolve_working_dir(&round)?;
        self.fetch_and_update_round(round, &settings, &working_dir)
    }

    pub fn process_due_rounds(&self) -> Result<()> {
        let now = Utc::now();
        let due_rounds = self
            .round_store
            .list_pending_due(now)
            .context("Load pending CodeRabbit rounds")?;
        for round in due_rounds {
            let settings = self
                .settings_store
                .get_or_default(round.repository_id)
                .context("Load repository settings for CodeRabbit retry")?;
            if settings.coderabbit_mode == CodeRabbitMode::Disabled {
                continue;
            }
            let working_dir = match self.resolve_working_dir(&round) {
                Ok(path) => path,
                Err(error) => {
                    tracing::warn!(
                        round_id = %round.id,
                        error = %error,
                        "Skipping CodeRabbit round due to missing working dir"
                    );
                    continue;
                }
            };
            if let Err(error) = self.fetch_and_update_round(round, &settings, &working_dir) {
                tracing::warn!(
                    error = %error,
                    "CodeRabbit round processing failed"
                );
            }
        }
        Ok(())
    }

    pub fn cleanup_rounds_if_needed(&self, repository_id: Uuid, pr_number: i64) -> Result<()> {
        let settings = self
            .settings_store
            .get_or_default(repository_id)
            .context("Load repository settings for CodeRabbit cleanup")?;
        if settings.coderabbit_retention == crate::data::CodeRabbitRetention::DeleteOnClose {
            self.round_store
                .delete_by_pr(repository_id, pr_number)
                .context("Delete CodeRabbit rounds for closed PR")?;
        }
        Ok(())
    }

    fn resolve_working_dir(&self, round: &CodeRabbitRound) -> Result<PathBuf> {
        if let Some(workspace_id) = round.workspace_id {
            if let Some(workspace) = self
                .workspace_store
                .get_by_id(workspace_id)
                .context("Load workspace for CodeRabbit fetch")?
            {
                return Ok(workspace.path);
            }
        }
        let repo = self
            .repo_store
            .get_by_id(round.repository_id)
            .context("Load repository for CodeRabbit fetch")?
            .ok_or_else(|| anyhow!("Repository not found for CodeRabbit fetch"))?;
        repo.base_path
            .clone()
            .ok_or_else(|| anyhow!("Repository base path missing for CodeRabbit fetch"))
    }

    fn fetch_and_update_round(
        &self,
        mut round: CodeRabbitRound,
        settings: &RepositorySettings,
        working_dir: &Path,
    ) -> Result<()> {
        let observed_at = Utc::now();
        let fetcher = CodeRabbitFetcher::new(working_dir.to_path_buf());
        let drafts = match fetcher.fetch_actionable_items(round.pr_number as u32, observed_at) {
            Ok(items) => items,
            Err(error) => {
                tracing::warn!(
                    round_id = %round.id,
                    error = %error,
                    "CodeRabbit fetch failed"
                );
                Vec::new()
            }
        };

        let window_end = observed_at + Duration::minutes(CODERABBIT_WINDOW_SLACK_MINUTES);
        let mut current_round_drafts = Vec::new();
        let mut foreign_by_commit: HashMap<String, Vec<CodeRabbitItemDraft>> = HashMap::new();

        for draft in drafts {
            match draft.commit_id.as_deref() {
                Some(commit_id) if commit_id.eq_ignore_ascii_case(&round.head_sha) => {
                    current_round_drafts.push(draft);
                }
                Some(commit_id) => {
                    foreign_by_commit
                        .entry(commit_id.to_string())
                        .or_default()
                        .push(draft);
                }
                None => {
                    if within_window(draft.created_at, Some(round.check_started_at), window_end) {
                        current_round_drafts.push(draft);
                    }
                }
            }
        }

        let to_insert = build_items(round.id, &current_round_drafts);
        if !to_insert.is_empty() {
            self.item_store
                .insert_items(&to_insert)
                .context("Insert CodeRabbit items")?;
        }

        let total_actionable = self
            .item_store
            .count_for_round(round.id)
            .context("Count CodeRabbit items for round")?;

        round.attempt_count += 1;
        round.actionable_count = total_actionable;
        round.updated_at = observed_at;

        if total_actionable > 0 {
            round.status = CodeRabbitRoundStatus::Complete;
            round.completed_at = Some(observed_at);
            round.next_fetch_at = None;
        } else {
            let backoff = &settings.coderabbit_backoff_seconds;
            if (round.attempt_count as usize) >= backoff.len() {
                round.status = CodeRabbitRoundStatus::Complete;
                round.completed_at = Some(observed_at);
                round.next_fetch_at = None;
            } else {
                let delay = backoff[round.attempt_count as usize - 1];
                round.next_fetch_at = Some(observed_at + Duration::seconds(delay));
            }
        }

        self.round_store
            .update(&round)
            .context("Update CodeRabbit round after fetch")?;

        self.capture_foreign_rounds(&round, foreign_by_commit, observed_at)
    }

    fn capture_foreign_rounds(
        &self,
        base_round: &CodeRabbitRound,
        foreign_by_commit: HashMap<String, Vec<CodeRabbitItemDraft>>,
        observed_at: DateTime<Utc>,
    ) -> Result<()> {
        for (commit_id, drafts) in foreign_by_commit {
            self.capture_round_for_commit(base_round, &commit_id, &drafts, observed_at)?;
        }
        Ok(())
    }

    fn capture_round_for_commit(
        &self,
        base_round: &CodeRabbitRound,
        commit_id: &str,
        drafts: &[CodeRabbitItemDraft],
        observed_at: DateTime<Utc>,
    ) -> Result<()> {
        let existing = self
            .round_store
            .get_latest_for_head(base_round.repository_id, base_round.pr_number, commit_id)
            .context("Load CodeRabbit round by commit")?;

        let mut round = if let Some(round) = existing {
            round
        } else {
            let check_started_at = drafts
                .iter()
                .map(|draft| draft.created_at)
                .min()
                .unwrap_or(observed_at);
            let round = CodeRabbitRound::new(
                base_round.repository_id,
                base_round.workspace_id,
                base_round.pr_number,
                commit_id.to_string(),
                CodeRabbitCheckState::Unknown.as_str().to_string(),
                check_started_at,
                observed_at,
            );
            self.round_store
                .create(&round)
                .context("Create CodeRabbit round for prior commit")?;
            round
        };

        let to_insert = build_items(round.id, drafts);
        if !to_insert.is_empty() {
            self.item_store
                .insert_items(&to_insert)
                .context("Insert CodeRabbit items for prior commit")?;
        }

        let total_actionable = self
            .item_store
            .count_for_round(round.id)
            .context("Count CodeRabbit items for prior commit")?;

        if total_actionable > 0 && round.status != CodeRabbitRoundStatus::Complete {
            round.status = CodeRabbitRoundStatus::Complete;
            round.completed_at = Some(observed_at);
            round.next_fetch_at = None;
        }
        round.actionable_count = total_actionable;
        round.updated_at = observed_at;
        self.round_store
            .update(&round)
            .context("Update CodeRabbit round for prior commit")?;
        Ok(())
    }
}

struct CodeRabbitFetcher {
    working_dir: PathBuf,
}

impl CodeRabbitFetcher {
    fn new(working_dir: PathBuf) -> Self {
        Self { working_dir }
    }

    fn fetch_actionable_items(
        &self,
        pr_number: u32,
        observed_at: DateTime<Utc>,
    ) -> Result<Vec<CodeRabbitItemDraft>> {
        let review_comments: Vec<GitHubReviewComment> = gh_api_paginated(
            &self.working_dir,
            &format!(
                "repos/{{owner}}/{{repo}}/pulls/{}/comments?per_page=100",
                pr_number
            ),
        )
        .context("Fetch CodeRabbit review comments")?;

        let issue_comments: Vec<GitHubIssueComment> = gh_api_paginated(
            &self.working_dir,
            &format!(
                "repos/{{owner}}/{{repo}}/issues/{}/comments?per_page=100",
                pr_number
            ),
        )
        .context("Fetch CodeRabbit issue comments")?;

        let reviews: Vec<GitHubReview> = gh_api_paginated(
            &self.working_dir,
            &format!(
                "repos/{{owner}}/{{repo}}/pulls/{}/reviews?per_page=100",
                pr_number
            ),
        )
        .context("Fetch CodeRabbit reviews")?;

        let mut drafts = Vec::new();

        for comment in review_comments {
            if !is_coderabbit_login(&comment.user.login) {
                continue;
            }
            if let Some((category, severity)) = parse_actionable(&comment.body) {
                let created_at = parse_timestamp(Some(&comment.created_at)).unwrap_or(observed_at);
                let updated_at = parse_timestamp(Some(&comment.updated_at)).unwrap_or(created_at);
                drafts.push(CodeRabbitItemDraft {
                    comment_id: comment.id,
                    commit_id: comment.commit_id.clone(),
                    source: CodeRabbitItemSource::ReviewComment,
                    category,
                    severity,
                    file_path: comment.path,
                    line: comment.line,
                    original_line: comment.original_line,
                    diff_hunk: comment.diff_hunk,
                    html_url: comment.html_url,
                    body: comment.body.clone(),
                    agent_prompt: extract_prompt_for_ai_agents(&comment.body),
                    created_at,
                    updated_at,
                });
            }
        }

        for comment in issue_comments {
            if !is_coderabbit_login(&comment.user.login) {
                continue;
            }
            if let Some((category, severity)) = parse_actionable(&comment.body) {
                let created_at = parse_timestamp(Some(&comment.created_at)).unwrap_or(observed_at);
                let updated_at = parse_timestamp(Some(&comment.updated_at)).unwrap_or(created_at);
                drafts.push(CodeRabbitItemDraft {
                    comment_id: comment.id,
                    commit_id: None,
                    source: CodeRabbitItemSource::IssueComment,
                    category,
                    severity,
                    file_path: None,
                    line: None,
                    original_line: None,
                    diff_hunk: None,
                    html_url: comment.html_url,
                    body: comment.body.clone(),
                    agent_prompt: extract_prompt_for_ai_agents(&comment.body),
                    created_at,
                    updated_at,
                });
            }
        }

        for review in reviews {
            if !is_coderabbit_login(&review.user.login) {
                continue;
            }
            let body = review.body.unwrap_or_default();
            if body.is_empty() {
                continue;
            }
            if let Some((category, severity)) = parse_actionable(&body) {
                let created_at =
                    parse_timestamp(review.submitted_at.as_deref()).unwrap_or(observed_at);
                drafts.push(CodeRabbitItemDraft {
                    comment_id: review.id,
                    commit_id: review.commit_id.clone(),
                    source: CodeRabbitItemSource::Review,
                    category,
                    severity,
                    file_path: None,
                    line: None,
                    original_line: None,
                    diff_hunk: None,
                    html_url: review.html_url,
                    body: body.clone(),
                    agent_prompt: extract_prompt_for_ai_agents(&body),
                    created_at,
                    updated_at: created_at,
                });
            }
        }

        Ok(drafts)
    }
}

fn gh_api_paginated<T: DeserializeOwned>(working_dir: &Path, endpoint: &str) -> Result<Vec<T>> {
    let output = Command::new("gh")
        .args(["api", "--paginate", "--slurp", endpoint])
        .current_dir(working_dir)
        .output()
        .context("Run gh api for CodeRabbit")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("gh api failed: {}", stderr.trim()));
    }

    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("Parse gh api output as JSON")?;
    parse_paginated(value)
}

fn parse_paginated<T: DeserializeOwned>(value: serde_json::Value) -> Result<Vec<T>> {
    match value {
        serde_json::Value::Array(pages) => {
            if pages
                .iter()
                .all(|page| matches!(page, serde_json::Value::Array(_)))
            {
                let mut items = Vec::new();
                for page in pages {
                    if let serde_json::Value::Array(entries) = page {
                        for entry in entries {
                            let parsed =
                                serde_json::from_value(entry).context("Parse gh api item")?;
                            items.push(parsed);
                        }
                    }
                }
                Ok(items)
            } else {
                let mut items = Vec::new();
                for entry in pages {
                    let parsed = serde_json::from_value(entry).context("Parse gh api item")?;
                    items.push(parsed);
                }
                Ok(items)
            }
        }
        other => {
            let parsed = serde_json::from_value(other).context("Parse gh api payload")?;
            Ok(vec![parsed])
        }
    }
}

fn is_coderabbit_login(login: &str) -> bool {
    CODERABBIT_LOGINS
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(login))
}

fn within_window(
    created_at: DateTime<Utc>,
    window_start: Option<DateTime<Utc>>,
    window_end: DateTime<Utc>,
) -> bool {
    if created_at > window_end {
        return false;
    }
    if let Some(start) = window_start {
        return created_at >= start;
    }
    true
}

fn parse_actionable(body: &str) -> Option<(CodeRabbitCategory, Option<CodeRabbitSeverity>)> {
    let lower = body.to_ascii_lowercase();
    let category = if lower.contains("potential issue") {
        CodeRabbitCategory::PotentialIssue
    } else if lower.contains("refactor suggestion") {
        CodeRabbitCategory::RefactorSuggestion
    } else {
        return None;
    };

    let severity = if lower.contains("critical") {
        Some(CodeRabbitSeverity::Critical)
    } else if lower.contains("major") {
        Some(CodeRabbitSeverity::Major)
    } else if lower.contains("minor") {
        Some(CodeRabbitSeverity::Minor)
    } else if lower.contains("trivial") {
        Some(CodeRabbitSeverity::Trivial)
    } else if lower.contains("info") {
        Some(CodeRabbitSeverity::Info)
    } else {
        None
    };

    Some((category, severity))
}

fn extract_prompt_for_ai_agents(body: &str) -> Option<String> {
    let marker = "Prompt for AI Agents";
    let start = body.find(marker)?;
    let rest = &body[start..];
    let fence_start = rest.find("```")?;
    let rest = &rest[fence_start + 3..];
    let fence_end = rest.find("```")?;
    let prompt = rest[..fence_end].trim();
    if prompt.is_empty() {
        None
    } else {
        Some(prompt.to_string())
    }
}

fn parse_timestamp(value: Option<&str>) -> Option<DateTime<Utc>> {
    value
        .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
        .map(|dt| dt.with_timezone(&Utc))
}

fn build_items(round_id: Uuid, drafts: &[CodeRabbitItemDraft]) -> Vec<CodeRabbitItem> {
    drafts
        .iter()
        .map(|draft| CodeRabbitItem {
            id: Uuid::new_v4(),
            round_id,
            comment_id: draft.comment_id,
            source: draft.source,
            category: draft.category,
            severity: draft.severity,
            file_path: draft.file_path.clone(),
            line: draft.line,
            original_line: draft.original_line,
            diff_hunk: draft.diff_hunk.clone(),
            html_url: draft.html_url.clone(),
            body: draft.body.clone(),
            agent_prompt: draft.agent_prompt.clone(),
            created_at: draft.created_at,
            updated_at: draft.updated_at,
        })
        .collect()
}

#[derive(Debug, Deserialize)]
struct GitHubUser {
    login: String,
}

#[derive(Debug, Deserialize)]
struct GitHubReviewComment {
    id: i64,
    user: GitHubUser,
    body: String,
    html_url: String,
    path: Option<String>,
    line: Option<i64>,
    original_line: Option<i64>,
    diff_hunk: Option<String>,
    commit_id: Option<String>,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Deserialize)]
struct GitHubIssueComment {
    id: i64,
    user: GitHubUser,
    body: String,
    html_url: String,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Deserialize)]
struct GitHubReview {
    id: i64,
    user: GitHubUser,
    body: Option<String>,
    html_url: String,
    commit_id: Option<String>,
    #[serde(rename = "submitted_at")]
    submitted_at: Option<String>,
}
