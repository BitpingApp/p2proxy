//! Terminal UI for p2proxy. Runs in-process with the daemon; the operator
//! can pass `--no-ui` (or set `NO_UI=true`) to skip it entirely — useful
//! under systemd, Docker, or other contexts without a TTY. `Ui::run` is
//! the public entry point.

use color_eyre::eyre::Result;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use models::events::Events;
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Tabs},
};
use std::{
    io, process,
    time::{Duration, Instant},
};
use tokio::{select, sync::mpsc::Receiver, time::interval};
use tracing::{debug, info};
use ui_state::{ConnectionStatus, ProxySession, UIState};

mod logs;
mod network;
mod overview;
// Sessions tab removed — proxy sessions are ephemeral and the table was
// permanently empty in practice because most sessions either fail before
// they emit `SocksStreamMessage::Initialized` (TLS/transport problems)
// or complete fast enough that the row disappears the next render. Keep
// `sessions/mod.rs` on disk for now in case we want to reuse the table
// rendering later; just don't surface it as a tab.
// mod sessions;
mod splashscreen;
mod ui_state;

// Palette is now sourced from the shared `tui_components::theme` crate
// so bitpingd and p2proxy can't drift. The aliases below preserve the
// p2proxy-specific names (PRIMARY/SECONDARY/ACCENT/WARN/MISC) — the
// underlying values come from one place. Update `libs/tui-components/
// src/theme.rs` to change anything globally.
use tui_components::theme;

const PRIMARY: Color = theme::PURPLE;
const SECONDARY: Color = theme::SUCCESS;
const ACCENT: Color = theme::PURPLE_LIGHT;
const BORDER: Color = theme::BORDER;
const SUCCESS: Color = theme::SUCCESS;
const ERROR: Color = theme::ERROR;
const WARN: Color = theme::WARNING;
const MISC: Color = theme::INFO;
const DIM: Color = theme::DIM;
const BACKGROUND: Color = theme::BG;
const FOREGROUND: Color = Color::Reset;

pub struct Ui {
    show_splash: bool,
    splash_start_time: Instant,
    show_logs: bool,
    tab_index: usize,
    tabs: Vec<&'static str>,
    animation_start: Instant,
    needs_render: bool,
    is_animating: bool,

    state: UIState,
    /// `tui-logger` widget state — owns the scroll position for the
    /// LOGS tab. Required by `tui_components::logs::logs_widget`; the
    /// previous in-tree TuiLoggerWidget didn't have one so j/k scroll
    /// and Esc-to-follow-tail didn't work in p2proxy. Now it does.
    log_state: tui_logger::TuiWidgetState,
    /// Shared export-prompt state. Driven by the `e` key handler and
    /// rendered as an overlay modal — exactly the same UX bitpingd
    /// already shipped, via the shared `tui_components::export` module.
    export_prompt: tui_components::export::ExportPromptState,
    /// NETWORK tab: index into `CONFIG.servers` of the currently-focused
    /// row. Up/Down arrows move it; Enter toggles
    /// `network_expanded` for the focused port. The collapsed view
    /// renders each server as a one-line summary; the expanded view
    /// renders the full rotation-pool table.
    pub(crate) network_selected_idx: usize,
    /// Set of listen ports the user has expanded in the NETWORK tab.
    /// Defaults to "first server expanded, rest collapsed" so the
    /// tab isn't empty on first open.
    pub(crate) network_expanded: std::collections::HashSet<u16>,
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            show_splash: true,
            splash_start_time: Instant::now(),
            show_logs: false,
            tab_index: 0,
            tabs: vec!["OVERVIEW", "NETWORK", "LOGS"],
            animation_start: Instant::now(),
            needs_render: true,
            is_animating: true,
            state: UIState::new(),
            log_state: tui_logger::TuiWidgetState::new(),
            export_prompt: tui_components::export::ExportPromptState::new(),
            network_selected_idx: 0,
            // Pre-expand the first server so the tab isn't all
            // one-liners on first open. The first entry in
            // `CONFIG.servers` order — whatever that is — gets the
            // big-render treatment until the user toggles.
            network_expanded: {
                let mut s = std::collections::HashSet::new();
                if let Some(first) = crate::CONFIG.servers.first() {
                    s.insert(first.port);
                }
                s
            },
        }
    }
}

