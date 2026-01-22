#!/bin/bash
# Test: Switching workspaces in sidebar should not jump to file tab

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
source "$SCRIPT_DIR/lib.sh"

if ! command -v sqlite3 >/dev/null 2>&1; then
  echo "sqlite3 not found. Install with: brew install sqlite3" >&2
  exit 1
fi

if [ ! -f "$PROJECT_ROOT/target/release/conduit" ]; then
  echo "Building conduit release binary..."
  (cd "$PROJECT_ROOT" && cargo build --release)
fi

DATA_DIR=$(create_data_dir "tab-switch-file")
ARTIFACT_DIR="$SCRIPT_DIR/artifacts/tab-switch-file"
mkdir -p "$ARTIFACT_DIR"

cleanup_local() {
  local sock="$1"
  if [ -n "$sock" ]; then
    close_daemon "$sock"
  fi
  rm -rf "$DATA_DIR"
}

# Build data dir structure
mkdir -p "$DATA_DIR/workspaces/conduit/kind-mist"
mkdir -p "$DATA_DIR/workspaces/conduit/live-jade"

# Initialize git repos in workspace directories
git init "$DATA_DIR/workspaces/conduit/kind-mist" >/dev/null 2>&1
git init "$DATA_DIR/workspaces/conduit/live-jade" >/dev/null 2>&1

cat > "$DATA_DIR/workspaces/conduit/kind-mist/README.md" <<'EOF_FILE'
FILE TAB MARKER
EOF_FILE

# Provide a dummy codex executable to satisfy tool detection
mkdir -p "$DATA_DIR/bin"
cat > "$DATA_DIR/bin/codex" <<'EOF_BIN'
#!/bin/sh
exit 0
EOF_BIN
chmod +x "$DATA_DIR/bin/codex"

# Write a minimal config to satisfy tool checks
cat > "$DATA_DIR/config.toml" <<EOF_CONFIG
[tools]
codex = "$DATA_DIR/bin/codex"
EOF_CONFIG

