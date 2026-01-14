use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use uuid::Uuid;

use crate::data::{
    CodeRabbitCategory, CodeRabbitComment, CodeRabbitCommentStore, CodeRabbitFeedbackScope,
    CodeRabbitItem, CodeRabbitItemKind, CodeRabbitItemSource, CodeRabbitItemStore, CodeRabbitMode,
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
    pub check_completed_at: Option<String>,
}

pub fn detect_completion(old: Option<&PrStatus>, new: &PrStatus) -> Option<CodeRabbitCompletion> {
    let pr_number = new.number?;
    let head_sha = new.head_sha.clone()?;
    let new_check = find_coderabbit_check(&new.status_checks)?;
    let new_state = check_state_from_status(new_check);
    if !new_state.is_terminal() {
        return None;
    }

    let old_check = old.and_then(|status| find_coderabbit_check(&status.status_checks));
    let old_state = old_check.map(check_state_from_status);
    let started_at = new_check.started_at.clone();
    let completed_at = new_check.completed_at.clone();
    let new_key = check_run_key(new_check);

    if let (Some(old_status), Some(old_check)) = (old_state, old_check) {
        let old_key = check_run_key(old_check);
        if old_status.is_terminal() && old_status == new_state && old_key == new_key {
            return None;
        }
    }

    Some(CodeRabbitCompletion {
        pr_number,
        head_sha,
        check_state: new_state,
        check_started_at: started_at,
        check_completed_at: completed_at,
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

fn check_run_key(check: &PrStatusCheck) -> Option<&str> {
    check
        .started_at
        .as_deref()
        .or_else(|| check.completed_at.as_deref())
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
pub struct CodeRabbitCommentDraft {
    pub comment_id: i64,
    pub commit_id: Option<String>,
    pub source: CodeRabbitItemSource,
    pub html_url: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CodeRabbitItemDraft {
    pub comment_id: i64,
    pub commit_id: Option<String>,
    pub source: CodeRabbitItemSource,
    pub kind: CodeRabbitItemKind,
    pub actionable: bool,
    pub category: Option<CodeRabbitCategory>,
    pub severity: Option<CodeRabbitSeverity>,
    pub section: Option<String>,
    pub file_path: Option<String>,
    pub line: Option<i64>,
    pub line_start: Option<i64>,
    pub line_end: Option<i64>,
    pub original_line: Option<i64>,
    pub diff_hunk: Option<String>,
    pub html_url: String,
    pub body: String,
    pub agent_prompt: Option<String>,
    pub item_key: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct CodeRabbitProcessor {
    repo_store: RepositoryStore,
    workspace_store: WorkspaceStore,
    settings_store: RepositorySettingsStore,
    round_store: CodeRabbitRoundStore,
    comment_store: CodeRabbitCommentStore,
    item_store: CodeRabbitItemStore,
}

impl CodeRabbitProcessor {
    pub fn new(
        repo_store: RepositoryStore,
        workspace_store: WorkspaceStore,
        settings_store: RepositorySettingsStore,
        round_store: CodeRabbitRoundStore,
        comment_store: CodeRabbitCommentStore,
        item_store: CodeRabbitItemStore,
    ) -> Self {
        Self {
            repo_store,
            workspace_store,
            settings_store,
            round_store,
            comment_store,
            item_store,
        }
    }

    pub fn handle_completion(
        &self,
        workspace_id: Uuid,
        completion: CodeRabbitCompletion,
    ) -> Result<Option<CodeRabbitRound>> {
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
            return Ok(None);
        }

        let observed_at = Utc::now();
        let pr_number = completion.pr_number as i64;
        let head_sha = completion.head_sha.clone();
        let parsed_started_at = parse_timestamp(completion.check_started_at.as_deref());
        let parsed_completed_at = parse_timestamp(completion.check_completed_at.as_deref());
        let check_started_at = parsed_started_at
            .or(parsed_completed_at)
            .unwrap_or(observed_at);
        let check_started_at_key = check_started_at.to_rfc3339();

        let existing = if parsed_started_at.is_some() {
            self.round_store
                .get_by_key(repo_id, pr_number, &head_sha, &check_started_at_key)
                .context("Check existing CodeRabbit round")?
        } else {
            self.round_store
                .get_latest_for_head(repo_id, pr_number, &head_sha)
                .context("Check existing CodeRabbit round by head")?
        };

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
            return Ok(None);
        }

        round.check_state = completion.check_state.as_str().to_string();
        round.workspace_id = round.workspace_id.or(Some(workspace_id));
        round.next_fetch_at = Some(observed_at);
        round.updated_at = observed_at;
        self.round_store
            .update(&round)
            .context("Update CodeRabbit round scheduling")?;

        let working_dir = self.resolve_working_dir(&round)?;
        let round = self.fetch_and_update_round(round, &settings, &working_dir)?;
        Ok(Some(round))
    }

    pub fn process_due_rounds(&self) -> Result<Vec<CodeRabbitRound>> {
        let now = Utc::now();
        let due_rounds = self
            .round_store
            .list_pending_due(now)
            .context("Load pending CodeRabbit rounds")?;
        let mut processed = Vec::new();
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
            match self.fetch_and_update_round(round, &settings, &working_dir) {
                Ok(round) => processed.push(round),
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "CodeRabbit round processing failed"
                    );
                }
            }
        }
        Ok(processed)
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

    pub fn get_latest_round_for_pr(
        &self,
        repository_id: Uuid,
        pr_number: i64,
    ) -> Result<Option<CodeRabbitRound>> {
        self.round_store
            .get_latest_for_pr(repository_id, pr_number)
            .context("Load CodeRabbit round for PR")
    }

    pub fn list_items_for_round(&self, round_id: Uuid) -> Result<Vec<CodeRabbitItem>> {
        self.item_store
            .list_for_round(round_id)
            .context("Load CodeRabbit items for round")
    }

    pub fn mark_round_notified(&self, round_id: Uuid, notified_at: DateTime<Utc>) -> Result<()> {
        self.round_store
            .mark_notified(round_id, notified_at)
            .context("Mark CodeRabbit round notified")
    }

    pub fn mark_round_processed(&self, round_id: Uuid, processed_at: DateTime<Utc>) -> Result<()> {
        self.round_store
            .mark_processed(round_id, processed_at)
            .context("Mark CodeRabbit round processed")
    }

    pub fn get_settings(&self, repository_id: Uuid) -> Result<RepositorySettings> {
        self.settings_store
            .get_or_default(repository_id)
            .context("Load repository settings for CodeRabbit")
    }

    pub fn save_settings(&self, settings: &RepositorySettings) -> Result<()> {
        self.settings_store
            .upsert(settings)
            .context("Update repository settings for CodeRabbit")
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
    ) -> Result<CodeRabbitRound> {
        let observed_at = Utc::now();
        let fetcher = CodeRabbitFetcher::new(working_dir.to_path_buf());
        let fetched = match fetcher.fetch_feedback(round.pr_number as u32, observed_at) {
            Ok(result) => result,
            Err(error) => {
                tracing::warn!(
                    round_id = %round.id,
                    error = %error,
                    "CodeRabbit fetch failed"
                );
                CodeRabbitFetchResult::default()
            }
        };

        let window_end = observed_at + Duration::minutes(CODERABBIT_WINDOW_SLACK_MINUTES);
        let mut current_comment_drafts = Vec::new();
        let mut foreign_comments_by_commit: HashMap<String, Vec<CodeRabbitCommentDraft>> =
            HashMap::new();

        for draft in fetched.comments {
            match draft.commit_id.as_deref() {
                Some(commit_id) if commit_id.eq_ignore_ascii_case(&round.head_sha) => {
                    current_comment_drafts.push(draft);
                }
                Some(commit_id) => {
                    foreign_comments_by_commit
                        .entry(commit_id.to_string())
                        .or_default()
                        .push(draft);
                }
                None => {
                    if within_window(draft.created_at, Some(round.check_started_at), window_end) {
                        current_comment_drafts.push(draft);
                    }
                }
            }
        }

        let mut current_item_drafts = Vec::new();
        let mut foreign_items_by_commit: HashMap<String, Vec<CodeRabbitItemDraft>> = HashMap::new();

        for draft in fetched.items {
            match draft.commit_id.as_deref() {
                Some(commit_id) if commit_id.eq_ignore_ascii_case(&round.head_sha) => {
                    current_item_drafts.push(draft);
                }
                Some(commit_id) => {
                    foreign_items_by_commit
                        .entry(commit_id.to_string())
                        .or_default()
                        .push(draft);
                }
                None => {
                    if within_window(draft.created_at, Some(round.check_started_at), window_end) {
                        current_item_drafts.push(draft);
                    }
                }
            }
        }

        let scope = settings.coderabbit_scope;
        if scope == CodeRabbitFeedbackScope::ActionableOnly {
            current_item_drafts.retain(|draft| draft.actionable);
            for drafts in foreign_items_by_commit.values_mut() {
                drafts.retain(|draft| draft.actionable);
            }
            foreign_items_by_commit.retain(|_, drafts| !drafts.is_empty());
            current_comment_drafts.clear();
            foreign_comments_by_commit.clear();
        }

        if scope == CodeRabbitFeedbackScope::All {
            let comments_to_insert = build_comments(round.id, &current_comment_drafts);
            if !comments_to_insert.is_empty() {
                self.comment_store
                    .insert_comments(&comments_to_insert)
                    .context("Insert CodeRabbit comments")?;
            }
        }

        let items_to_insert = build_items(round.id, &current_item_drafts);
        if !items_to_insert.is_empty() {
            self.item_store
                .insert_items(&items_to_insert)
                .context("Insert CodeRabbit items")?;
        }

        let total_count = self
            .item_store
            .count_for_round(round.id)
            .context("Count CodeRabbit items for round")?;
        let actionable_count = self
            .item_store
            .count_actionable_for_round(round.id)
            .context("Count actionable CodeRabbit items for round")?;

        round.attempt_count += 1;
        round.total_count = total_count;
        round.actionable_count = actionable_count;
        round.updated_at = observed_at;

        if total_count > 0 {
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

        self.capture_foreign_rounds(
            &round,
            foreign_comments_by_commit,
            foreign_items_by_commit,
            observed_at,
            scope,
        )?;

        Ok(round)
    }

    fn capture_foreign_rounds(
        &self,
        base_round: &CodeRabbitRound,
        foreign_comments_by_commit: HashMap<String, Vec<CodeRabbitCommentDraft>>,
        foreign_items_by_commit: HashMap<String, Vec<CodeRabbitItemDraft>>,
        observed_at: DateTime<Utc>,
        scope: CodeRabbitFeedbackScope,
    ) -> Result<()> {
        let mut grouped: HashMap<String, (Vec<CodeRabbitCommentDraft>, Vec<CodeRabbitItemDraft>)> =
            HashMap::new();

        for (commit_id, drafts) in foreign_comments_by_commit {
            grouped
                .entry(commit_id)
                .or_insert_with(|| (Vec::new(), Vec::new()))
                .0
                .extend(drafts);
        }

        for (commit_id, drafts) in foreign_items_by_commit {
            grouped
                .entry(commit_id)
                .or_insert_with(|| (Vec::new(), Vec::new()))
                .1
                .extend(drafts);
        }

        for (commit_id, (comment_drafts, item_drafts)) in grouped {
            if comment_drafts.is_empty() && item_drafts.is_empty() {
                continue;
            }
            self.capture_round_for_commit(
                base_round,
                &commit_id,
                &comment_drafts,
                &item_drafts,
                observed_at,
                scope,
            )?;
        }
        Ok(())
    }

    fn capture_round_for_commit(
        &self,
        base_round: &CodeRabbitRound,
        commit_id: &str,
        comment_drafts: &[CodeRabbitCommentDraft],
        item_drafts: &[CodeRabbitItemDraft],
        observed_at: DateTime<Utc>,
        scope: CodeRabbitFeedbackScope,
    ) -> Result<()> {
        if comment_drafts.is_empty() && item_drafts.is_empty() {
            return Ok(());
        }
        let existing = self
            .round_store
            .get_latest_for_head(base_round.repository_id, base_round.pr_number, commit_id)
            .context("Load CodeRabbit round by commit")?;

        let mut round = if let Some(round) = existing {
            round
        } else {
            let check_started_at = comment_drafts
                .iter()
                .map(|draft| draft.created_at)
                .chain(item_drafts.iter().map(|draft| draft.created_at))
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

        if scope == CodeRabbitFeedbackScope::All {
            let comments_to_insert = build_comments(round.id, comment_drafts);
            if !comments_to_insert.is_empty() {
                self.comment_store
                    .insert_comments(&comments_to_insert)
                    .context("Insert CodeRabbit comments for prior commit")?;
            }
        }

        let items_to_insert = build_items(round.id, item_drafts);
        if !items_to_insert.is_empty() {
            self.item_store
                .insert_items(&items_to_insert)
                .context("Insert CodeRabbit items for prior commit")?;
        }

        let total_count = self
            .item_store
            .count_for_round(round.id)
            .context("Count CodeRabbit items for prior commit")?;
        let actionable_count = self
            .item_store
            .count_actionable_for_round(round.id)
            .context("Count actionable CodeRabbit items for prior commit")?;

        if total_count > 0 && round.status != CodeRabbitRoundStatus::Complete {
            round.status = CodeRabbitRoundStatus::Complete;
            round.completed_at = Some(observed_at);
            round.next_fetch_at = None;
        }
        round.total_count = total_count;
        round.actionable_count = actionable_count;
        round.updated_at = observed_at;
        self.round_store
            .update(&round)
            .context("Update CodeRabbit round for prior commit")?;
        Ok(())
    }
}

#[derive(Default)]
struct CodeRabbitFetchResult {
    comments: Vec<CodeRabbitCommentDraft>,
    items: Vec<CodeRabbitItemDraft>,
}

struct CodeRabbitFetcher {
    working_dir: PathBuf,
}

impl CodeRabbitFetcher {
    fn new(working_dir: PathBuf) -> Self {
        Self { working_dir }
    }

    fn fetch_feedback(
        &self,
        pr_number: u32,
        observed_at: DateTime<Utc>,
    ) -> Result<CodeRabbitFetchResult> {
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

        let mut comments = Vec::new();
        let mut items = Vec::new();

        for comment in review_comments {
            if !is_coderabbit_login(&comment.user.login) {
                continue;
            }
            let created_at = parse_timestamp(Some(&comment.created_at)).unwrap_or(observed_at);
            let updated_at = parse_timestamp(Some(&comment.updated_at)).unwrap_or(created_at);
            let body = sanitize_comment_body(&comment.body);
            comments.push(CodeRabbitCommentDraft {
                comment_id: comment.id,
                commit_id: comment.commit_id.clone(),
                source: CodeRabbitItemSource::ReviewComment,
                html_url: comment.html_url.clone(),
                body: body.clone(),
                created_at,
                updated_at,
            });

            if let Some((category, severity)) = parse_actionable(&body) {
                let line_start = comment.line.or(comment.original_line);
                items.push(CodeRabbitItemDraft {
                    comment_id: comment.id,
                    commit_id: comment.commit_id.clone(),
                    source: CodeRabbitItemSource::ReviewComment,
                    kind: CodeRabbitItemKind::Actionable,
                    actionable: true,
                    category: Some(category),
                    severity,
                    section: None,
                    file_path: comment.path.clone(),
                    line: comment.line,
                    line_start,
                    line_end: line_start,
                    original_line: comment.original_line,
                    diff_hunk: comment.diff_hunk.clone(),
                    html_url: comment.html_url.clone(),
                    body: body.clone(),
                    agent_prompt: extract_prompt_for_ai_agents(&body),
                    item_key: item_key_for_inline_comment(
                        CodeRabbitItemSource::ReviewComment,
                        comment.id,
                    ),
                    created_at,
                    updated_at,
                });
            }
        }

        for comment in issue_comments {
            if !is_coderabbit_login(&comment.user.login) {
                continue;
            }
            let created_at = parse_timestamp(Some(&comment.created_at)).unwrap_or(observed_at);
            let updated_at = parse_timestamp(Some(&comment.updated_at)).unwrap_or(created_at);
            let body = sanitize_comment_body(&comment.body);
            comments.push(CodeRabbitCommentDraft {
                comment_id: comment.id,
                commit_id: None,
                source: CodeRabbitItemSource::IssueComment,
                html_url: comment.html_url.clone(),
                body: body.clone(),
                created_at,
                updated_at,
            });
            items.extend(extract_section_items(
                &body,
                CodeRabbitItemSource::IssueComment,
                comment.id,
                None,
                &comment.html_url,
                created_at,
                updated_at,
            ));
        }

        for review in reviews {
            if !is_coderabbit_login(&review.user.login) {
                continue;
            }
            let body = review.body.unwrap_or_default();
            if body.is_empty() {
                continue;
            }
            let created_at = parse_timestamp(review.submitted_at.as_deref()).unwrap_or(observed_at);
            let body = sanitize_comment_body(&body);
            comments.push(CodeRabbitCommentDraft {
                comment_id: review.id,
                commit_id: review.commit_id.clone(),
                source: CodeRabbitItemSource::Review,
                html_url: review.html_url.clone(),
                body: body.clone(),
                created_at,
                updated_at: created_at,
            });
            items.extend(extract_section_items(
                &body,
                CodeRabbitItemSource::Review,
                review.id,
                review.commit_id.clone(),
                &review.html_url,
                created_at,
                created_at,
            ));
        }

        Ok(CodeRabbitFetchResult { comments, items })
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

fn sanitize_comment_body(body: &str) -> String {
    strip_internal_state(body)
}

fn strip_internal_state(body: &str) -> String {
    let start_marker = "<!-- internal state start -->";
    let end_marker = "<!-- internal state end -->";
    let mut result = String::with_capacity(body.len());
    let mut remaining = body;

    loop {
        let Some(start) = remaining.find(start_marker) else {
            result.push_str(remaining);
            break;
        };
        let after_start = &remaining[start + start_marker.len()..];
        let Some(end) = after_start.find(end_marker) else {
            return body.to_string();
        };
        result.push_str(&remaining[..start]);
        remaining = &after_start[end + end_marker.len()..];
    }

    result
}

fn extract_section_items(
    body: &str,
    source: CodeRabbitItemSource,
    comment_id: i64,
    commit_id: Option<String>,
    html_url: &str,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
) -> Vec<CodeRabbitItemDraft> {
    let mut drafts = Vec::new();
    let sections = [
        (
            "outside diff range comments",
            CodeRabbitItemKind::OutsideDiff,
            "outside-diff-range",
        ),
        ("nitpick comments", CodeRabbitItemKind::Nitpick, "nitpick"),
    ];

    for (label, kind, section) in sections {
        for item in parse_section_items(body, label) {
            let key = item_key_for_section(
                kind,
                &item.file_path,
                item.line_start,
                item.line_end,
                item.title.as_deref(),
                &item.body,
            );
            let line = if item.line_start == item.line_end {
                Some(item.line_start)
            } else {
                None
            };
            let actionable = !matches!(kind, CodeRabbitItemKind::Nitpick);
            drafts.push(CodeRabbitItemDraft {
                comment_id,
                commit_id: commit_id.clone(),
                source,
                kind,
                actionable,
                category: None,
                severity: None,
                section: Some(section.to_string()),
                file_path: Some(item.file_path),
                line,
                line_start: Some(item.line_start),
                line_end: Some(item.line_end),
                original_line: None,
                diff_hunk: None,
                html_url: html_url.to_string(),
                body: item.body,
                agent_prompt: None,
                item_key: key,
                created_at,
                updated_at,
            });
        }
    }

    drafts
}

#[derive(Debug)]
struct ParsedSectionItem {
    file_path: String,
    line_start: i64,
    line_end: i64,
    title: Option<String>,
    body: String,
}

fn parse_section_items(body: &str, label: &str) -> Vec<ParsedSectionItem> {
    let mut items = Vec::new();
    for block in find_section_blocks(body, label) {
        items.extend(parse_items_from_block(block));
    }
    items
}

fn find_section_blocks<'a>(body: &'a str, label: &str) -> Vec<&'a str> {
    let mut blocks = Vec::new();
    let label_lower = label.to_ascii_lowercase();
    let mut offset = 0usize;
    for line in body.lines() {
        let lower = line.to_ascii_lowercase();
        if lower.contains("<summary>") && lower.contains(&label_lower) {
            if let Some(start) = body[..offset].rfind("<details>") {
                if let Some((block, _end)) = extract_details_block(body, start) {
                    blocks.push(block);
                }
            }
        }
        offset = offset.saturating_add(line.len() + 1);
    }
    blocks
}

fn extract_details_block(body: &str, start: usize) -> Option<(&str, usize)> {
    let mut idx = start;
    let mut depth = 0i64;
    while idx < body.len() {
        let rest = &body[idx..];
        let next_open = rest.find("<details>");
        let next_close = rest.find("</details>");
        let (next_pos, is_open) = match (next_open, next_close) {
            (Some(open), Some(close)) => {
                if open <= close {
                    (open, true)
                } else {
                    (close, false)
                }
            }
            (Some(open), None) => (open, true),
            (None, Some(close)) => (close, false),
            (None, None) => break,
        };
        let tag_start = idx + next_pos;
        if is_open {
            depth += 1;
            idx = tag_start + "<details>".len();
        } else {
            depth -= 1;
            idx = tag_start + "</details>".len();
            if depth == 0 {
                return Some((&body[start..idx], idx));
            }
        }
    }
    None
}

fn parse_items_from_block(block: &str) -> Vec<ParsedSectionItem> {
    let mut items = Vec::new();
    let mut current_file: Option<String> = None;
    let mut current_item: Option<ItemBuilder> = None;

    for line in block.lines() {
        let normalized = strip_blockquote_prefix(line);
        if let Some(file_path) = parse_file_summary_line(normalized) {
            finalize_item(&mut current_item, &mut items);
            current_file = Some(file_path);
            continue;
        }

        if let Some(header) = parse_item_header_line(normalized) {
            finalize_item(&mut current_item, &mut items);
            current_item = Some(ItemBuilder {
                file_path: current_file.clone(),
                line_start: Some(header.line_start),
                line_end: Some(header.line_end),
                title: header.title,
                body_lines: vec![normalized.trim().to_string()],
            });
            continue;
        }

        if let Some(ref mut item) = current_item {
            item.body_lines.push(normalized.to_string());
        }
    }

    finalize_item(&mut current_item, &mut items);
    items
}

#[derive(Debug)]
struct ItemHeader {
    line_start: i64,
    line_end: i64,
    title: Option<String>,
}

fn parse_item_header_line(line: &str) -> Option<ItemHeader> {
    let trimmed = line.trim();
    if !trimmed.starts_with('`') || trimmed.starts_with("```") {
        return None;
    }
    let rest = &trimmed[1..];
    let backtick_end = rest.find('`')?;
    let range = &rest[..backtick_end];
    let (line_start, line_end) = parse_line_range(range)?;
    let after = rest[backtick_end + 1..].trim();
    let title = extract_bold_title(after).or_else(|| extract_plain_title(after));
    Some(ItemHeader {
        line_start,
        line_end,
        title,
    })
}

fn parse_file_summary_line(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let start = trimmed.find("<summary>")?;
    let rest = &trimmed[start + "<summary>".len()..];
    let end = rest.find("</summary>")?;
    let summary = rest[..end].trim();
    let path = summary.split(" (").next()?.trim();
    if looks_like_path(path) {
        Some(path.to_string())
    } else {
        None
    }
}

fn looks_like_path(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }
    trimmed.contains('/') || trimmed.contains('.')
}

fn parse_line_range(value: &str) -> Option<(i64, i64)> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some((start, end)) = trimmed.split_once('-') {
        let start = start.trim().parse::<i64>().ok()?;
        let end = end.trim().parse::<i64>().ok()?;
        return Some((start, end));
    }
    let single = trimmed.parse::<i64>().ok()?;
    Some((single, single))
}

fn strip_blockquote_prefix(line: &str) -> &str {
    let trimmed = line.trim_start();
    if let Some(rest) = trimmed.strip_prefix('>') {
        rest.trim_start()
    } else {
        trimmed
    }
}

fn extract_bold_title(value: &str) -> Option<String> {
    let start = value.find("**")?;
    let rest = &value[start + 2..];
    let end = rest.find("**")?;
    let title = rest[..end].trim();
    if title.is_empty() {
        None
    } else {
        Some(title.to_string())
    }
}

fn extract_plain_title(value: &str) -> Option<String> {
    let trimmed = value.trim_start_matches(':').trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn finalize_item(current_item: &mut Option<ItemBuilder>, items: &mut Vec<ParsedSectionItem>) {
    let Some(builder) = current_item.take() else {
        return;
    };
    let Some(file_path) = builder.file_path else {
        return;
    };
    let Some(line_start) = builder.line_start else {
        return;
    };
    let line_end = builder.line_end.unwrap_or(line_start);
    let body = builder.body_lines.join("\n").trim().to_string();
    if body.is_empty() {
        return;
    }
    items.push(ParsedSectionItem {
        file_path,
        line_start,
        line_end,
        title: builder.title,
        body,
    });
}

struct ItemBuilder {
    file_path: Option<String>,
    line_start: Option<i64>,
    line_end: Option<i64>,
    title: Option<String>,
    body_lines: Vec<String>,
}

fn item_key_for_inline_comment(source: CodeRabbitItemSource, comment_id: i64) -> String {
    hash_key(&format!("comment:{}:{}", source.as_str(), comment_id))
}

fn item_key_for_section(
    kind: CodeRabbitItemKind,
    file_path: &str,
    line_start: i64,
    line_end: i64,
    title: Option<&str>,
    body: &str,
) -> String {
    let text = title.unwrap_or(body);
    let normalized = normalize_key_text(text);
    let raw = format!(
        "{}|{}|{}|{}|{}",
        kind.as_str(),
        file_path,
        line_start,
        line_end,
        normalized
    );
    hash_key(&raw)
}

fn normalize_key_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn hash_key(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
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

fn build_comments(round_id: Uuid, drafts: &[CodeRabbitCommentDraft]) -> Vec<CodeRabbitComment> {
    drafts
        .iter()
        .map(|draft| CodeRabbitComment {
            id: Uuid::new_v4(),
            round_id,
            comment_id: draft.comment_id,
            source: draft.source,
            html_url: draft.html_url.clone(),
            body: draft.body.clone(),
            created_at: draft.created_at,
            updated_at: draft.updated_at,
        })
        .collect()
}

fn build_items(round_id: Uuid, drafts: &[CodeRabbitItemDraft]) -> Vec<CodeRabbitItem> {
    drafts
        .iter()
        .map(|draft| CodeRabbitItem {
            id: Uuid::new_v4(),
            round_id,
            comment_id: draft.comment_id,
            source: draft.source,
            kind: draft.kind,
            actionable: draft.actionable,
            category: draft.category,
            severity: draft.severity,
            section: draft.section.clone(),
            file_path: draft.file_path.clone(),
            line: draft.line,
            line_start: draft.line_start,
            line_end: draft.line_end,
            original_line: draft.original_line,
            diff_hunk: draft.diff_hunk.clone(),
            html_url: draft.html_url.clone(),
            body: draft.body.clone(),
            agent_prompt: draft.agent_prompt.clone(),
            item_key: draft.item_key.clone(),
            created_at: draft.created_at,
            updated_at: draft.updated_at,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_outside_diff_and_nitpicks() {
        let body = r#"
> <details>
> <summary>Outside diff range comments (2)</summary><blockquote>
>
> <details>
> <summary>src/data/database.rs (1)</summary><blockquote>
>
> `337-343`: **Early return skips subsequent migrations.**
>
> If `fork_seeds` exists but lacks `seed_prompt_text`, this exits early.
> </blockquote></details>
> <details>
> <summary>src/ui/app.rs (1)</summary><blockquote>
>
> `8555-8572`: **Fix CI: missing coderabbit_processor.**
> </blockquote></details>
>
> </blockquote></details>

<details>
<summary>Nitpick comments (1)</summary><blockquote>
<details>
<summary>src/data/coderabbit.rs (1)</summary><blockquote>
`117-133`: **Silent fallbacks may mask database corruption.**
</blockquote></details>
</blockquote></details>
"#;

        let now = Utc::now();
        let items = extract_section_items(
            body,
            CodeRabbitItemSource::Review,
            123,
            Some("deadbeef".to_string()),
            "https://example.com",
            now,
            now,
        );

        let outside_diff: Vec<_> = items
            .iter()
            .filter(|item| item.kind == CodeRabbitItemKind::OutsideDiff)
            .collect();
        let nitpicks: Vec<_> = items
            .iter()
            .filter(|item| item.kind == CodeRabbitItemKind::Nitpick)
            .collect();

        assert_eq!(outside_diff.len(), 2);
        assert_eq!(nitpicks.len(), 1);
        assert!(outside_diff.iter().all(|item| item.actionable));
        assert!(nitpicks.iter().all(|item| !item.actionable));
        assert_eq!(outside_diff[0].line_start, Some(337));
        assert_eq!(outside_diff[0].line_end, Some(343));
        assert_eq!(nitpicks[0].line_start, Some(117));
        assert_eq!(nitpicks[0].line_end, Some(133));
    }
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
