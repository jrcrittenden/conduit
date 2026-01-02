# Conduit

A multi-agent Terminal User Interface (TUI) for orchestrating AI coding assistants. Run Claude Code and Codex CLI side-by-side with tab-based session management.

## Features

- **Multi-Agent Support** - Seamlessly switch between Claude Code and Codex CLI
- **Tab-Based Sessions** - Run multiple concurrent agent sessions (up to 10 tabs)
- **Real-Time Streaming** - Watch agent responses as they're generated
- **Token Usage Tracking** - Monitor input/output tokens and estimated costs
- **Session Persistence** - Resume previous sessions with their full context
- **Rich Terminal UI** - Markdown rendering, syntax highlighting, and animations

## Installation

### Prerequisites

- Rust 1.70+ (for building from source)
- At least one supported agent installed:
  - [Claude Code](https://github.com/anthropics/claude-code) (`claude` binary)
  - [Codex CLI](https://github.com/openai/codex) (`codex` binary)

### Build from Source

```bash
git clone https://github.com/fcoury/conduit.git
cd conduit
cargo build --release
```

The binary will be available at `target/release/conduit`.

## Usage

```bash
# Start with default agent (Claude Code)
conduit

# Start in a specific directory
conduit /path/to/project
```

### Keyboard Shortcuts

| Shortcut | Action |
|----------|--------|
| `Ctrl+N` | New tab (opens agent selector) |
| `Ctrl+W` | Close current tab |
| `Tab` | Switch to next tab |
| `Ctrl+1-9` | Jump to specific tab |
| `Enter` | Submit prompt |
| `Shift+Enter` | Add newline in input |
| `Up/Down` | Navigate command history |
| `Ctrl+C` | Interrupt agent |
| `Ctrl+Q` | Quit |

## Architecture

```
src/
├── main.rs              # Entry point
├── agent/               # Agent integration layer
│   ├── runner.rs        # AgentRunner trait
│   ├── events.rs        # Unified event types
│   ├── stream.rs        # JSONL stream parser
│   ├── claude.rs        # Claude Code implementation
│   └── codex.rs         # Codex CLI implementation
├── config/              # Configuration
│   └── settings.rs      # App settings and pricing
└── ui/                  # Terminal UI
    ├── app.rs           # Main event loop
    ├── tab_manager.rs   # Tab orchestration
    ├── session.rs       # Per-tab state
    └── components/      # UI components
        ├── chat_view.rs
        ├── input_box.rs
        ├── status_bar.rs
        └── ...
```

## Supported Agents

### Claude Code

Spawns the `claude` binary in headless mode with streaming JSON output:
- Real-time event streaming
- Tool execution (Read, Edit, Write, Bash, Glob, Grep)
- Session resumption

### Codex CLI

Spawns the `codex` binary with structured JSON output:
- Full automation mode
- Session persistence
- Event-based communication

## Configuration

Default settings in `src/config/settings.rs`:

| Setting | Default |
|---------|---------|
| Default agent | Claude Code |
| Max tabs | 10 |
| Show token usage | Yes |
| Show cost | Yes |

### Pricing (Claude Sonnet)

- Input tokens: $3.00 / 1M tokens
- Output tokens: $15.00 / 1M tokens

## License

MIT
