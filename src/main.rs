use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use conduit::{
    agent::events::AgentEvent,
    config::save_tool_path,
    repro::bundle::{ReproBundle, ReproBundleMeta, ReproExportMode},
    repro::tape::{ReproTape, ReproTapeEntry},
    ui::terminal_guard,
    util::{self, Tool, ToolAvailability},
    App, Config,
};
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "conduit")]
#[command(about = "Multi-agent TUI for Claude Code, Codex CLI, Gemini CLI, and OpenCode")]
struct Cli {
    /// Custom data directory (default: ~/.conduit)
    #[arg(long, value_name = "PATH")]
    data_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Debug keyboard input - shows raw key events as you press them
    DebugKeys,

    /// Migrate a VSCode theme to Conduit TOML format
    MigrateTheme {
        /// Path to VSCode theme JSON file
        #[arg(value_name = "INPUT")]
        input: PathBuf,

        /// Output path (default: ~/.conduit/themes/<name>.toml)
        #[arg(short, long, value_name = "OUTPUT")]
        output: Option<PathBuf>,

        /// Extract common colors into a palette section
        #[arg(long)]
        palette: bool,
    },

    /// Start the web server
    Serve {
        /// Host address to bind to
        #[arg(long, default_value = "127.0.0.1")]
        host: String,

        /// Port to listen on
        #[arg(short, long, default_value_t = 3000)]
        port: u16,
    },

    /// Create or inspect repro bundles (deterministic snapshots)
    Repro {
        #[command(subcommand)]
        command: ReproCommands,
    },
}

#[derive(Subcommand)]
enum ReproCommands {
    /// Export a repro bundle containing the SQLite DB snapshot and an (optionally empty) tape.
    Export {
        /// Output path (.zip recommended)
        #[arg(long, value_name = "PATH")]
        out: PathBuf,

        /// Export mode (shareable performs aggressive scrubbing)
        #[arg(long, value_enum, default_value_t = ReproExportModeArg::Local)]
        mode: ReproExportModeArg,
    },

    /// Inspect a repro bundle (prints metadata)
    Inspect {
        /// Bundle path
        #[arg(value_name = "PATH")]
        bundle: PathBuf,
    },

    /// Extract a repro bundle into a Conduit data directory layout on disk.
    Extract {
        /// Bundle path
        #[arg(value_name = "PATH")]
        bundle: PathBuf,

        /// Output directory (will contain conduit.db + repro/)
        #[arg(long, value_name = "DIR")]
        out_dir: PathBuf,

        /// Overwrite existing files in the output directory
        #[arg(long)]
        overwrite: bool,
    },

