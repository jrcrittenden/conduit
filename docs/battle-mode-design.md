# Battle Mode: Design Document

## Overview

Battle Mode is a killer demo feature that pits two AI agents (Claude Code vs Codex CLI) against each other on the same prompt, racing to complete it first. This creates a visually compelling, shareable experience perfect for X/Twitter virality.

## Why This Will Go Viral

1. **Visual Drama**: Split-screen racing with real-time progress
2. **Inherent Controversy**: "Claude vs GPT" debates drive engagement
3. **Unique Differentiator**: No other tool can do head-to-head agent battles
4. **Perfect Demo Format**: 30-second clips with clear winners
5. **Shareable Results**: Auto-generated comparison cards

## User Experience

### Starting a Battle

```
/battle <prompt>
```

Or via keyboard shortcut: `Ctrl+B` to toggle battle mode, then submit prompt.

### UI Layout

```
┌─────────────────────────────────────────────────────────────────────┐
│  ⚔️  BATTLE MODE: Claude vs Codex                    ⏱️ 00:00:00    │
├─────────────────────────────┬───────────────────────────────────────┤
│  CLAUDE CODE                │  CODEX CLI                            │
│  ──────────────────────     │  ──────────────────────               │
│  Model: claude-sonnet-4     │  Model: gpt-5.2-codex                 │
│  Status: 🔄 Processing      │  Status: ✅ Complete                  │
│  Tokens: 1,234 in / 567 out │  Tokens: 1,456 in / 678 out           │
│  Cost: $0.023               │  Cost: $0.034                         │
├─────────────────────────────┼───────────────────────────────────────┤
│                             │                                       │
│  > Reading file...          │  > Creating function...               │
│  > Editing config.ts        │  > Done!                              │
│  > Running tests...         │                                       │
│                             │                                       │
│                             │                                       │
├─────────────────────────────┴───────────────────────────────────────┤
│  Prompt: "Add a login form with email validation"                   │
└─────────────────────────────────────────────────────────────────────┘
```

### Race Metrics (Real-time)

- **Timer**: Elapsed time since battle started
- **Token Counter**: Input/output tokens per agent
- **Cost Tracker**: Running cost estimate
- **Progress Indicators**: Tool calls, file changes
- **Status**: Processing/Complete/Error

### Winner Detection & Results

When both agents complete (or one errors out):

```
┌─────────────────────────────────────────────────────────────────────┐
│                        🏆 BATTLE RESULTS 🏆                          │
├─────────────────────────────┬───────────────────────────────────────┤
│  🥇 WINNER: CODEX CLI       │  🥈 CLAUDE CODE                       │
│  ──────────────────────     │  ──────────────────────               │
│  Time: 23.4s                │  Time: 31.2s (+7.8s)                  │
│  Tokens: 2,345              │  Tokens: 1,876 (-469)                 │
│  Cost: $0.045               │  Cost: $0.032 (-$0.013)               │
│  Files: 3 modified          │  Files: 2 modified                    │
│  Tools: 8 calls             │  Tools: 12 calls                      │
├─────────────────────────────┴───────────────────────────────────────┤
│  [R] Run tests on both  [C] Compare diffs  [S] Share  [Esc] Close   │
└─────────────────────────────────────────────────────────────────────┘
```

### Shareable Results Card

When user presses `S` (Share), generate a text block for X:

```
⚔️ AI BATTLE RESULTS ⚔️

Prompt: "Add login form with validation"

🥇 Codex CLI (GPT-5.2)
   ⏱️ 23.4s | 💰 $0.045 | 📁 3 files

🥈 Claude Code (Sonnet-4)
   ⏱️ 31.2s | 💰 $0.032 | 📁 2 files

Winner: Codex by 7.8s ⚡

Try it: github.com/user/conduit
#AIBattle #ClaudeVsGPT #DevTools
```

## Technical Implementation

### New Types