impl Ui {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mark_dirty(&mut self) {
        self.needs_render = true;
    }
    pub fn mark_clean(&mut self) {
        self.needs_render = false;
    }

    pub fn start_animation(&mut self) {
        self.animation_start = Instant::now();
        self.is_animating = true;
    }

    fn update_animation_state(&mut self) -> bool {
        let progress = self.animation_progress();
        if progress >= 1.0 && self.is_animating {
            self.is_animating = false;
            true // One final render needed
        } else {
            self.is_animating
        }
    }

    pub fn toggle_logs(&mut self) {
        self.show_logs = !self.show_logs;
        self.start_animation()
    }

    /// Network tab: move the focus to the next server (wrapping).
    pub fn network_select_next(&mut self) {
        let n = crate::CONFIG.servers.len();
        if n == 0 {
            return;
        }
        self.network_selected_idx = (self.network_selected_idx + 1) % n;
    }

    /// Network tab: move the focus to the previous server (wrapping).
    pub fn network_select_prev(&mut self) {
        let n = crate::CONFIG.servers.len();
        if n == 0 {
            return;
        }
        self.network_selected_idx = if self.network_selected_idx == 0 {
            n - 1
        } else {
            self.network_selected_idx - 1
        };
    }

    /// Network tab: expand-or-collapse the currently focused server.
    /// Operates on the `port` (not the index) because `network_expanded`
    /// uses ports as keys — surviving `CONFIG.servers` reordering on a
    /// future config reload without dropping the expanded set.
    pub fn network_toggle_expand_selected(&mut self) {
        let Some(server) = crate::CONFIG.servers.get(self.network_selected_idx) else {
            return;
        };
        if !self.network_expanded.remove(&server.port) {
            self.network_expanded.insert(server.port);
        }
    }

    pub fn next_tab(&mut self) {
        self.tab_index = (self.tab_index + 1) % self.tabs.len();
        // Skip the slide-in animation on tab switch — it adds an 800ms
        // ramp-up where the new tab's contents grow from zero height,
        // which feels like a hang. Just paint the destination tab in full
        // immediately.
        self.mark_dirty();
    }

