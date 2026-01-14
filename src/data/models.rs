//! Data models for repositories and workspaces

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

use crate::agent::AgentType;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum QueuedMessageMode {
    Steer,
    FollowUp,
}

impl QueuedMessageMode {
    pub fn label(&self) -> &'static str {
        match self {
            QueuedMessageMode::Steer => "Steering",
            QueuedMessageMode::FollowUp => "Queued",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QueuedImageAttachment {
    pub path: PathBuf,
    pub placeholder: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QueuedMessage {
    pub id: Uuid,
    pub mode: QueuedMessageMode,
    pub text: String,
    pub images: Vec<QueuedImageAttachment>,
    pub created_at: DateTime<Utc>,
}

/// Represents a git repository that can have multiple workspaces
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repository {
    /// Unique identifier
    pub id: Uuid,
    /// Display name for the repository
    pub name: String,
    /// Path to the base repository (original checkout)
    pub base_path: Option<PathBuf>,
    /// Remote repository URL (for future cloning support)
    pub repository_url: Option<String>,
    /// When the repository was added
    pub created_at: DateTime<Utc>,
    /// Last time the repository was modified
    pub updated_at: DateTime<Utc>,
}

impl Repository {
    /// Create a new repository from a local path
    pub fn from_local_path(name: impl Into<String>, base_path: PathBuf) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            base_path: Some(base_path),
            repository_url: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Create a new repository from a remote URL (for future use)
    pub fn from_url(name: impl Into<String>, url: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            base_path: None,
            repository_url: Some(url.into()),
            created_at: now,
            updated_at: now,
        }
    }
}

/// Represents a workspace (git worktree) within a repository
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    /// Unique identifier
    pub id: Uuid,
    /// Parent repository ID
    pub repository_id: Uuid,
    /// Display name for the workspace
    pub name: String,
    /// Git branch this workspace is on
    pub branch: String,
    /// Path to the worktree directory
    pub path: PathBuf,
    /// When the workspace was created
    pub created_at: DateTime<Utc>,
    /// Last time the workspace was accessed
    pub last_accessed: DateTime<Utc>,
    /// Whether this is the default/main workspace
    pub is_default: bool,
    /// When the workspace was archived (None = active)
    pub archived_at: Option<DateTime<Utc>>,
    /// Commit SHA at the time of archive (if recorded)
    pub archived_commit_sha: Option<String>,
}

impl Workspace {
    /// Create a new workspace
    pub fn new(
        repository_id: Uuid,
        name: impl Into<String>,
        branch: impl Into<String>,
        path: PathBuf,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            repository_id,
            name: name.into(),
            branch: branch.into(),
            path,
            created_at: now,
            last_accessed: now,
            is_default: false,
            archived_at: None,
            archived_commit_sha: None,
        }
    }

    /// Create a default workspace (for the main branch)
    pub fn new_default(
        repository_id: Uuid,
        name: impl Into<String>,
        branch: impl Into<String>,
        path: PathBuf,
    ) -> Self {
        let mut ws = Self::new(repository_id, name, branch, path);
        ws.is_default = true;
        ws
    }

    /// Update the last accessed timestamp
    pub fn touch(&mut self) {
        self.last_accessed = Utc::now();
    }

    /// Check if this workspace is archived
    pub fn is_archived(&self) -> bool {
        self.archived_at.is_some()
    }
}

