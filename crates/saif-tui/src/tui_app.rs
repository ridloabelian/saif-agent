use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::Terminal;
use saif_config::{load_skin, ProfileScope, SaifConfig, SkinConfig};
use saif_core::{AgentEvent, AgentRunner, ContextCompressor, TurnOptions};
use saif_persistence::{PersistenceStore, SqliteStore};
use saif_providers::ChatMessage;
use saif_rpc_contract::default_command_catalog;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
struct TranscriptLine {
    role: String,
    text: String,
}

struct AppState {
    skin: SkinConfig,
    model_label: String,
    session_id: Option<String>,
    transcript: Vec<TranscriptLine>,
    streaming_buffer: String,
    active_tool: Option<String>,
    input: String,
    busy: bool,
    scroll_offset: u16,
    history: Vec<ChatMessage>,
    should_quit: bool,
}

fn parse_color(name: &str) -> Color {
    match name.trim().to_ascii_lowercase().as_str() {
        "red" => Color::Red,
        "light_red" => Color::LightRed,
        "blue" => Color::Blue,
        "cyan" => Color::Cyan,
        "yellow" => Color::Yellow,
        "green" => Color::Green,
        "white" => Color::White,
        "gray" => Color::Gray,
        "dark_gray" => Color::DarkGray,
        _ => Color::Cyan,
    }
}

pub async fn run_tui(
    profile: Arc<ProfileScope>,
    cfg: Arc<SaifConfig>,
    store: Option<SqliteStore>,
    agent: AgentRunner,
    cwd: PathBuf,
) -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_tui_loop(&mut terminal, profile, cfg, store, agent, cwd).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    res
}

