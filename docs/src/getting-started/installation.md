# Installation

## Quick Install

The fastest way to install Conduit:

```bash
curl -fsSL https://getconduit.sh/install | sh
```

This script automatically detects your platform (macOS/Linux) and architecture, downloads the appropriate binary, and installs it to `~/.local/bin`.

## Prerequisites

Before using Conduit, ensure you have:

- **Git** — Required for workspace and worktree management
- **At least one AI agent:**
  - [Claude Code](https://docs.anthropic.com/en/docs/claude-code) — `npm install -g @anthropic-ai/claude-code`
  - [Codex CLI](https://github.com/openai/codex) — `npm install -g @openai/codex`
  - [Gemini CLI](https://github.com/google-gemini/gemini-cli) — `npm install -g @anthropic-ai/gemini-cli`

## Alternative Installation Methods

### Homebrew (macOS/Linux)

```bash
brew install conduit-cli/tap/conduit
```

### Cargo (from crates.io)

```bash
cargo install conduit-tui
```

### Build from Source

Clone the repository and build with Cargo:

```bash
git clone https://github.com/conduit-cli/conduit.git
cd conduit
cargo build --release
```

The binary will be at `./target/release/conduit`.

#### Add to PATH

```bash
# Copy to a directory in your PATH
cp ./target/release/conduit ~/.local/bin/

# Or create a symlink
ln -s $(pwd)/target/release/conduit ~/.local/bin/conduit
```

## Verify Installation

```bash
# Check Conduit is installed
conduit --version

# Start the TUI
conduit
```

## First Run

On first launch, Conduit will:

1. **Detect Git** — Shows an error dialog if Git is not found
2. **Detect Agents** — Searches for `claude`, `codex`, and `gemini` binaries
3. **Create Config Directory** — Creates `~/.conduit/` for settings and data

If no agents are found, you'll be prompted to configure tool paths in the settings.

## Directory Structure

Conduit stores its data in `~/.conduit/`:

```
~/.conduit/
├── config.toml      # Configuration file
├── conduit.db       # SQLite database (sessions, workspaces)
├── logs/            # Application logs
├── workspaces/      # Workspace data
└── themes/          # Custom theme files
```

## Next Steps

- [Quick Start](./quick-start.md) — Get up and running in 5 minutes
- [Configuration](../configuration/overview.md) — Customize Conduit