/// Represents a saved session tab for persistence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionTab {
    /// Unique identifier
    pub id: Uuid,
    /// Tab index (ordering)
    pub tab_index: i32,
    /// Associated workspace ID (optional)
    pub workspace_id: Option<Uuid>,
    /// Agent type (Claude or Codex)
    pub agent_type: AgentType,
    /// Agent mode (Build or Plan) - only applicable to Claude
    pub agent_mode: Option<String>,
    /// Agent session ID (for resume and history loading)
    pub agent_session_id: Option<String>,
    /// Selected model
    pub model: Option<String>,
    /// PR number if a PR exists for this session's branch
    pub pr_number: Option<i32>,
    /// When the tab was created
    pub created_at: DateTime<Utc>,
    /// Pending user message that hasn't been confirmed by agent yet
    pub pending_user_message: Option<String>,
    /// Queued messages waiting to be delivered
    pub queued_messages: Vec<QueuedMessage>,
    /// Input history for arrow-up restoration
    pub input_history: Vec<String>,
    /// Fork seed ID (if this tab was created via fork)
    pub fork_seed_id: Option<Uuid>,
    /// AI-generated session title/description
    pub title: Option<String>,
}

impl SessionTab {
    /// Create a new session tab
    pub fn new(
        tab_index: i32,
        agent_type: AgentType,
        workspace_id: Option<Uuid>,
        agent_session_id: Option<String>,
        model: Option<String>,
        pr_number: Option<i32>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            tab_index,
            workspace_id,
            agent_type,
            agent_mode: None,
            agent_session_id,
            model,
            pr_number,
            created_at: Utc::now(),
            pending_user_message: None,
            queued_messages: Vec::new(),
            input_history: Vec::new(),
            fork_seed_id: None,
            title: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CodeRabbitMode {
    Auto,
    Enabled,
    Disabled,
}

impl CodeRabbitMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            CodeRabbitMode::Auto => "auto",
            CodeRabbitMode::Enabled => "enabled",
            CodeRabbitMode::Disabled => "disabled",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "enabled" => CodeRabbitMode::Enabled,
            "disabled" => CodeRabbitMode::Disabled,
            _ => CodeRabbitMode::Auto,
        }
    }
}

impl std::str::FromStr for CodeRabbitMode {
    type Err = std::convert::Infallible;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(CodeRabbitMode::parse(value))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CodeRabbitRetention {
    Keep,
    DeleteOnClose,
}

impl CodeRabbitRetention {
    pub fn as_str(&self) -> &'static str {
        match self {
            CodeRabbitRetention::Keep => "keep",
            CodeRabbitRetention::DeleteOnClose => "delete-on-close",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "delete-on-close" => CodeRabbitRetention::DeleteOnClose,
            _ => CodeRabbitRetention::Keep,
        }
    }
}

impl std::str::FromStr for CodeRabbitRetention {
    type Err = std::convert::Infallible;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(CodeRabbitRetention::parse(value))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CodeRabbitFeedbackScope {
    ActionableOnly,
    All,
}

impl CodeRabbitFeedbackScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            CodeRabbitFeedbackScope::ActionableOnly => "actionable-only",
            CodeRabbitFeedbackScope::All => "all",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "actionable-only" => CodeRabbitFeedbackScope::ActionableOnly,
            _ => CodeRabbitFeedbackScope::All,
        }
    }
}

impl std::str::FromStr for CodeRabbitFeedbackScope {
    type Err = std::convert::Infallible;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(CodeRabbitFeedbackScope::parse(value))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CodeRabbitReviewLoopDoneCondition {
    ActionableZero,
    TotalZero,
    ScopeZero,
}

impl CodeRabbitReviewLoopDoneCondition {
    pub fn as_str(&self) -> &'static str {
        match self {
            CodeRabbitReviewLoopDoneCondition::ActionableZero => "actionable-zero",
            CodeRabbitReviewLoopDoneCondition::TotalZero => "total-zero",
            CodeRabbitReviewLoopDoneCondition::ScopeZero => "scope-zero",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "total-zero" => CodeRabbitReviewLoopDoneCondition::TotalZero,
            "scope-zero" => CodeRabbitReviewLoopDoneCondition::ScopeZero,
            _ => CodeRabbitReviewLoopDoneCondition::ActionableZero,
        }
    }