# Create database schema + seed repositories/workspaces
DB_PATH="$DATA_DIR/conduit.db"
sqlite3 "$DB_PATH" <<'EOF_SQL'
CREATE TABLE IF NOT EXISTS repositories (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    base_path TEXT,
    repository_url TEXT,
    workspace_mode TEXT,
    archive_delete_branch INTEGER,
    archive_remote_prompt INTEGER,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS workspaces (
    id TEXT PRIMARY KEY,
    repository_id TEXT NOT NULL,
    name TEXT NOT NULL,
    branch TEXT NOT NULL,
    path TEXT NOT NULL,
    created_at TEXT NOT NULL,
    last_accessed TEXT NOT NULL,
    is_default INTEGER NOT NULL DEFAULT 0,
    archived_at TEXT,
    archived_commit_sha TEXT,
    FOREIGN KEY (repository_id) REFERENCES repositories(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS app_state (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS session_tabs (
    id TEXT PRIMARY KEY,
    tab_index INTEGER NOT NULL,
    is_open INTEGER NOT NULL DEFAULT 1,
    workspace_id TEXT,
    agent_type TEXT NOT NULL,
    agent_mode TEXT DEFAULT 'build',
    agent_session_id TEXT,
    model TEXT,
    pr_number INTEGER,
    created_at TEXT NOT NULL,
    pending_user_message TEXT,
    queued_messages TEXT NOT NULL DEFAULT '[]',
    input_history TEXT NOT NULL DEFAULT '[]',
    fork_seed_id TEXT,
    title TEXT,
    FOREIGN KEY (workspace_id) REFERENCES workspaces(id) ON DELETE SET NULL
);

INSERT INTO repositories (
    id, name, base_path, repository_url, workspace_mode,
    archive_delete_branch, archive_remote_prompt, created_at, updated_at
) VALUES (
    '11111111-1111-1111-1111-111111111111',
    'conduit',
    'DATA_DIR_PLACEHOLDER/workspaces/conduit',
    NULL,
    'checkout',
    0,
    0,
    datetime('now'),
    datetime('now')
);

INSERT INTO workspaces (id, repository_id, name, branch, path, created_at, last_accessed, is_default)
VALUES
  ('11111111-1111-1111-1111-111111111112','11111111-1111-1111-1111-111111111111','kind-mist','test/kind-mist','DATA_DIR_PLACEHOLDER/workspaces/conduit/kind-mist',datetime('now'),datetime('now'),0),
  ('11111111-1111-1111-1111-111111111113','11111111-1111-1111-1111-111111111111','live-jade','test/live-jade','DATA_DIR_PLACEHOLDER/workspaces/conduit/live-jade',datetime('now'),datetime('now'),0);

INSERT OR REPLACE INTO app_state(key,value,updated_at) VALUES('sidebar_visible','false',datetime('now'));
INSERT OR REPLACE INTO app_state(key,value,updated_at) VALUES('tree_collapsed_repos','',datetime('now'));
INSERT OR REPLACE INTO app_state(key,value,updated_at) VALUES('tree_selected_index','0',datetime('now'));

INSERT INTO session_tabs (
    id, tab_index, is_open, workspace_id, agent_type, agent_mode, agent_session_id,
    model, pr_number, created_at, pending_user_message, queued_messages, input_history,
    fork_seed_id, title
) VALUES
  ('22222222-2222-2222-2222-222222222221', 0, 1, '11111111-1111-1111-1111-111111111112', 'codex', 'build', NULL, NULL, NULL, datetime('now'), NULL, '[]', '[]', NULL, NULL);
EOF_SQL

# Replace placeholder path with actual data dir path
python3 - <<PY
import sqlite3
from pathlib import Path

db = Path("$DB_PATH")
data_dir = Path("$DATA_DIR")

conn = sqlite3.connect(db)
cur = conn.cursor()
cur.execute("UPDATE workspaces SET path = REPLACE(path, 'DATA_DIR_PLACEHOLDER', ?)", (str(data_dir),))
cur.execute("UPDATE repositories SET base_path = REPLACE(base_path, 'DATA_DIR_PLACEHOLDER', ?)", (str(data_dir),))
conn.commit()
conn.close()
PY

sock=""
trap 'cleanup_local "$sock"' EXIT

sock=$(start_conduit "$DATA_DIR" 200 40)
wait_idle "$sock" 500 5000 > /dev/null

assert_contains "$sock" "(kind-mist)" "Initial workspace tab visible"

# Open file viewer tab
press "$sock" ":"
type_text "$sock" "open README.md"
press "$sock" "Enter"
wait_idle "$sock" 500 5000 > /dev/null

assert_contains "$sock" "FILE TAB MARKER" "File viewer shows file content"

# Return to workspace tab
press "$sock" "Tab"
wait_idle "$sock" 300 3000 > /dev/null

# Open live-jade workspace from sidebar
ctrl "$sock" "t"
wait_idle "$sock" 300 3000 > /dev/null
press "$sock" "Down"
press "$sock" "Enter"
wait_idle "$sock" 500 5000 > /dev/null

# Switch back to kind-mist
ctrl "$sock" "t"
wait_idle "$sock" 300 3000 > /dev/null
ctrl "$sock" "t"
wait_idle "$sock" 300 3000 > /dev/null
press "$sock" "Up"
press "$sock" "Enter"
wait_idle "$sock" 500 5000 > /dev/null

# Switch to live-jade again (should not land on file tab)
ctrl "$sock" "t"
wait_idle "$sock" 300 3000 > /dev/null
ctrl "$sock" "t"
wait_idle "$sock" 300 3000 > /dev/null
press "$sock" "Down"
press "$sock" "Enter"
wait_idle "$sock" 500 5000 > /dev/null

assert_not_contains "$sock" "FILE TAB MARKER" "Sidebar switch should not jump to file tab"

log_pass "Sidebar workspace switching ignores file tab index"