    // Main function that sets up and runs the UI
    pub async fn run_ui(
        mut state_events: Receiver<Events>,
        shutdown: tokio_util::sync::CancellationToken,
    ) -> Result<()> {
        // Set up terminal
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // Create UI state
        let mut ui = Ui::new();

        // 100ms = 10fps. Drives the "tick" that repaints time-sensitive
        // widgets (bandwidth chart's seconds-ago X axis, animations,
        // splash transitions) even when no swarm event arrived. Without
        // this the chart only advances when something else nudges the
        // loop — a keystroke, a mouse-move (alt-screen mouse capture
        // events), or a state change — which is why the bandwidth line
        // looked frozen unless the user wiggled the mouse.
        let mut idle_interval = interval(Duration::from_millis(100));
        let mut event_stream = EventStream::new();

        ui.run_splash_screen_animation(&mut terminal).await;

        // Force a draw the very first time through the loop. Without this
        // the post-splash content only paints when something else nudges
        // it (idle tick after 500ms, an event, or a keystroke), which
        // makes startup feel frozen.
        terminal.draw(|f| ui.render(f))?;

        // Main loop
        loop {
            select! {
                // External shutdown (Ctrl+C in main) — tear the terminal
                // back down so the alt-screen is restored before the
                // graceful-disconnect logs print.
                _ = shutdown.cancelled() => {
                    break;
                }
                _ = idle_interval.tick(), if !ui.is_animating => {
                    ui.mark_dirty();
                },
                Some(event) = state_events.recv() => ui.handle_swarm_events(event),
                Some(Ok(event)) = event_stream.next() => {
                    if let Event::Key(key) = event {
                        // Any keystroke forces a fresh paint, so resize /
                        // focus / unrecognised keys all redraw rather than
                        // appearing to hang.
                        ui.mark_dirty();

                        // Export prompt eats every keystroke while open —
                        // matches bitpingd's mode-switched dispatch. Esc
                        // / Enter close the modal, all others edit the
                        // path field via the shared state machine.
                        //
                        // Critical: we previously did `continue` here,
                        // which skipped the `terminal.draw` call at the
                        // bottom of the loop body — so the prompt
                        // looked frozen while typing. Use a branch
                        // (if/else) instead so the redraw still runs.
                        if ui.export_prompt.is_open() {
                            use tui_components::export::PromptKeyOutcome;
                            match ui.export_prompt.handle_key(key.code) {
                                PromptKeyOutcome::Confirm(dest) => {
                                    match tui_components::logs::copy_log_file_to(&dest) {
                                        Ok(bytes) => {
                                            tracing::info!(path = %dest, bytes, "logs exported");
                                            ui.export_prompt.set_status_ok(
                                                std::path::PathBuf::from(dest),
                                                bytes,
                                            );
                                        }
                                        Err(e) => {
                                            tracing::warn!(?e, path = %dest, "log export failed");
                                            ui.export_prompt.set_status_err(e.to_string());
                                        }
                                    }
                                }
                                PromptKeyOutcome::Close
                                | PromptKeyOutcome::Redraw
                                | PromptKeyOutcome::Ignored => {}
                            }
                        } else {
                        match key.code {
                            KeyCode::Char('q') => {
                                break;
                            },
                            KeyCode::Char('e') | KeyCode::Char('E') => {
                                // Open the shared export modal. Default
                                // path is `~/p2proxy-logs-<ts>.txt` if
                                // we can resolve the user's home dir,
                                // else just the file basename in cwd.
                                let default_path = Ui::default_export_path();
                                ui.export_prompt.open(default_path);
                            }
                            KeyCode::Char('l') => ui.toggle_logs(),
                            KeyCode::Tab => ui.next_tab(),
                            KeyCode::BackTab => ui.previous_tab(),
                            KeyCode::Up => {
                                // Network tab: cycle the focused server
                                // up (wraps). Other tabs: no-op (the
                                // logs pane handles its own scrolling
                                // via TuiWidgetState).
                                if ui.tab_index == 1 {
                                    ui.network_select_prev();
                                }
                                ui.mark_dirty();
                            }
                            KeyCode::Char('j') => {
                                // vim-style down for navigation in
                                // network tab. Keeps parity with the
                                // logs pane's j/k.
                                if ui.tab_index == 1 {
                                    ui.network_select_next();
                                }
                                ui.mark_dirty();
                            }
                            KeyCode::Char('k') => {
                                if ui.tab_index == 1 {
                                    ui.network_select_prev();
                                }
                                ui.mark_dirty();
                            }
                            KeyCode::Enter | KeyCode::Char(' ') => {
                                // Toggle expansion of the focused
                                // server in NETWORK. No-op elsewhere.
                                if ui.tab_index == 1 {
                                    ui.network_toggle_expand_selected();
                                }
                                ui.mark_dirty();
                            }
                            KeyCode::Down => {
                                // Network tab: cycle focus down. Same
                                // wraparound behaviour as Up.
                                if ui.tab_index == 1 {
                                    ui.network_select_next();
                                }
                                let _ = ();
                                // (legacy session-selection branch
                                // intentionally removed — see
                                // `Up` arm for the new behaviour.)
                                // }).await;
                                ui.mark_dirty();
                            }
                            _ => {}
                        }
                        }  // close else (export-prompt is_open branch)
                    }
                }
            };

            if ui.needs_render || ui.is_animating {
                terminal.draw(|f| {
                    ui.render(f);
                })?;
            }
        }

        // Restore terminal. We return Ok here rather than process::exit so
        // main can drive the rest of the shutdown (graceful libp2p
        // disconnect, error propagation through join_set). Past behaviour
        // was process::exit(0) which slammed the runtime down before
        // drive_network could send close frames.
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;

        Ok(())
    }