    /// Run the app using a repro bundle's DB snapshot (and repro/ artifacts) as the data dir.
    Run {
        /// Bundle path
        #[arg(value_name = "PATH")]
        bundle: PathBuf,

        /// UI to run (tui or web)
        #[arg(long, value_enum, default_value_t = ReproRunUiArg::Tui)]
        ui: ReproRunUiArg,

        /// Host address to bind to (web only)
        #[arg(long, default_value = "127.0.0.1")]
        host: String,

        /// Port to listen on (web only)
        #[arg(short, long, default_value_t = 3000)]
        port: u16,

        /// Require tool availability (git + at least one agent) and prompt for paths in the TUI.
        ///
        /// By default, repro runs skip tool checks to avoid blocking reproduction.
        #[arg(long)]
        require_tools: bool,

        /// After replaying, switch back to live mode so you can continue the session with a real agent.
        #[arg(long)]
        continue_live: bool,

        /// Pause replay after emitting the given tape sequence number.
        #[arg(long)]
        pause_at: Option<u64>,

        /// Pause replay before the first occurrence of the given event type.
        #[arg(long, value_enum)]
        pause_before: Option<ReproPauseBeforeArg>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ReproExportModeArg {
    Local,
    Shareable,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ReproRunUiArg {
    Tui,
    Web,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ReproPauseBeforeArg {
    TurnCompleted,
    AssistantFinal,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::DebugKeys) => {
            run_debug_keys()?;
        }
        Some(Commands::MigrateTheme {
            input,
            output,
            palette,
        }) => {
            util::init_data_dir(cli.data_dir);
            run_migrate_theme(&input, output.as_deref(), palette)?;
        }
        Some(Commands::Serve { host, port }) => {
            util::init_data_dir(cli.data_dir);
            conduit::repro::runtime::init_from_env();
            run_web_server(host, port).await?;
        }
        Some(Commands::Repro { command }) => {
            run_repro(command, cli.data_dir).await?;
        }
        None => {
            util::init_data_dir(cli.data_dir);
            conduit::repro::runtime::init_from_env();
            let require_tools = !conduit::repro::runtime::is_replay()
                || conduit::repro::runtime::continue_live_after_replay();
            run_app(require_tools).await?;
        }
    }

    Ok(())
}

async fn run_repro(command: ReproCommands, data_dir: Option<PathBuf>) -> Result<()> {
    match command {
        ReproCommands::Export { out, mode } => {
            util::init_data_dir(data_dir);
            let db_src = util::database_path();
            if !db_src.exists() {
                anyhow::bail!("database not found at {}", db_src.display());
            }

            let temp = tempfile::tempdir()?;
            let db_snapshot = temp.path().join("db.sqlite");

            // For now, use a plain file copy (CLI is expected to run when the app isn't holding an open connection).
            // In-app export should use SQLite `VACUUM INTO` for a consistent online snapshot.
            std::fs::copy(&db_src, &db_snapshot)?;

            let meta = ReproBundleMeta {
                schema_version: 0, // set by create()
                export_mode: match mode {
                    ReproExportModeArg::Local => ReproExportMode::Local,
                    ReproExportModeArg::Shareable => ReproExportMode::Shareable,
                },
                created_at_ms: now_ms(),
                app_version: env!("CARGO_PKG_VERSION").to_string(),
                os: std::env::consts::OS.to_string(),
                git_commit: None,
            };

            let tape = {
                let tape_path = conduit::repro::runtime::tape_path();
                if tape_path.exists() {
                    ReproTape::read_jsonl_from_path(&tape_path).map_err(|e| {
                        anyhow::anyhow!(
                            "failed to read repro tape at {}: {}",
                            tape_path.display(),
                            e
                        )
                    })?
                } else {
                    ReproTape::new()
                }
            };
            ReproBundle::create(&out, meta, tape, &db_snapshot, None)?;
            println!("{}", out.display());
        }
        ReproCommands::Inspect { bundle } => {
            let opened = ReproBundle::open(&bundle)?;
            println!("{}", serde_json::to_string_pretty(&opened.meta)?);
        }
        ReproCommands::Extract {
            bundle,
            out_dir,
            overwrite,
        } => {
            ReproBundle::extract_to_data_dir(&bundle, &out_dir, overwrite)?;
            println!("{}", out_dir.display());
        }
        ReproCommands::Run {
            bundle,
            ui,
            host,
            port,
            require_tools,
            continue_live,
            pause_at,
            pause_before,
        } => {
            let prepared = ReproBundle::prepare_data_dir(&bundle)?;
            util::init_data_dir(Some(prepared.data_dir.clone()));
            let tape = ReproTape::read_jsonl_from_path(&conduit::repro::runtime::tape_path()).ok();
            let total_events = tape.as_ref().map(repro_tape_max_seq).unwrap_or_default();
            let mut pause_at_seq = pause_at;
            if pause_at_seq.is_none() {
                if let (Some(kind), Some(tape)) = (pause_before, tape.as_ref()) {
                    pause_at_seq = resolve_pause_before(tape, kind);
                }
            }
            conduit::repro::runtime::set_mode(conduit::repro::runtime::ReproMode::Replay {
                continue_live,
            });
            conduit::repro::runtime::init_replay_controller(pause_at_seq, total_events);

            match ui {
                ReproRunUiArg::Tui => {
                    let _keep_temp_dir_alive = prepared;
                    run_app(require_tools).await?;
                }
                ReproRunUiArg::Web => {
                    let _keep_temp_dir_alive = prepared;
                    run_web_server(host, port).await?;
                }
            }
        }
    }
    Ok(())
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn repro_tape_entry_seq(entry: &ReproTapeEntry) -> u64 {
    match entry {
        ReproTapeEntry::AgentEvent { seq, .. }
        | ReproTapeEntry::AgentInput { seq, .. }
        | ReproTapeEntry::Note { seq, .. } => *seq,
    }
}

fn repro_tape_max_seq(tape: &ReproTape) -> u64 {
    tape.entries
        .iter()
        .map(repro_tape_entry_seq)
        .max()
        .unwrap_or_default()
}

fn resolve_pause_before(tape: &ReproTape, kind: ReproPauseBeforeArg) -> Option<u64> {
    tape.entries.iter().find_map(|entry| match entry {
        ReproTapeEntry::AgentEvent { seq, event, .. } => match kind {
            ReproPauseBeforeArg::TurnCompleted => {
                matches!(event, AgentEvent::TurnCompleted(_)).then_some(seq.saturating_sub(1))
            }
            ReproPauseBeforeArg::AssistantFinal => match event {
                AgentEvent::AssistantMessage(msg) if msg.is_final => Some(seq.saturating_sub(1)),
                _ => None,
            },
        },
        _ => None,
    })
}

/// Run the main application
async fn run_app(require_tools: bool) -> Result<()> {
    // Install panic hook to restore terminal state before printing panic message
    terminal_guard::install_panic_hook();

    // Initialize logging to file (~/.conduit/logs/conduit.log)
    fs::create_dir_all(util::logs_dir())?;

    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(util::log_file_path())?;

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::builder()
                .with_default_directive(tracing::Level::WARN.into())
                .from_env_lossy(),
        )
        .with_writer(log_file)
        .with_ansi(false) // Disable ANSI colors in log file
        .init();

    // Create config (loads from ~/.conduit/config.toml if present)
    let config = Config::load();

    // Initialize theme from config
    conduit::ui::components::init_theme(config.theme_name.as_deref(), config.theme_path.as_deref());

    // Detect tool availability
    let mut tools = ToolAvailability::detect(&config.tool_paths);

    if require_tools {
        // Check MANDATORY requirement: git
        // Conduit exists for git worktree management, cannot function without git
        if !tools.is_available(Tool::Git) {
            match run_blocking_tool_dialog(Tool::Git, &tools)? {
                Some(path) => {
                    tools.update_tool(Tool::Git, path.clone());
                    if let Err(e) = save_tool_path(Tool::Git, &path) {
                        eprintln!("Warning: Failed to save git path to config: {}", e);
                    }
                }
                None => {
                    // User chose to quit
                    return Ok(());
                }
            }
        }

        // Check critical requirement: at least one agent
        if !tools.has_any_agent() {
            // Prefer Claude, but accept any available agent
            let preferred_agent = Tool::Claude;
            match run_blocking_tool_dialog(preferred_agent, &tools)? {
                Some(path) => {
                    // Determine which agent based on path name
                    let file_name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_ascii_lowercase())
                        .unwrap_or_default();
                    let tool = if file_name.contains("codex") {
                        Tool::Codex
                    } else if file_name.contains("gemini") {
                        Tool::Gemini
                    } else if file_name.contains("opencode") {
                        Tool::Opencode
                    } else {
                        Tool::Claude
                    };
                    tools.update_tool(tool, path.clone());
                    if let Err(e) = save_tool_path(tool, &path) {
                        eprintln!("Warning: Failed to save agent path to config: {}", e);
                    }
                }
                None => {
                    // User chose to quit
                    return Ok(());
                }
            }
        }
    } else {
        if !tools.is_available(Tool::Git) {
            tracing::info!("repro run: git not available; continuing without tool checks");
        }
        if !tools.has_any_agent() {
            tracing::info!(
                "repro run: no agent binaries available; continuing without tool checks"
            );
        }
    }

    // Create and run app with tool availability
    let mut app = App::new(config, tools);
    app.run().await
}

/// Run a blocking dialog to get a tool path from the user
///
/// This creates a minimal TUI just for the dialog, then returns control.
/// Returns Some(path) if user provided a valid path, None if user chose to quit.
fn run_blocking_tool_dialog(tool: Tool, _tools: &ToolAvailability) -> Result<Option<PathBuf>> {
    use conduit::ui::components::{MissingToolDialog, MissingToolDialogState, MissingToolResult};
    use crossterm::{
        event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
        execute,
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    };
    use ratatui::{backend::CrosstermBackend, Terminal};
    use std::io::stdout;

    // Set up terminal
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create dialog state
    let mut state = MissingToolDialogState::default();
    state.show(tool);

    let result = loop {
        // Draw
        terminal.draw(|f| {
            let dialog = MissingToolDialog::new(&state);
            f.render_widget(dialog, f.area());
        })?;

        // Handle events
        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(KeyEvent {
                code,
                modifiers,
                kind: KeyEventKind::Press,
                ..
            }) = event::read()?
            {
                match (code, modifiers) {
                    // Quit
                    (KeyCode::Char('q'), _) | (KeyCode::Esc, _) => {
                        break None;
                    }
                    // Validate and submit
                    (KeyCode::Enter, _) => {
                        if let Some(result) = state.validate() {
                            match result {
                                MissingToolResult::PathProvided(path) => {
                                    break Some(path);
                                }
                                MissingToolResult::Quit => {
                                    break None;
                                }
                                MissingToolResult::Skipped => {
                                    // This shouldn't happen for required tools
                                    break None;
                                }
                            }
                        }
                        // If validation failed, error is set in state and we continue
                    }
                    // Text input
                    (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                        state.insert_char(c);
                    }
                    (KeyCode::Backspace, _) => {
                        state.backspace();
                    }
                    (KeyCode::Delete, _) => {
                        state.delete();
                    }
                    (KeyCode::Left, _) => {
                        state.move_left();
                    }
                    (KeyCode::Right, _) => {
                        state.move_right();
                    }
                    (KeyCode::Home, _) | (KeyCode::Char('a'), KeyModifiers::CONTROL) => {
                        state.move_to_start();
                    }
                    (KeyCode::End, _) | (KeyCode::Char('e'), KeyModifiers::CONTROL) => {
                        state.move_to_end();
                    }
                    (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                        state.clear_input();
                    }
                    _ => {}
                }
            }
        }
    };

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

