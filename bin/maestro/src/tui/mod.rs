//! Interactive TUI dashboard for the Maestro indexer.
//!
//! Phase 3: adds a log panel fed by a custom `tracing` layer. The run loop
//! subscribes to every `EventBus` channel, folds incoming events into an
//! [`AppState`], drains a shared log ring buffer, and repaints on each tick.
//! No core, handler, or storage code knows the TUI exists — it's a fourth
//! consumer alongside `logger` and `metrics_bridge`.

mod app;
mod log_layer;

/// Central color palette. Every widget reads its colors from here so the
/// dashboard stays visually consistent and can be re-themed by touching a
/// single file.
mod theme {
    use ratatui::style::Color;

    /// Brand color used by the header badge and the backfill gauge fill.
    pub const PRIMARY: Color = Color::Cyan;
    /// Foreground used on top of `PRIMARY` backgrounds (e.g. the header badge).
    pub const PRIMARY_FG: Color = Color::Black;
    /// Healthy / connected state.
    pub const SUCCESS: Color = Color::Green;
    /// Warning state (slow path, degraded).
    pub const WARN: Color = Color::Yellow;
    /// Failure state (disconnected, errors, aborts).
    pub const ERROR: Color = Color::Red;
    /// Debug-level accent (rarely surfaced but needs its own slot).
    pub const DEBUG: Color = Color::Blue;
    /// Secondary / de-emphasized content: keys, timestamps, trace lines.
    pub const MUTED: Color = Color::DarkGray;
}

use std::io::{Stdout, stdout};
use std::panic;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers, poll, read,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use maestro_core::events::EventBus;
use maestro_core::services::IndexMode;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Gauge, Paragraph, Row, Table};
use tokio::sync::watch;
use tracing::Level;

use app::AppState;
use log_layer::LogLine;
pub use log_layer::{LogBuffer, TuiLogLayer};

const TICK_RATE: Duration = Duration::from_millis(33); // ~30 Hz

/// Run the TUI until the shutdown signal fires.
///
/// The [`TerminalGuard`] restores the previous terminal state on drop —
/// even if the future is cancelled or a panic unwinds through this task.
/// `shutdown_tx` is used to forward quit-key presses to the rest of the
/// binary (in raw mode, crossterm does not translate Ctrl+C to SIGINT).
pub async fn run(
    bus: EventBus,
    shutdown_tx: watch::Sender<bool>,
    mut shutdown_rx: watch::Receiver<bool>,
    logs: Option<LogBuffer>,
) -> Result<()> {
    let mut guard = TerminalGuard::enter().context("failed to enter TUI mode")?;
    let mut ticker = tokio::time::interval(TICK_RATE);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut indexer_rx = bus.subscribe_indexer();
    let mut backfill_rx = bus.subscribe_backfill();
    let mut handler_rx = bus.subscribe_handler();
    let mut chain_rx = bus.subscribe_chain();
    let mut cursor_rx = bus.watch_cursor();
    let mut chain_state_rx = bus.watch_chain();

    let mut state = AppState::new(logs);
    state.set_cursor(*cursor_rx.borrow());
    state.set_chain_state(chain_state_rx.borrow().clone());

    loop {
        tokio::select! {
            biased;
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
            _ = ticker.tick() => {
                if handle_input(&mut state).context("failed to read terminal input")? {
                    let _ = shutdown_tx.send(true);
                    break;
                }
                guard
                    .terminal
                    .draw(|f| draw(f, &state))
                    .context("failed to draw frame")?;
            }
            ev = indexer_rx.recv() => {
                if let Ok(ev) = ev {
                    state.on_indexer(ev);
                }
            }
            ev = backfill_rx.recv() => {
                if let Ok(ev) = ev {
                    state.on_backfill(ev);
                }
            }
            ev = handler_rx.recv() => {
                if let Ok(ev) = ev {
                    state.on_handler(ev);
                }
            }
            ev = chain_rx.recv() => {
                if let Ok(ev) = ev {
                    state.on_chain(ev);
                }
            }
            res = cursor_rx.changed() => {
                if res.is_ok() {
                    state.set_cursor(*cursor_rx.borrow());
                }
            }
            res = chain_state_rx.changed() => {
                if res.is_ok() {
                    state.set_chain_state(chain_state_rx.borrow().clone());
                }
            }
        }
    }

    Ok(())
}

