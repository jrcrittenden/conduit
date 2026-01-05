Architecture Redesign Plan
==========================

Context
-------
The current UI loop mixes input handling, state mutation, rendering, data access,
and background orchestration in `src/ui/app.rs`. The goal is to preserve existing
keybindings and UX flows while simplifying event handling and component reuse.

Current Event Flow (Simplified)
-------------------------------
Input/Timers:
  crossterm events -> handle_key_event/handle_mouse_event -> execute_action
  per-frame tick -> animation updates

Agent + background:
  AgentRunner -> AppEvent::Agent -> handle_agent_event
  PR preflight -> AppEvent::PrPreflightCompleted -> handle_pr_preflight_result

State is mutated across:
  handle_key_event, execute_action, handle_mouse_event, handle_app_event,
  and many helper methods in App.

Pain Points
-----------
- Two parallel event systems: direct input handling and AppEvent channel.
- `App` owns everything (UI state, data access, git/worktree, agent orchestration).
- Blocking IO happens inside the UI loop (git/db/worktree, filesystem, gh).
- Component interaction logic lives in `App` rather than components.
- InputMode/ViewMode behave like a state machine but transitions are scattered.

Redesign Goals
--------------
- Preserve keybindings and behaviors 1:1.
- Unify all inputs and async results as `AppEvent`s.
- Centralize state mutation in a reducer.
- Run git/db/worktree and filesystem operations via spawn_blocking.
- Move component hit-testing and interaction logic into components.

Target Architecture
-------------------
App
  - event loop (draw + dispatch)
  - dispatch(event) -> reducer -> effects
  - effect executor -> AppEvent results

AppState (pure data)
  - tab/session state, UI modes, dialog state, layout rects, metrics

AppEvent (all inputs/results)
  - Input(crossterm::Event)
  - Tick
  - Agent events
  - Background results (sidebar load, workspace ops, PR preflight, etc.)

Action (user intent)
  - keybinding-derived actions (existing enum preserved)

Effect (side effects)
  - load/save DB
  - git/worktree ops
  - spawn agent
  - PR preflight/open
  - debug export

Component Interaction
---------------------
Each component owns its interaction logic:
  - render(...)
  - handle_action(...)
  - handle_click(...)
App orchestrates focus and routes actions/clicks to the active component.

Migration Steps
---------------
1) Introduce AppState and move state fields out of App.
2) Add Effect enum + dispatch pipeline and funnel all events through it.
3) Extract background operations into spawn_blocking effects.
4) Move hit-testing and per-component logic into components.
5) Split AgentSession into SessionState + SessionView (domain vs UI widgets).
6) Add reducer tests for keybinding transitions and critical flows.