    Ok(result)
}

/// Run the theme migration command
fn run_migrate_theme(input: &Path, output: Option<&Path>, extract_palette: bool) -> Result<()> {
    use conduit::ui::components::theme::migrate::{
        migrate_vscode_theme, write_theme_file, MigrateOptions,
    };

    // Check input file exists
    if !input.exists() {
        anyhow::bail!("Input file not found: {}", input.display());
    }

    println!("Migrating VSCode theme: {}", input.display());

    // Configure migration options
    let options = MigrateOptions {
        extract_palette,
        verbose: false,
    };

    // Perform migration
    let result = migrate_vscode_theme(input, &options)
        .map_err(|e| anyhow::anyhow!("Migration failed: {}", e))?;

    // Determine output path
    let output_path = if let Some(path) = output {
        path.to_path_buf()
    } else {
        // Default to ~/.conduit/themes/<sanitized-name>.toml
        let themes_dir = util::data_dir().join("themes");
        let sanitized_name: String = result
            .name
            .chars()
            .map(|c: char| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '-'
                }
            })
            .collect::<String>()
            .to_lowercase();
        themes_dir.join(format!("{}.toml", sanitized_name))
    };

    // Write output file
    write_theme_file(&output_path, &result.toml)
        .map_err(|e| anyhow::anyhow!("Failed to write output: {}", e))?;

    println!("Theme migrated successfully!");
    println!("  Name: {}", result.name);
    println!("  Type: {}", if result.is_light { "light" } else { "dark" });
    println!("  Output: {}", output_path.display());
    println!();
    println!("To use this theme, add to your ~/.conduit/config.toml:");
    println!("  [theme]");
    println!(
        "  name = \"{}\"",
        output_path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
    );

    Ok(())
}

