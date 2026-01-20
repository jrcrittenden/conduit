# Agents

Conduit orchestrates AI coding assistants called **agents**.

## Supported Agents

| Agent | Provider | Context Window |
|-------|----------|----------------|
| [Claude Code](./agents/claude-code.md) | Anthropic | 200K tokens |
| [Codex CLI](./agents/codex.md) | OpenAI | 272K tokens |
| [Gemini CLI](./agents/gemini.md) | Google | 1M tokens |

## Selecting an Agent

The default agent is configured in `~/.conduit/config.toml`:

```toml
default_agent = "claude"  # or "codex" or "gemini"
```

## Agent Detection

On startup, Conduit searches for:
- `claude` binary (Claude Code)
- `codex` binary (Codex CLI)
- `gemini` binary (Gemini CLI)

Configure custom paths in settings if needed.

## Agent Capabilities

All agents can:
- Read and write files
- Execute shell commands
- Search codebases
- Analyze code structure

See individual agent pages for specific features.