    pub fn handle_swarm_events(&mut self, event: Events) {
        debug!(?event, "Got new event");
        match event {
            Events::LocalPeerId(peer_id) => {
                self.state.local_peer_id.replace(peer_id);
            }
            Events::Connection(connection_events) => match connection_events {
                models::events::ConnectionEvents::Connecting => {
                    self.state.connection_status = ConnectionStatus::Connecting;
                }
                models::events::ConnectionEvents::Connected(peer_id) => {
                    self.state.connection_status = ConnectionStatus::Connected(peer_id);
                    // The hub we're relaying through counts as a peer for
                    // dashboard purposes — without this the CONNECTED
                    // PEERS gauge stays at 0 forever even when we have a
                    // live hub link.
                    self.state.peers.insert(peer_id);
                    // Once we're actually routing, the most recent error
                    // is irrelevant noise — clear the banner.
                    self.state.last_error = None;
                }
                models::events::ConnectionEvents::Disconnected(peer_id) => {
                    // Per-peer disconnect. Previously the variant was
                    // unit (no peer id) so this branch nuked the
                    // entire `state.peers` set on a single peer
                    // dropping. Now we drop just that peer, plus
                    // remove it from every server's rotation pool —
                    // dead candidates shouldn't keep showing up as
                    // standby rows in the NETWORK tab.
                    self.state.peers.remove(&peer_id);
                    self.state.peer_bandwidth.remove(&peer_id);
                    for pool in self.state.server_pools.values_mut() {
                        pool.retain(|p| *p != peer_id);
                    }
                    // Clear any per-server "active" pointer that was
                    // still on this peer. Belt-and-braces — the swarm
                    // also emits `ActiveDestination { peer: None }`
                    // via the `PeerDisconnected` flow, but doing it
                    // here too means the TUI stays consistent if
                    // either event arrives first.
                    self.state.active_destinations.retain(|_port, p| *p != peer_id);
                    // Only flip the global status to Disconnected if
                    // every peer is gone — keeps the OVERVIEW status
                    // "Connected" while some destinations are still
                    // alive.
                    if self.state.peers.is_empty() {
                        self.state.connection_status = ConnectionStatus::Disconnected;
                    }
                }
            },
            Events::Session(session_events) => match session_events {
                models::events::SessionEvents::New(id, endpoint, peer_id) => {
                    // Destination peers count too — a session opens a
                    // proxy stream to one, so they show up in NETWORK
                    // alongside the hub.
                    self.state.peers.insert(peer_id);
                    self.state.session_peer.insert(id, peer_id);
                    self.state.sessions.insert(
                        id,
                        ProxySession {
                            id,
                            peer_id,
                            endpoint,
                        },
                    );
                }
                models::events::SessionEvents::End(uuid) => {
                    self.state.sessions.remove(&uuid);
                    self.state.session_peer.remove(&uuid);
                    // Intentionally don't remove peer_id from peers —
                    // the peer connection itself may outlive the
                    // session (e.g. stream pool keeping it warm), and
                    // we'd need a refcount to do this right. Stale
                    // peers get cleaned up on the next Disconnected.
                }
            },
            Events::Bandwidth(bandwidth_events) => match bandwidth_events {
                models::events::BandwidthEvents::Upload(session_id, u) => {
                    self.state.add_upload(u);
                    // Per-peer accumulator — only works if we still
                    // have the session→peer mapping, which we do until
                    // `SessionEvents::End` removes it. Late bandwidth
                    // events from a closed session land in the table
                    // anyway because the peer entry persists.
                    if let Some(&peer) = self.state.session_peer.get(&session_id) {
                        self.state.peer_bandwidth.entry(peer).or_default().0 += u;
                    }
                }
                models::events::BandwidthEvents::Download(session_id, d) => {
                    self.state.add_download(d);
                    if let Some(&peer) = self.state.session_peer.get(&session_id) {
                        self.state.peer_bandwidth.entry(peer).or_default().1 += d;
                    }
                }
            },
            Events::Error(message) => {
                self.state.last_error = Some(message);
            }
            Events::ServerPool { port, peers } => {
                // Replace, don't merge — the new FindNodes result is
                // the current truth for this server. Previous peers
                // that fell out of the country/bandwidth filters drop
                // out of the table automatically.
                self.state.server_pools.insert(port, peers);
            }
            Events::ActiveDestination { port, peer, source } => {
                match peer {
                    Some(p) => {
                        self.state.peers.insert(p);
                        self.state.active_destinations.insert(port, p);
                        match source {
                            Some(s) => {
                                self.state.destination_sources.insert(port, s);
                            }
                            None => {
                                self.state.destination_sources.remove(&port);
                            }
                        }
                    }
                    None => {
                        self.state.active_destinations.remove(&port);
                        self.state.destination_sources.remove(&port);
                    }
                }
            }
            Events::PinnedPeerStatuses { port, statuses } => {
                self.state.pinned_statuses.insert(port, statuses);
            }
        };

        self.mark_dirty();
    }