    pub fn is_done(
        self,
        actionable_count: i64,
        total_count: i64,
        scope: CodeRabbitFeedbackScope,
    ) -> bool {
        match self {
            CodeRabbitReviewLoopDoneCondition::ActionableZero => actionable_count == 0,
            CodeRabbitReviewLoopDoneCondition::TotalZero => total_count == 0,
            CodeRabbitReviewLoopDoneCondition::ScopeZero => match scope {
                CodeRabbitFeedbackScope::All => total_count == 0,
                CodeRabbitFeedbackScope::ActionableOnly => actionable_count == 0,
            },
        }
    }
}

impl std::str::FromStr for CodeRabbitReviewLoopDoneCondition {
    type Err = std::convert::Infallible;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(CodeRabbitReviewLoopDoneCondition::parse(value))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RepositorySettings {
    pub repository_id: Uuid,
    pub coderabbit_mode: CodeRabbitMode,
    pub coderabbit_retention: CodeRabbitRetention,
    pub coderabbit_scope: CodeRabbitFeedbackScope,
    pub coderabbit_backoff_seconds: Vec<i64>,
    pub coderabbit_review_loop_enabled: bool,
    pub coderabbit_review_loop_scope: CodeRabbitFeedbackScope,
    pub coderabbit_review_loop_done_condition: CodeRabbitReviewLoopDoneCondition,
    pub coderabbit_review_loop_ask_before_enqueue: bool,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CodeRabbitRoundStatus {
    Pending,
    Complete,
}

impl CodeRabbitRoundStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            CodeRabbitRoundStatus::Pending => "pending",
            CodeRabbitRoundStatus::Complete => "complete",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "complete" => CodeRabbitRoundStatus::Complete,
            _ => CodeRabbitRoundStatus::Pending,
        }
    }
}

impl std::str::FromStr for CodeRabbitRoundStatus {
    type Err = std::convert::Infallible;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(CodeRabbitRoundStatus::parse(value))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CodeRabbitItemSource {
    ReviewComment,
    IssueComment,
    Review,
}

impl CodeRabbitItemSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            CodeRabbitItemSource::ReviewComment => "review-comment",
            CodeRabbitItemSource::IssueComment => "issue-comment",
            CodeRabbitItemSource::Review => "review",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "issue-comment" => CodeRabbitItemSource::IssueComment,
            "review" => CodeRabbitItemSource::Review,
            _ => CodeRabbitItemSource::ReviewComment,
        }
    }
}

impl std::str::FromStr for CodeRabbitItemSource {
    type Err = std::convert::Infallible;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(CodeRabbitItemSource::parse(value))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CodeRabbitItemKind {
    Actionable,
    Nitpick,
    OutsideDiff,
}

impl CodeRabbitItemKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            CodeRabbitItemKind::Actionable => "actionable",
            CodeRabbitItemKind::Nitpick => "nitpick",
            CodeRabbitItemKind::OutsideDiff => "outside-diff",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "nitpick" => CodeRabbitItemKind::Nitpick,
            "outside-diff" => CodeRabbitItemKind::OutsideDiff,
            _ => CodeRabbitItemKind::Actionable,
        }
    }
}

impl std::str::FromStr for CodeRabbitItemKind {
    type Err = std::convert::Infallible;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(CodeRabbitItemKind::parse(value))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CodeRabbitCategory {
    PotentialIssue,
    RefactorSuggestion,
}

impl CodeRabbitCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            CodeRabbitCategory::PotentialIssue => "potential-issue",
            CodeRabbitCategory::RefactorSuggestion => "refactor-suggestion",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "potential-issue" => Some(CodeRabbitCategory::PotentialIssue),
            "refactor-suggestion" => Some(CodeRabbitCategory::RefactorSuggestion),
            _ => None,
        }
    }
}