/// Run the web server
async fn run_web_server(host: String, port: u16) -> Result<()> {
    use conduit::core::ConduitCore;
    use conduit::web::{run_server, ServerConfig, WebAppState};

    // Initialize logging to stdout for web server mode
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::builder()
                .with_default_directive(tracing::Level::INFO.into())
                .from_env_lossy(),
        )
        .init();

    // Create config
    let config = Config::load();

    // Detect tool availability
    let tools = ToolAvailability::detect(&config.tool_paths);

    // Create ConduitCore
    let core = ConduitCore::new(config, tools);

    // Create web app state
    let state = WebAppState::new(core);

    // Configure server
    let server_config = ServerConfig {
        host,
        port,
        cors_permissive: true,
    };

    // Run server
    run_server(state, server_config).await?;

    Ok(())
}

/// Run the keyboard debug mode
fn run_debug_keys() -> Result<()> {
    use crossterm::{
        event::{
            self, Event, KeyCode, KeyEvent, KeyModifiers, KeyboardEnhancementFlags,
            PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
        },
        execute,
        terminal::{
            disable_raw_mode, enable_raw_mode, supports_keyboard_enhancement, EnterAlternateScreen,
            LeaveAlternateScreen,
        },
    };
    use std::io::{stdout, Write};

    // Set up terminal
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;

    // Enable Kitty keyboard protocol for proper Ctrl+Shift detection
    // REPORT_ALL_KEYS_AS_ESCAPE_CODES is required for full modifier detection
    let keyboard_enhancement_enabled =
        if supports_keyboard_enhancement().is_ok_and(|supported| supported) {
            execute!(
                stdout,
                PushKeyboardEnhancementFlags(
                    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                        | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
                        | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
                )
            )
            .is_ok()
        } else {
            false
        };

    // Clear screen and show instructions
    execute!(
        stdout,
        crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
        crossterm::cursor::MoveTo(0, 0)
    )?;

    println!("=== Conduit Key Debug Mode ===\r");
    println!("\r");
    if keyboard_enhancement_enabled {
        println!("Kitty keyboard protocol: ENABLED (Ctrl+Shift combos supported)\r");
    } else {
        println!("Kitty keyboard protocol: NOT AVAILABLE (limited modifier support)\r");
    }
    println!("\r");
    println!("Press any key combination to see how it's detected.\r");
    println!("Press Ctrl+C to exit.\r");
    println!("\r");
    println!("---\r");
    stdout.flush()?;

    loop {
        if event::poll(std::time::Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(KeyEvent {
                    code,
                    modifiers,
                    kind,
                    state,
                }) => {
                    // Format modifiers
                    let mut mod_parts = Vec::new();
                    if modifiers.contains(KeyModifiers::CONTROL) {
                        mod_parts.push("Ctrl");
                    }
                    if modifiers.contains(KeyModifiers::SHIFT) {
                        mod_parts.push("Shift");
                    }
                    if modifiers.contains(KeyModifiers::ALT) {
                        mod_parts.push("Alt");
                    }
                    if modifiers.contains(KeyModifiers::SUPER) {
                        mod_parts.push("Super");
                    }
                    if modifiers.contains(KeyModifiers::HYPER) {
                        mod_parts.push("Hyper");
                    }
                    if modifiers.contains(KeyModifiers::META) {
                        mod_parts.push("Meta");
                    }

                    let mod_str = if mod_parts.is_empty() {
                        "(none)".to_string()
                    } else {
                        mod_parts.join("+")
                    };

                    // Format key code
                    let key_str = match code {
                        KeyCode::Char(c) => format!("Char('{}')", c),
                        KeyCode::F(n) => format!("F{}", n),
                        KeyCode::Backspace => "Backspace".to_string(),
                        KeyCode::Enter => "Enter".to_string(),
                        KeyCode::Left => "Left".to_string(),
                        KeyCode::Right => "Right".to_string(),
                        KeyCode::Up => "Up".to_string(),
                        KeyCode::Down => "Down".to_string(),
                        KeyCode::Home => "Home".to_string(),
                        KeyCode::End => "End".to_string(),
                        KeyCode::PageUp => "PageUp".to_string(),
                        KeyCode::PageDown => "PageDown".to_string(),
                        KeyCode::Tab => "Tab".to_string(),
                        KeyCode::BackTab => "BackTab".to_string(),
                        KeyCode::Delete => "Delete".to_string(),
                        KeyCode::Insert => "Insert".to_string(),
                        KeyCode::Null => "Null".to_string(),
                        KeyCode::Esc => "Esc".to_string(),
                        KeyCode::CapsLock => "CapsLock".to_string(),
                        KeyCode::ScrollLock => "ScrollLock".to_string(),
                        KeyCode::NumLock => "NumLock".to_string(),
                        KeyCode::PrintScreen => "PrintScreen".to_string(),
                        KeyCode::Pause => "Pause".to_string(),
                        KeyCode::Menu => "Menu".to_string(),
                        KeyCode::KeypadBegin => "KeypadBegin".to_string(),
                        _ => format!("{:?}", code),
                    };

                    println!(
                        "Key: {:20} | Modifiers: {:20} | Kind: {:?} | State: {:?}\r",
                        key_str, mod_str, kind, state
                    );
                    stdout.flush()?;

                    // Exit on Ctrl+C
                    if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
                        break;
                    }
                }
                Event::Mouse(mouse) => {
                    println!("Mouse: {:?}\r", mouse);
                    stdout.flush()?;
                }
                Event::Resize(w, h) => {
                    println!("Resize: {}x{}\r", w, h);
                    stdout.flush()?;
                }
                Event::FocusGained => {
                    println!("Focus: Gained\r");
                    stdout.flush()?;
                }
                Event::FocusLost => {
                    println!("Focus: Lost\r");
                    stdout.flush()?;
                }
                Event::Paste(text) => {
                    println!("Paste: {:?}\r", text);
                    stdout.flush()?;
                }
            }
        }
    }

    // Restore terminal
    if keyboard_enhancement_enabled {
        if let Err(e) = execute!(stdout, PopKeyboardEnhancementFlags) {
            eprintln!(
                "Warning: Failed to restore keyboard enhancement flags: {}",
                e
            );
        }
    }
    disable_raw_mode()?;
    execute!(stdout, LeaveAlternateScreen)?;

    println!("Key debug mode exited.");

    Ok(())
}