/// Drain pending crossterm events without blocking. Updates `state` for log
/// filter/scroll keys and returns `true` if the user requested a quit
/// (`q`, `Esc`, or `Ctrl+C`).
fn handle_input(state: &mut AppState) -> Result<bool> {
    while poll(Duration::ZERO).context("event poll failed")? {
        if let Event::Key(key) = read().context("event read failed")? {
            if is_quit(&key) {
                return Ok(true);
            }
            apply_key(state, &key);
        }
    }
    Ok(false)
}

fn is_quit(key: &KeyEvent) -> bool {
    matches!(
        (key.code, key.modifiers),
        (KeyCode::Char('q'), KeyModifiers::NONE)
            | (KeyCode::Char('Q'), KeyModifiers::NONE)
            | (KeyCode::Esc, _)
            | (KeyCode::Char('c'), KeyModifiers::CONTROL)
    )
}

fn apply_key(state: &mut AppState, key: &KeyEvent) {
    match (key.code, key.modifiers) {
        (KeyCode::Char('f') | KeyCode::Char('F'), _) => state.cycle_log_filter(),
        (KeyCode::PageUp, _) => state.scroll_logs_up(10),
        (KeyCode::PageDown, _) => state.scroll_logs_down(10),
        (KeyCode::Up, _) => state.scroll_logs_up(1),
        (KeyCode::Down, _) => state.scroll_logs_down(1),
        (KeyCode::End, _) | (KeyCode::Char('g' | 'G'), _) => state.scroll_logs_to_tail(),
        _ => {}
    }
}

fn draw(frame: &mut ratatui::Frame<'_>, state: &AppState) {
    let has_backfill = state.backfill.is_some();
    let mut constraints: Vec<Constraint> = vec![Constraint::Length(3)]; // header
    if has_backfill {
        constraints.push(Constraint::Length(3)); // backfill gauge
    }
    constraints.push(Constraint::Length(3)); // blocks tape
    constraints.push(Constraint::Min(8)); // main body (stats + handlers + logs)
    constraints.push(Constraint::Length(1)); // footer

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(frame.area());

    let mut idx = 0usize;
    frame.render_widget(header(state), chunks[idx]);
    idx += 1;
    if has_backfill {
        frame.render_widget(backfill_gauge(state), chunks[idx]);
        idx += 1;
    }
    render_blocks_tape(frame, state, chunks[idx]);
    idx += 1;
    let body_idx = idx;
    idx += 1;
    let footer_idx = idx;

    // Main area: top half stats+handlers, bottom half logs.
    let main = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(chunks[body_idx]);

    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(main[0]);

    frame.render_widget(stats_panel(state), top[0]);
    frame.render_widget(handlers_table(state), top[1]);
    render_logs_panel(frame, state, main[1]);

    frame.render_widget(footer(), chunks[footer_idx]);
}

/// Render the animated blockchain tape: the newest N indexed blocks flow
/// from left to right with a per-cell colour that fades based on how long
/// ago the block arrived. The cell right before the cursor is the newest
/// and pulses SUCCESS-bright for the first ~1.5 s.
fn render_blocks_tape(frame: &mut ratatui::Frame<'_>, state: &AppState, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Blocks ")
        .title_style(Style::default().fg(theme::MUTED));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let tape = state.blocks_tape();
    if tape.is_empty() {
        let placeholder = Paragraph::new(Span::styled(
            " waiting for blocks… ",
            Style::default().fg(theme::MUTED),
        ));
        frame.render_widget(placeholder, inner);
        return;
    }

    let line = build_tape_line(tape.iter().copied(), inner.width as usize, Instant::now());
    frame.render_widget(Paragraph::new(line), inner);
}