impl std::str::FromStr for CodeRabbitCategory {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        CodeRabbitCategory::parse(value).ok_or(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CodeRabbitSeverity {
    Critical,
    Major,
    Minor,
    Trivial,
    Info,
}

impl CodeRabbitSeverity {
    pub fn as_str(&self) -> &'static str {
        match self {
            CodeRabbitSeverity::Critical => "critical",
            CodeRabbitSeverity::Major => "major",
            CodeRabbitSeverity::Minor => "minor",
            CodeRabbitSeverity::Trivial => "trivial",
            CodeRabbitSeverity::Info => "info",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "critical" => Some(CodeRabbitSeverity::Critical),
            "major" => Some(CodeRabbitSeverity::Major),
            "minor" => Some(CodeRabbitSeverity::Minor),
            "trivial" => Some(CodeRabbitSeverity::Trivial),
            "info" => Some(CodeRabbitSeverity::Info),
            _ => None,
        }
    }
}

impl std::str::FromStr for CodeRabbitSeverity {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        CodeRabbitSeverity::parse(value).ok_or(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeRabbitRound {
    pub id: Uuid,
    pub repository_id: Uuid,
    pub workspace_id: Option<Uuid>,
    pub pr_number: i64,
    pub head_sha: String,
    pub check_state: String,
    pub check_started_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub status: CodeRabbitRoundStatus,
    pub attempt_count: i64,
    pub next_fetch_at: Option<DateTime<Utc>>,
    pub actionable_count: i64,
    pub total_count: i64,
    pub completed_at: Option<DateTime<Utc>>,
    pub notified_at: Option<DateTime<Utc>>,
    pub processed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl CodeRabbitRound {
    pub fn new(
        repository_id: Uuid,
        workspace_id: Option<Uuid>,
        pr_number: i64,
        head_sha: String,
        check_state: String,
        check_started_at: DateTime<Utc>,
        observed_at: DateTime<Utc>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            repository_id,
            workspace_id,
            pr_number,
            head_sha,
            check_state,
            check_started_at,
            observed_at,
            status: CodeRabbitRoundStatus::Pending,
            attempt_count: 0,
            next_fetch_at: Some(observed_at),
            actionable_count: 0,
            total_count: 0,
            completed_at: None,
            notified_at: None,
            processed_at: None,
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeRabbitComment {
    pub id: Uuid,
    pub round_id: Uuid,
    pub comment_id: i64,
    pub source: CodeRabbitItemSource,
    pub html_url: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeRabbitItem {
    pub id: Uuid,
    pub round_id: Uuid,
    pub comment_id: i64,
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

/// Metadata for a forked session seed prompt
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForkSeed {
    /// Unique identifier
    pub id: Uuid,
    /// Agent type (Claude or Codex)
    pub agent_type: AgentType,
    /// Parent agent session ID
    pub parent_session_id: Option<String>,
    /// Parent workspace ID
    pub parent_workspace_id: Option<Uuid>,
    /// When the fork seed was created
    pub created_at: DateTime<Utc>,
    /// Hash of the seed prompt (no raw transcript stored)
    pub seed_prompt_hash: String,
    /// Optional path or pointer to a stored seed prompt (if configured)
    pub seed_prompt_path: Option<String>,
    /// Estimated tokens for the seed prompt
    pub token_estimate: i64,
    /// Context window size for the model at fork time
    pub context_window: i64,
    /// Whether the first assistant reply should be suppressed
    pub seed_ack_filtered: bool,
}

impl ForkSeed {
    pub fn new(
        agent_type: AgentType,
        parent_session_id: Option<String>,
        parent_workspace_id: Option<Uuid>,
        seed_prompt_hash: String,
        seed_prompt_path: Option<String>,
        token_estimate: i64,
        context_window: i64,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            agent_type,
            parent_session_id,
            parent_workspace_id,
            created_at: Utc::now(),
            seed_prompt_hash,
            seed_prompt_path,
            token_estimate,
            context_window,
            seed_ack_filtered: true,
        }
    }
}