    pub fn previous_tab(&mut self) {
        if self.tab_index > 0 {
            self.tab_index -= 1;
        } else {
            self.tab_index = self.tabs.len().saturating_sub(1);
        }
        // See next_tab — skip the slide-in for instant tab response.
        self.mark_dirty();
    }
    // Calculate animation progress (0.0 to 1.0) for elements
    fn animation_progress(&self) -> f32 {
        let elapsed = self.animation_start.elapsed().as_millis() as f32;
        let duration = 800.0; // Animation duration in ms

        (elapsed / duration).min(1.0)
    }

    // Apply easing function to animation progress
    fn ease_out_expo(&self, progress: f32) -> f32 {
        if progress >= 1.0 {
            1.0
        } else {
            1.0 - 2.0f32.powf(-10.0 * progress)
        }
    }

    pub fn render(&mut self, frame: &mut Frame<'_>) {
        // Create the main layout
        let size = frame.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),                                   // Header with tabs
                Constraint::Min(0),                                      // Main content
                Constraint::Length(if self.show_logs { 10 } else { 3 }), // Footer/logs
            ])
            .split(size);

        // Render tabs
        self.render_tabs(frame, chunks[0]);

        // Render main content based on selected tab. Indices are tightly
        // coupled to the `tabs` Vec at construction time — keep both in
        // sync if you add/remove tabs.
        match self.tab_index {
            0 => self.render_overview_tab(frame, chunks[1]),
            1 => self.render_network_tab(frame, chunks[1]),
            2 => self.render_logs_tab(frame, chunks[1]),
            _ => {}
        }

        // Render footer or logs
        self.render_footer_or_logs(frame, chunks[2]);

        // Export success/error banner (toast-style) — overlays the
        // bottom of the main content area for ~5s after the user
        // confirms the prompt. Same TTL bitpingd uses.
        if let Some(status) = self
            .export_prompt
            .status_within_ttl(tui_components::export::EXPORT_STATUS_TTL_SECS)
        {
            let banner_h: u16 = 3;
            let banner_area = ratatui::layout::Rect::new(
                chunks[1].x,
                chunks[1]
                    .y
                    .saturating_add(chunks[1].height.saturating_sub(banner_h)),
                chunks[1].width,
                banner_h,
            );
            tui_components::export::draw_banner(frame, banner_area, status);
        }

        // Export prompt overlay last so it sits on top of everything.
        tui_components::export::draw_prompt(frame, &self.export_prompt);

        self.mark_clean();
    }

    /// Pre-fill for the export prompt. Picks `~/p2proxy-logs-<ts>.txt`
    /// when the user has a home directory (which is everyone running
    /// the CLI in TUI mode by definition); falls back to the cwd
    /// otherwise. Called fresh each time the user presses `e` so the
    /// timestamp is current.
    pub(crate) fn default_export_path() -> String {
        let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S");
        let filename = format!("p2proxy-logs-{stamp}.txt");
        match std::env::var("HOME") {
            Ok(home) if !home.is_empty() => format!("{home}/{filename}"),
            _ => filename,
        }
    }

    fn render_tabs(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let tabs = Tabs::new(self.tabs.clone())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(SECONDARY))
                    .title(" P2PROXY SYSTEM ")
                    .title_alignment(Alignment::Center)
                    .style(Style::default().bg(BACKGROUND)),
            )
            .select(self.tab_index)
            .style(Style::default().fg(FOREGROUND))
            .highlight_style(Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD));

        frame.render_widget(tabs, area);
    }
}