/// Build the tape's Line by rendering cells right-to-left so the newest
/// block is always anchored on the right edge, then reversing. Cells that
/// don't fit are dropped.
fn build_tape_line(
    blocks: impl DoubleEndedIterator<Item = app::BlockAnim>,
    max_width: usize,
    now: Instant,
) -> Line<'static> {
    let sep = "━━";
    let mut spans_rev: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;
    let mut first = true;
    for b in blocks.rev() {
        let label = format!(" #{} ", b.number);
        let cell_len = label.chars().count();
        let needed = if first {
            cell_len
        } else {
            cell_len + sep.chars().count()
        };
        if used + needed > max_width {
            break;
        }
        if !first {
            spans_rev.push(Span::styled(sep, Style::default().fg(theme::MUTED)));
        }
        spans_rev.push(Span::styled(label, pulse_style(now, b.inserted_at, first)));
        used += needed;
        first = false;
    }
    spans_rev.reverse();
    Line::from(spans_rev)
}

/// Map the age of a block cell to a style. The newest cell (`is_newest`)
/// flashes on a SUCCESS background for ~250 ms, then dims through two
/// intermediate tiers before settling on MUTED.
fn pulse_style(now: Instant, inserted_at: Instant, is_newest: bool) -> Style {
    let age = now.saturating_duration_since(inserted_at);
    if is_newest && age < Duration::from_millis(250) {
        return Style::default()
            .fg(theme::PRIMARY_FG)
            .bg(theme::SUCCESS)
            .add_modifier(Modifier::BOLD);
    }
    if age < Duration::from_millis(700) {
        return Style::default()
            .fg(theme::SUCCESS)
            .add_modifier(Modifier::BOLD);
    }
    if age < Duration::from_millis(1_500) {
        return Style::default().fg(theme::PRIMARY);
    }
    Style::default().fg(theme::MUTED)
}

fn header(state: &AppState) -> Paragraph<'static> {
    let (status_text, status_color) = if state.chain.connected {
        ("connected", theme::SUCCESS)
    } else {
        ("disconnected", theme::ERROR)
    };
    let mode = state
        .mode
        .as_ref()
        .map(IndexMode::as_label)
        .unwrap_or("–")
        .to_string();
    let summary = format!(
        " · mode {mode} · finalized {} · spec v{} · up {}",
        state.chain.finalized_head,
        state.chain.spec_version,
        format_duration(state.uptime()),
    );

    Paragraph::new(Line::from(vec![
        Span::styled(
            " Maestro ",
            Style::default()
                .fg(theme::PRIMARY_FG)
                .bg(theme::PRIMARY)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            status_text,
            Style::default()
                .fg(status_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(summary),
    ]))
    .block(Block::default().borders(Borders::ALL))
}

fn backfill_gauge(state: &AppState) -> Gauge<'static> {
    let bf = state.backfill.expect("has_backfill checked by caller");
    let ratio = state.backfill_ratio().unwrap_or(0.0);
    let rate = state
        .backfill_rate()
        .map(|r| format!("{r:.1} b/s"))
        .unwrap_or_else(|| "–".to_string());
    let eta = state
        .backfill_eta()
        .map(format_duration)
        .unwrap_or_else(|| "–".to_string());
    let label = format!(
        "{}/{}  {:.1}%  [{}..{}]  {rate}  ETA {eta}",
        bf.persisted,
        bf.total,
        ratio * 100.0,
        bf.from,
        bf.to
    );
    Gauge::default()
        .block(Block::default().borders(Borders::ALL).title(" Backfill "))
        .gauge_style(Style::default().fg(theme::PRIMARY))
        .ratio(ratio)
        .label(label)
}

fn stats_panel(state: &AppState) -> Paragraph<'static> {
    let mode = state
        .mode
        .as_ref()
        .map(IndexMode::as_label)
        .unwrap_or("–")
        .to_string();
    let cursor = format!("head {} · tail {}", state.cursor.head, state.cursor.tail);
    let last = state
        .last_block
        .map(|b| {
            format!(
                "#{}  ({} ext · {} ev · {} ms)",
                b.number, b.extrinsics, b.events, b.duration_ms
            )
        })
        .unwrap_or_else(|| "–".to_string());
    let avg = state
        .avg_latency_ms()
        .map(|v| format!("{v:.1} ms"))
        .unwrap_or_else(|| "–".to_string());
    let stop = state.stop_reason.clone().unwrap_or_else(|| "–".to_string());

    let lines = vec![
        kv_line("Mode", mode),
        kv_line("Cursor", cursor),
        kv_line("Last block", last),
        kv_line("Avg latency", avg),
        kv_line("Stop reason", stop),
    ];

    Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" Stats "))
}