async fn run_tui_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    profile: Arc<ProfileScope>,
    cfg: Arc<SaifConfig>,
    store: Option<SqliteStore>,
    agent: AgentRunner,
    cwd: PathBuf,
) -> anyhow::Result<()> {
    let skin = load_skin(&profile.home_dir, &cfg.display.skin);
    let model_label = if cfg.model.is_empty() {
        "auto".to_string()
    } else {
        cfg.model.clone()
    };

    let mut state = AppState {
        skin,
        model_label,
        session_id: None,
        transcript: vec![TranscriptLine {
            role: "system".to_string(),
            text: "Saif Agent TUI ready. Type a prompt or /help for commands.".to_string(),
        }],
        streaming_buffer: String::new(),
        active_tool: None,
        input: String::new(),
        busy: false,
        scroll_offset: 0,
        history: Vec::new(),
        should_quit: false,
    };

    let (ev_tx, mut ev_rx) = mpsc::unbounded_channel::<AgentEvent>();
    let (done_tx, mut done_rx) =
        mpsc::unbounded_channel::<Result<(String, Vec<ChatMessage>), String>>();
    let mut current_cancel: Option<CancellationToken> = None;

    while !state.should_quit {
        while let Ok(ev) = ev_rx.try_recv() {
            match ev {
                AgentEvent::SessionStarted { session_id } => {
                    state.session_id = Some(session_id);
                }
                AgentEvent::ContentDelta(d) => {
                    state.streaming_buffer.push_str(&d);
                }
                AgentEvent::ThinkingDelta(_) => {}
                AgentEvent::ToolStart { name, .. } => {
                    state.active_tool = Some(format!("{} {name}", state.skin.tool_prefix));
                }
                AgentEvent::ToolComplete { name, ok, .. } => {
                    state.active_tool = None;
                    state.transcript.push(TranscriptLine {
                        role: "tool".to_string(),
                        text: format!("[{name}] {}", if ok { "ok" } else { "err" }),
                    });
                }
                AgentEvent::Compacted { pruned, summarized } => {
                    state.transcript.push(TranscriptLine {
                        role: "system".to_string(),
                        text: format!(
                            "Compacted context (pruned={pruned}, summarized={summarized})."
                        ),
                    });
                }
                AgentEvent::FinalResponse(resp) => {
                    let final_text = if state.streaming_buffer.is_empty() {
                        resp
                    } else {
                        std::mem::take(&mut state.streaming_buffer)
                    };
                    if !final_text.is_empty() {
                        state.transcript.push(TranscriptLine {
                            role: state.skin.branding.response_label.clone(),
                            text: final_text,
                        });
                    }
                }
                AgentEvent::Usage(_) => {}
            }
        }

        while let Ok(done_res) = done_rx.try_recv() {
            state.busy = false;
            state.active_tool = None;
            current_cancel = None;
            match done_res {
                Ok((sid, msgs)) => {
                    state.session_id = Some(sid);
                    state.history = msgs;
                    state.streaming_buffer.clear();
                }
                Err(err) => {
                    state.transcript.push(TranscriptLine {
                        role: "error".to_string(),
                        text: err,
                    });
                }
            }
        }

        terminal.draw(|f| draw_ui(f, &state))?;

        if event::poll(Duration::from_millis(40))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match (key.code, key.modifiers) {
                    (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                        if let Some(cancel) = current_cancel.take() {
                            cancel.cancel();
                            state.transcript.push(TranscriptLine {
                                role: "system".to_string(),
                                text: "Interrupted active turn.".to_string(),
                            });
                        } else {
                            state.should_quit = true;
                        }
                    }
                    (KeyCode::Esc, _) => {
                        if let Some(cancel) = current_cancel.take() {
                            cancel.cancel();
                        }
                    }
                    (KeyCode::PageUp, _) => {
                        state.scroll_offset = state.scroll_offset.saturating_add(5);
                    }
                    (KeyCode::PageDown, _) => {
                        state.scroll_offset = state.scroll_offset.saturating_sub(5);
                    }
                    (KeyCode::Backspace, _) => {
                        state.input.pop();
                    }
                    (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                        state.input.push(c);
                    }
                    (KeyCode::Enter, _) => {
                        let submitted = state.input.trim().to_string();
                        state.input.clear();
                        if submitted.is_empty() {
                            continue;
                        }
                        if submitted.starts_with('/') && !submitted.starts_with("/skill ") {
                            handle_slash_command(
                                &submitted,
                                &mut state,
                                &profile,
                                &cfg,
                                store.as_ref(),
                            )
                            .await;
                            continue;
                        }
                        if state.busy {
                            state.transcript.push(TranscriptLine {
                                role: "system".to_string(),
                                text: "Agent is busy; wait for current turn or press Ctrl+C to interrupt.".to_string(),
                            });
                            continue;
                        }
                        state.transcript.push(TranscriptLine {
                            role: "user".to_string(),
                            text: submitted.clone(),
                        });
                        state.busy = true;
                        let cancel = CancellationToken::new();
                        current_cancel = Some(cancel.clone());
                        let runner = agent.clone();
                        let prior = (!state.history.is_empty()).then(|| state.history.clone());
                        let sid = state.session_id.clone();
                        let opts = TurnOptions {
                            session_id: sid,
                            source: "tui".to_string(),
                            cwd: cwd.clone(),
                            model_override: (!state.model_label.is_empty()
                                && state.model_label != "auto")
                                .then(|| state.model_label.clone()),
                            ..TurnOptions::default()
                        };
                        let ev_tx_clone = ev_tx.clone();
                        let done_tx_clone = done_tx.clone();
                        tokio::spawn(async move {
                            let res = runner
                                .run_conversation(
                                    &submitted,
                                    prior,
                                    opts,
                                    Some(ev_tx_clone),
                                    cancel,
                                    None,
                                )
                                .await;
                            let mapped = match res {
                                Ok(out) => Ok((out.session_id, out.messages)),
                                Err(e) => Err(e.to_string()),
                            };
                            let _ = done_tx_clone.send(mapped);
                        });
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

async fn handle_slash_command(
    raw: &str,
    state: &mut AppState,
    profile: &ProfileScope,
    cfg: &SaifConfig,
    store: Option<&SqliteStore>,
) {
    let mut parts = raw.trim_start_matches('/').splitn(2, char::is_whitespace);
    let cmd = parts.next().unwrap_or("").to_ascii_lowercase();
    let arg = parts.next().unwrap_or("").trim();

    match cmd.as_str() {
        "quit" | "exit" | "q" => state.should_quit = true,
        "clear" => state.transcript.clear(),
        "new" | "reset" => {
            state.session_id = None;
            state.history.clear();
            state.transcript.push(TranscriptLine {
                role: "system".into(),
                text: "Started fresh session.".into(),
            });
        }
        "help" | "h" | "?" => {
            let lines: Vec<String> = default_command_catalog()
                .into_iter()
                .map(|c| format!("/{:<10} — {}", c.name, c.description))
                .collect();
            state.transcript.push(TranscriptLine {
                role: "system".into(),
                text: lines.join("\n"),
            });
        }
        "skin" => {
            let target = if arg.is_empty() { "default" } else { arg };
            state.skin = load_skin(&profile.home_dir, target);
            state.transcript.push(TranscriptLine {
                role: "system".into(),
                text: format!("Switched skin to '{}'.", state.skin.name),
            });
        }
        "model" | "m" => {
            if !arg.is_empty() {
                state.model_label = arg.to_string();
            }
            state.transcript.push(TranscriptLine {
                role: "system".into(),
                text: format!("Active model: {}", state.model_label),
            });
        }
        "status" => {
            state.transcript.push(TranscriptLine {
                role: "system".into(),
                text: format!(
                    "Profile: {} | Home: {} | Model: {} | History msgs: {}",
                    profile.profile_name,
                    profile.home_dir.display(),
                    state.model_label,
                    state.history.len()
                ),
            });
        }
        "compress" | "compact" => {
            let res = ContextCompressor::compress(
                &state.history,
                cfg.compression.keep_tail_tokens,
                Some(cfg),
                Some(&profile.secrets),
            )
            .await;
            state.history = res.messages;
            state.transcript.push(TranscriptLine {
                role: "system".into(),
                text: format!(
                    "Compressed context (pruned {}, summarized {}).",
                    res.pruned_tool_outputs, res.summarized_messages
                ),
            });
        }
        "sessions" | "ls" => {
            if let Some(db) = store {
                if let Ok(list) = db.list_sessions(10).await {
                    let body = list
                        .into_iter()
                        .map(|s| format!("{} ({} msgs) {}", s.id, s.message_count, s.preview))
                        .collect::<Vec<_>>()
                        .join("\n");
                    state.transcript.push(TranscriptLine {
                        role: "system".into(),
                        text: if body.is_empty() {
                            "No stored sessions yet.".into()
                        } else {
                            body
                        },
                    });
                }
            }
        }
        _ => {
            state.transcript.push(TranscriptLine {
                role: "system".into(),
                text: format!("Unknown slash command '/{cmd}'. Type /help."),
            });
        }
    }
}

fn draw_ui(f: &mut ratatui::Frame<'_>, state: &AppState) {
    let border_col = parse_color(&state.skin.colors.banner_border);
    let accent_col = parse_color(&state.skin.colors.banner_accent);
    let resp_col = parse_color(&state.skin.colors.response_border);

    let show_slash_menu = state.input.starts_with('/') && state.input.len() <= 16;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if show_slash_menu {
            vec![
                Constraint::Length(3),
                Constraint::Min(8),
                Constraint::Length(6),
                Constraint::Length(3),
                Constraint::Length(1),
            ]
        } else {
            vec![
                Constraint::Length(3),
                Constraint::Min(8),
                Constraint::Length(3),
                Constraint::Length(1),
            ]
        })
        .split(f.area());

    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            format!(" {} ", state.skin.branding.agent_name),
            Style::default().fg(accent_col).add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            "| model: {} | session: {}",
            state.model_label,
            state.session_id.as_deref().unwrap_or("new")
        )),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_col)),
    );
    f.render_widget(header, chunks[0]);

    let mut lines: Vec<Line<'_>> = Vec::new();
    for entry in &state.transcript {
        let style = match entry.role.as_str() {
            "user" => Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
            "system" => Style::default().fg(Color::DarkGray),
            "tool" => Style::default().fg(Color::Yellow),
            "error" => Style::default().fg(Color::Red),
            _ => Style::default().fg(resp_col).add_modifier(Modifier::BOLD),
        };
        lines.push(Line::from(Span::styled(format!("[{}]", entry.role), style)));
        for sub in entry.text.lines() {
            lines.push(Line::from(format!("  {sub}")));
        }
    }
    if !state.streaming_buffer.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("[{}] (streaming)", state.skin.branding.response_label),
            Style::default().fg(resp_col),
        )));
        for sub in state.streaming_buffer.lines() {
            lines.push(Line::from(format!("  {sub}")));
        }
    }

    let body_widget = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((state.scroll_offset, 0))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Transcript")
                .border_style(Style::default().fg(resp_col)),
        );
    f.render_widget(body_widget, chunks[1]);

    let (input_idx, status_idx) = if show_slash_menu {
        let prefix = state.input.trim_start_matches('/');
        let items: Vec<ListItem<'_>> = default_command_catalog()
            .into_iter()
            .filter(|c| c.name.starts_with(prefix))
            .take(4)
            .map(|c| ListItem::new(format!("/{} — {}", c.name, c.description)))
            .collect();
        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Slash Commands"),
        );
        f.render_widget(list, chunks[2]);
        (3, 4)
    } else {
        (2, 3)
    };

    let input_widget = Paragraph::new(format!(
        "{}{}",
        state.skin.branding.prompt_symbol, state.input
    ))
    .block(Block::default().borders(Borders::ALL).title(if state.busy {
        "Running..."
    } else {
        "Prompt"
    }));
    f.render_widget(input_widget, chunks[input_idx]);

    let status_text = if let Some(tool) = &state.active_tool {
        format!(
            "{} {} | Ctrl+C interrupt",
            state.skin.spinner.thinking_face, tool
        )
    } else if state.busy {
        format!(
            "{} Working... | Esc/Ctrl+C interrupt",
            state.skin.spinner.thinking_face
        )
    } else {
        format!(
            "{} Ready | /help | Ctrl+C quit",
            state.skin.spinner.waiting_face
        )
    };
    f.render_widget(
        Paragraph::new(status_text).style(Style::default().fg(Color::Gray)),
        chunks[status_idx],
    );
}