```rust
// src/ui/battle.rs

/// Battle mode state
pub struct BattleSession {
    /// Unique ID for this battle
    pub id: Uuid,

    /// The prompt being battled
    pub prompt: String,

    /// Left agent (Claude)
    pub left: BattleAgent,

    /// Right agent (Codex)
    pub right: BattleAgent,

    /// Battle start time
    pub started_at: Instant,

    /// Battle state
    pub state: BattleState,

    /// Working directory
    pub working_dir: PathBuf,
}

pub struct BattleAgent {
    /// Agent type
    pub agent_type: AgentType,

    /// Agent handle
    pub handle: Option<AgentHandle>,

    /// Chat view for this agent
    pub chat_view: ChatView,

    /// Processing state
    pub is_processing: bool,

    /// Token usage
    pub usage: TokenUsage,

    /// Time to completion
    pub completion_time: Option<Duration>,

    /// Error if any
    pub error: Option<String>,

    /// Files modified
    pub files_modified: Vec<String>,

    /// Tool calls count
    pub tool_calls: usize,
}

pub enum BattleState {
    /// Battle in progress
    Racing,

    /// Both agents completed
    Completed {
        winner: AgentType,
        margin: Duration,
    },

    /// One agent errored
    Error {
        failed: AgentType,
        error: String,
    },

    /// User viewing results
    Results,
}
```

### Tab Integration

```rust
// Extend TabManager or create a new enum

pub enum Tab {
    /// Normal single-agent session
    Session(AgentSession),

    /// Battle mode with two agents
    Battle(BattleSession),
}
```

### Event Handling

Battle mode needs to:
1. Spawn both agents simultaneously
2. Route events to the correct agent based on session ID
3. Detect completion/error for each agent
4. Compute winner when both complete

```rust
// In app.rs event loop

AppEvent::Agent { session_id, event } => {
    if let Some(battle) = self.active_battle_mut() {
        // Route to correct agent in battle
        if battle.left.session_id == session_id {
            battle.left.handle_event(event);
        } else if battle.right.session_id == session_id {
            battle.right.handle_event(event);
        }

        // Check if battle is complete
        if battle.is_complete() {
            battle.state = BattleState::Completed { ... };
        }
    }
}
```

### Rendering

```rust
// src/ui/components/battle_view.rs

impl Widget for BattleView {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Split area vertically
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(50),
                Constraint::Percentage(50),
            ])
            .split(area);

        // Render left agent (Claude)
        self.render_agent(&self.left, chunks[0], buf, true);

        // Render right agent (Codex)
        self.render_agent(&self.right, chunks[1], buf, false);

        // Render divider with timer
        self.render_divider(area, buf);
    }
}
```

## Actions to Add

```rust
// In action.rs

pub enum Action {
    // ... existing actions ...

    /// Toggle battle mode
    ToggleBattleMode,

    /// Start a battle with the current input
    StartBattle,

    /// View battle results
    ViewBattleResults,

    /// Share battle results to clipboard
    ShareBattleResults,

    /// Run tests on battle output
    RunBattleTests,

    /// Compare diffs between agents
    CompareBattleDiffs,
}
```

## File Structure

```
src/ui/
├── battle/
│   ├── mod.rs           # Module exports
│   ├── session.rs       # BattleSession struct
│   ├── agent.rs         # BattleAgent struct
│   ├── state.rs         # BattleState enum
│   └── results.rs       # Results generation & sharing
├── components/
│   ├── battle_view.rs   # Split-pane battle rendering
│   └── battle_header.rs # Timer and status header
└── ...
```

## Implementation Phases

### Phase 1: Core Battle Session
- [ ] Create BattleSession, BattleAgent, BattleState types
- [ ] Integrate with TabManager (Tab enum)
- [ ] Basic dual-agent spawning

### Phase 2: Split-Pane UI
- [ ] BattleView widget with horizontal split
- [ ] Per-agent status headers
- [ ] Shared timer display

### Phase 3: Event Routing
- [ ] Route events to correct agent
- [ ] Completion detection
- [ ] Error handling

### Phase 4: Results & Sharing
- [ ] Winner determination logic
- [ ] Results overlay rendering
- [ ] Shareable text generation
- [ ] Clipboard integration

### Phase 5: Polish
- [ ] Animations (racing indicators)
- [ ] Sound effects (optional)
- [ ] Keyboard shortcuts
- [ ] Help text updates

## Demo Script

For maximum X virality:

1. **Hook** (5s): "Which AI is actually better at coding? Let's find out."
2. **Setup** (5s): Show Conduit, type `/battle "Add dark mode toggle"`
3. **Race** (15s): Split screen showing both agents working
4. **Results** (5s): Winner announcement with stats
5. **CTA** (2s): "Try Conduit: [link]"

## Success Metrics

- Demo video < 30 seconds
- Clear visual winner
- Shareable result format
- Works on first try (no setup)
- Stats that spark debate (cost vs speed vs quality)