fn kv_line(key: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{key:>12}"), Style::default().fg(theme::MUTED)),
        Span::raw("  "),
        Span::raw(value),
    ])
}

fn handlers_table(state: &AppState) -> Table<'static> {
    let header = Row::new(vec![
        Cell::from("Pallet"),
        Cell::from("Events"),
        Cell::from("Persisted"),
        Cell::from("Errors"),
    ])
    .style(
        Style::default()
            .fg(theme::MUTED)
            .add_modifier(Modifier::BOLD),
    );

    let rows: Vec<Row<'static>> = if state.handlers.is_empty() {
        vec![Row::new(vec![
            Cell::from("–"),
            Cell::from("–"),
            Cell::from("–"),
            Cell::from("–"),
        ])]
    } else {
        state
            .handlers
            .iter()
            .map(|(name, stats)| {
                let errors_style = if stats.errors > 0 {
                    Style::default()
                        .fg(theme::ERROR)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                Row::new(vec![
                    Cell::from((*name).to_string()),
                    Cell::from(stats.events_processed.to_string()),
                    Cell::from(stats.persisted.to_string()),
                    Cell::from(stats.errors.to_string()).style(errors_style),
                ])
            })
            .collect()
    };

    let widths = [
        Constraint::Length(16),
        Constraint::Length(12),
        Constraint::Length(12),
        Constraint::Length(8),
    ];

    Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(" Handlers "))
}

fn footer() -> Paragraph<'static> {
    let bold = Style::default().add_modifier(Modifier::BOLD);
    Paragraph::new(Line::from(vec![
        Span::styled("q", bold),
        Span::raw(" quit · "),
        Span::styled("f", bold),
        Span::raw(" filter · "),
        Span::styled("PgUp/PgDn", bold),
        Span::raw(" scroll · "),
        Span::styled("End", bold),
        Span::raw(" follow · "),
        Span::styled("Ctrl+C", bold),
        Span::raw(" shutdown"),
    ]))
}

fn render_logs_panel(frame: &mut ratatui::Frame<'_>, state: &AppState, area: Rect) {
    let title = format!(" Logs [{}] ", state.log_filter.as_label());
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let visible = inner.height as usize;
    let lines = match state.logs.as_ref() {
        Some(buf) => buf.snapshot_filtered(state.log_filter.min_level()),
        None => Vec::new(),
    };

    let total = lines.len();
    let end = total.saturating_sub(state.log_scroll);
    let start = end.saturating_sub(visible);
    let slice = &lines[start..end];

    let rendered: Vec<Line<'static>> = slice.iter().map(format_log_line).collect();
    frame.render_widget(Paragraph::new(rendered), inner);
}

fn format_log_line(line: &LogLine) -> Line<'static> {
    let (label, style) = level_style(line.level);
    let mut spans = vec![
        Span::styled(format_log_time(line.at), Style::default().fg(theme::MUTED)),
        Span::raw(" "),
        Span::styled(label, style),
        Span::raw(" "),
        Span::raw(line.message.clone()),
    ];
    if !line.fields.is_empty() {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            line.fields.clone(),
            Style::default().fg(theme::MUTED),
        ));
    }
    Line::from(spans)
}

fn level_style(level: Level) -> (&'static str, Style) {
    match level {
        Level::ERROR => (
            "ERROR",
            Style::default()
                .fg(theme::ERROR)
                .add_modifier(Modifier::BOLD),
        ),
        Level::WARN => ("WARN ", Style::default().fg(theme::WARN)),
        Level::INFO => ("INFO ", Style::default().fg(theme::SUCCESS)),
        Level::DEBUG => ("DEBUG", Style::default().fg(theme::DEBUG)),
        Level::TRACE => ("TRACE", Style::default().fg(theme::MUTED)),
    }
}

fn format_log_time(at: SystemTime) -> String {
    let secs = at
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let h = (secs / 3600) % 24;
    let m = (secs / 60) % 60;
    let s = secs % 60;
    format!("{h:02}:{m:02}:{s:02}")
}

fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}h{m:02}m{s:02}s")
    } else if m > 0 {
        format!("{m}m{s:02}s")
    } else {
        format!("{s}s")
    }
}

