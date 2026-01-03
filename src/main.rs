use anyhow::Result;
use clap::{Parser, Subcommand};
use conduit::{util, App, Config};
use std::fs::{self, OpenOptions};

#[derive(Parser)]
#[command(name = "conduit")]
#[command(about = "Multi-agent TUI for Claude Code and Codex CLI")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Debug keyboard input - shows raw key events as you press them
    DebugKeys,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::DebugKeys) => {
            run_debug_keys()?;
        }
        None => {
            run_app().await?;
        }
    }

    Ok(())
}

/// Run the main application
async fn run_app() -> Result<()> {
    // Initialize logging to file (~/.conduit/logs/conduit.log)
    fs::create_dir_all(util::logs_dir())?;

    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(util::log_file_path())?;

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::WARN.into()),
        )
        .with_writer(log_file)
        .with_ansi(false) // Disable ANSI colors in log file
        .init();

    // Create config (loads from ~/.conduit/config.toml if present)
    let config = Config::load();

    // Create and run app
    let mut app = App::new(config);
    app.run().await
}

/// Run the keyboard debug mode
fn run_debug_keys() -> Result<()> {
    use crossterm::{
        event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
        execute,
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    };
    use std::io::{stdout, Write};

    // Set up terminal
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;

    // Clear screen and show instructions
    execute!(
        stdout,
        crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
        crossterm::cursor::MoveTo(0, 0)
    )?;

    println!("=== Conduit Key Debug Mode ===\r");
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
    disable_raw_mode()?;
    execute!(stdout, LeaveAlternateScreen)?;

    println!("Key debug mode exited.");

    Ok(())
}