/// RAII guard that owns the ratatui [`Terminal`] and restores terminal state
/// on drop. Also installs a panic hook on creation so an unwinding panic
/// leaves the terminal in a usable state before the default hook prints.
struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalGuard {
    fn enter() -> Result<Self> {
        install_panic_hook();
        enable_raw_mode().context("enable_raw_mode failed")?;
        let mut out = stdout();
        execute!(out, EnterAlternateScreen, EnableMouseCapture)
            .context("failed to enter alternate screen")?;
        let backend = CrosstermBackend::new(out);
        let terminal = Terminal::new(backend).context("failed to create ratatui terminal")?;
        Ok(Self { terminal })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        );
        let _ = self.terminal.show_cursor();
    }
}

/// Install a panic hook once that restores the terminal before delegating to
/// the previously installed hook. Without this, a panic inside the TUI task
/// would leave the terminal in raw mode and garble the user's shell.
fn install_panic_hook() {
    use std::sync::Once;
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        let prev = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            let _ = disable_raw_mode();
            let _ = execute!(stdout(), LeaveAlternateScreen, DisableMouseCapture);
            prev(info);
        }));
    });
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use maestro_core::events::{BackfillEvent, ChainEvent, HandlerEvent, IndexerEvent};
    use maestro_core::models::BlockHash;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;

    fn render(state: &AppState, width: u16, height: u16) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal.draw(|f| draw(f, state)).expect("draw");
        terminal.backend().buffer().clone()
    }

    /// Flatten the buffer into a single string so assertions can just call
    /// `.contains()`. Rows are separated by '\n'. This dodges styled-span
    /// boundaries — ratatui renders each grapheme into a single cell.
    fn flatten(buf: &Buffer) -> String {
        let mut out = String::with_capacity((buf.area.width * buf.area.height) as usize);
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn header_renders_brand_and_disconnected_status_by_default() {
        let state = AppState::new(None);
        let buf = render(&state, 120, 20);
        let text = flatten(&buf);
        assert!(text.contains("Maestro"), "missing brand\n{text}");
        assert!(text.contains("disconnected"), "missing status\n{text}");
        assert!(text.contains("spec v0"), "missing spec\n{text}");
    }

    #[test]
    fn header_reflects_connected_chain_state() {
        let mut state = AppState::new(None);
        state.on_chain(ChainEvent::RpcConnected {
            url: "ws://node".into(),
        });
        state.on_chain(ChainEvent::RuntimeUpgraded { spec_version: 42 });
        let buf = render(&state, 120, 20);
        let text = flatten(&buf);
        assert!(text.contains("connected"));
        assert!(text.contains("spec v42"));
    }

    #[test]
    fn backfill_gauge_shows_progress_and_eta() {
        let mut state = AppState::new(None);
        let t0 = Instant::now();
        state.on_backfill_at(
            BackfillEvent::Planned {
                from: 0,
                to: 99,
                total: 100,
            },
            t0,
        );
        for i in 0..10u64 {
            state.on_backfill_at(
                BackfillEvent::BlockPersisted { number: i },
                t0 + Duration::from_millis(200 * i),
            );
        }
        let buf = render(&state, 120, 24);
        let text = flatten(&buf);
        assert!(text.contains("Backfill"), "gauge title\n{text}");
        assert!(text.contains("10/100"), "progress count\n{text}");
        assert!(text.contains("10.0%"), "percent\n{text}");
        assert!(text.contains("ETA"), "eta label\n{text}");
        assert!(text.contains("b/s"), "rate label\n{text}");
    }

    #[test]
    fn layout_omits_gauge_when_no_plan() {
        let state = AppState::new(None);
        let buf = render(&state, 120, 20);
        let text = flatten(&buf);
        assert!(!text.contains("Backfill"), "gauge should be hidden\n{text}");
        assert!(text.contains("Stats"));
        assert!(text.contains("Handlers"));
    }

    #[test]
    fn stats_panel_shows_last_block_and_avg_latency() {
        let mut state = AppState::new(None);
        state.on_indexer(IndexerEvent::BlockIndexed {
            number: 123,
            hash: BlockHash([0u8; 32]),
            extrinsics: 4,
            events: 9,
            duration_ms: 42,
        });
        let buf = render(&state, 120, 24);
        let text = flatten(&buf);
        assert!(text.contains("#123"), "last block\n{text}");
        assert!(text.contains("4 ext"), "extrinsics\n{text}");
        assert!(text.contains("9 ev"), "events\n{text}");
        assert!(text.contains("42.0 ms"), "avg latency\n{text}");
    }

    #[test]
    fn handlers_table_lists_aggregated_stats() {
        let mut state = AppState::new(None);
        state.on_handler(HandlerEvent::EventProcessed {
            pallet: "Balances",
            event_name: "Transfer".into(),
            block: 1,
        });
        state.on_handler(HandlerEvent::Persisted {
            pallet: "Balances",
            table: "transfers",
            count: 7,
            block: 1,
        });
        state.on_handler(HandlerEvent::Error {
            pallet: "Balances",
            block: 1,
            error: "boom".into(),
        });
        let buf = render(&state, 120, 24);
        let text = flatten(&buf);
        assert!(text.contains("Balances"));
        assert!(text.contains("Pallet"));
        assert!(text.contains("Events"));
        assert!(text.contains("Persisted"));
        // counters: 1 processed, 7 persisted, 1 error.
        assert!(text.contains(" 1 "), "processed count missing\n{text}");
        assert!(text.contains(" 7 "), "persisted count missing\n{text}");
    }

    #[test]
    fn footer_shows_key_shortcuts() {
        let state = AppState::new(None);
        let buf = render(&state, 120, 20);
        let text = flatten(&buf);
        assert!(text.contains("quit"));
        assert!(text.contains("filter"));
        assert!(text.contains("scroll"));
    }

    #[test]
    fn logs_panel_title_reflects_filter_label() {
        let buffer = LogBuffer::default();
        let mut state = AppState::new(Some(buffer));
        state.cycle_log_filter(); // All → Info
        let buf = render(&state, 120, 30);
        let text = flatten(&buf);
        assert!(text.contains("Logs [info+]"), "filter label\n{text}");
    }

    #[test]
    fn blocks_tape_renders_recent_block_numbers_with_newest_on_the_right() {
        let mut state = AppState::new(None);
        let t0 = Instant::now();
        for i in 100u64..=104 {
            state.on_indexer_at(
                IndexerEvent::BlockIndexed {
                    number: i,
                    hash: BlockHash([0u8; 32]),
                    extrinsics: 0,
                    events: 0,
                    duration_ms: 5,
                },
                t0 + Duration::from_millis(10 * (i - 100)),
            );
        }
        let buf = render(&state, 120, 20);
        let text = flatten(&buf);
        assert!(text.contains("Blocks"), "tape title\n{text}");
        for n in 100..=104 {
            assert!(text.contains(&format!("#{n}")), "missing #{n}\n{text}");
        }
        // Newest block #104 should appear AFTER #100 (to the right) on the
        // tape row — locate both and compare indices.
        let idx_100 = text.find("#100").unwrap();
        let idx_104 = text.find("#104").unwrap();
        assert!(idx_100 < idx_104, "newest should be right of oldest");
    }

    #[test]
    fn blocks_tape_shows_placeholder_until_first_block() {
        let state = AppState::new(None);
        let buf = render(&state, 120, 20);
        let text = flatten(&buf);
        assert!(text.contains("Blocks"));
        assert!(
            text.contains("waiting for blocks"),
            "placeholder missing\n{text}"
        );
    }

    #[test]
    fn pulse_style_flashes_newest_block_and_fades_older_ones() {
        let now = Instant::now();
        let flash = pulse_style(now, now, true);
        assert_eq!(flash.bg, Some(theme::SUCCESS));

        let warm = pulse_style(now + Duration::from_millis(400), now, false);
        assert_eq!(warm.fg, Some(theme::SUCCESS));
        assert!(warm.bg.is_none());

        let cool = pulse_style(now + Duration::from_millis(1_000), now, false);
        assert_eq!(cool.fg, Some(theme::PRIMARY));

        let faded = pulse_style(now + Duration::from_secs(5), now, false);
        assert_eq!(faded.fg, Some(theme::MUTED));
    }
}
