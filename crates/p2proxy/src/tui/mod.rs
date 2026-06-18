//! Terminal UI for p2proxy. Runs in-process with the daemon; the operator
//! can pass `--no-ui` (or set `NO_UI=true`) to skip it entirely — useful
//! under systemd, Docker, or other contexts without a TTY. `Ui::run` is
//! the public entry point.

use color_eyre::eyre::Result;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, MouseEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures::StreamExt;
use proxy_core::config::Config;
use proxy_core::events::Events;
use std::sync::Arc;
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph, Tabs},
};
use std::{
    io,
    time::{Duration, Instant},
};
use tokio::{select, sync::mpsc::Receiver, time::interval};
use tracing::debug;
use ui_state::{AddrSource, ConnectionStatus, UIState};

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
const BACKGROUND: Color = theme::BG;
const FOREGROUND: Color = Color::Reset;

pub struct Ui {
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
    config: Arc<Config>,
}

impl Ui {
    pub fn new(config: Arc<Config>) -> Self {
        let mut network_expanded = std::collections::HashSet::new();
        if let Some(first) = config.servers.first() {
            network_expanded.insert(first.port);
        }
        Self {
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
            network_expanded,
            config,
        }
    }

    pub fn mark_dirty(&mut self) {
        self.needs_render = true;
    }
    pub fn mark_clean(&mut self) {
        self.needs_render = false;
    }

    pub fn toggle_logs(&mut self) {
        self.show_logs = !self.show_logs;
        self.mark_dirty();
    }

    /// The logs are scrollable when the LOGS tab is selected or the logs
    /// panel is toggled on — both render the same `log_state`.
    fn logs_focused(&self) -> bool {
        self.tabs.get(self.tab_index) == Some(&"LOGS") || self.show_logs
    }

    /// Drive the log viewport's scroll state. Returns `true` when `code` was a
    /// scroll key (so the caller skips the normal tab/network bindings). j/k
    /// and arrows page through history; Esc resumes following the tail.
    fn scroll_logs(&mut self, code: KeyCode) -> bool {
        use tui_logger::TuiWidgetEvent;
        let event = match code {
            KeyCode::Char('j') | KeyCode::Down | KeyCode::PageDown => TuiWidgetEvent::NextPageKey,
            KeyCode::Char('k') | KeyCode::Up | KeyCode::PageUp => TuiWidgetEvent::PrevPageKey,
            KeyCode::Esc => TuiWidgetEvent::EscapeKey,
            _ => return false,
        };
        self.log_state.transition(event);
        true
    }

    /// Network tab: move the focus to the next server (wrapping).
    pub fn network_select_next(&mut self) {
        let n = self.config.servers.len();
        if n == 0 {
            return;
        }
        self.network_selected_idx = (self.network_selected_idx + 1) % n;
    }

    /// Network tab: move the focus to the previous server (wrapping).
    pub fn network_select_prev(&mut self) {
        let n = self.config.servers.len();
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
        let Some(server) = self.config.servers.get(self.network_selected_idx) else {
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
        config: Arc<Config>,
    ) -> Result<()> {
        // Set up terminal
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // Create UI state
        let mut ui = Ui::new(config);

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
                // 10fps repaint heartbeat — drives animations and the
                // bandwidth chart's seconds-ago axis even when no swarm event
                // or keystroke arrives. Always on: gating it on `is_animating`
                // froze every animation (the flag is set at startup and never
                // cleared, so the tick never fired and a started animation
                // hung on its first frame until the next keypress).
                _ = idle_interval.tick() => {
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
                        } else if ui.logs_focused() && ui.scroll_logs(key.code) {
                            // Scroll key consumed by the log viewport.
                        } else {
                        match key.code {
                            KeyCode::Char('q') => {
                                break;
                            },
                            KeyCode::Char('e') | KeyCode::Char('E') => {
                                // Open the shared export modal, pre-filled with
                                // a timestamped filename in the current dir.
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
                    } else if let Event::Mouse(mouse) = event {
                        // Wheel scrolls the log viewport when it's focused —
                        // up pages back, down pages forward. Other mouse
                        // events are ignored.
                        if ui.logs_focused() {
                            let scroll = match mouse.kind {
                                MouseEventKind::ScrollUp => {
                                    Some(tui_logger::TuiWidgetEvent::PrevPageKey)
                                }
                                MouseEventKind::ScrollDown => {
                                    Some(tui_logger::TuiWidgetEvent::NextPageKey)
                                }
                                _ => None,
                            };
                            if let Some(scroll) = scroll {
                                ui.log_state.transition(scroll);
                                ui.mark_dirty();
                            }
                        }
                    }
                }
            };

            if ui.needs_render || ui.is_animating {
                terminal.draw(|f| {
                    ui.render(f);
                })?;
            }
        }

        // Show a shutdown frame before tearing the terminal down so quitting
        // reads as deliberate, not a freeze, while main drives the rest of the
        // cleanup (graceful libp2p disconnect) and prints its progress.
        let _ = terminal.draw(Self::render_shutting_down);
        tokio::time::sleep(Duration::from_millis(400)).await;

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
                proxy_core::events::ConnectionEvents::Connecting => {
                    self.state.connection_status = ConnectionStatus::Connecting;
                }
                proxy_core::events::ConnectionEvents::Connected(peer_id) => {
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
                proxy_core::events::ConnectionEvents::Disconnected(peer_id) => {
                    // Per-peer disconnect. Previously the variant was
                    // unit (no peer id) so this branch nuked the
                    // entire `state.peers` set on a single peer
                    // dropping. Now we drop just that peer, plus
                    // remove it from every server's rotation pool —
                    // dead candidates shouldn't keep showing up as
                    // standby rows in the NETWORK tab.
                    self.state.peers.remove(&peer_id);
                    self.state.peer_bandwidth.remove(&peer_id);
                    // Drop the learned route too — a remembered direct IP
                    // would otherwise outrank a fresh relayed route on the
                    // next reconnect and render as a stale "live" egress.
                    self.state.peer_addresses.remove(&peer_id);
                    for pool in self.state.server_pools.values_mut() {
                        pool.retain(|p| *p != peer_id);
                    }
                    // Clear any per-server "active" pointer that was
                    // still on this peer. Belt-and-braces — the swarm
                    // also emits `ActiveDestination { peer: None }`
                    // via the `PeerDisconnected` flow, but doing it
                    // here too means the TUI stays consistent if
                    // either event arrives first.
                    self.state
                        .active_destinations
                        .retain(|_port, p| *p != peer_id);
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
                proxy_core::events::SessionEvents::New(id, _target, peer_id) => {
                    // A session opens a proxy stream to its peer, so the peer
                    // shows up in NETWORK alongside the hub.
                    self.state.peers.insert(peer_id);
                    self.state.session_peer.insert(id, peer_id);
                    self.state.sessions.insert(id);
                }
                proxy_core::events::SessionEvents::End(uuid) => {
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
                proxy_core::events::BandwidthEvents::Upload(session_id, u) => {
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
                proxy_core::events::BandwidthEvents::Download(session_id, d) => {
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
                // Record each candidate's hub-resolved route, preferring a
                // direct address over a relay circuit when the hub offered
                // both. note_peer_address keeps the active peer's live
                // direct route from being clobbered by this candidate.
                for pp in &peers {
                    let candidate = pp
                        .addresses
                        .iter()
                        .find(|a| AddrSource::classify_candidate(a) == AddrSource::Direct)
                        .or_else(|| pp.addresses.first());
                    if let Some(addr) = candidate {
                        self.state.note_peer_address(
                            pp.peer_id,
                            addr.clone(),
                            AddrSource::Candidate,
                        );
                    }
                }
                // Replace, don't merge — the new FindNodes result is
                // the current truth for this server. Previous peers
                // that fell out of the country/bandwidth filters drop
                // out of the table automatically.
                let mut ids: Vec<_> = peers.into_iter().map(|pp| pp.peer_id).collect();
                // Keep the active peer visible even if it's a sticky/JIT
                // exit the latest FindNodes set doesn't include.
                if let Some(active) = self.state.active_destinations.get(&port) {
                    if !ids.contains(active) {
                        ids.push(*active);
                    }
                }
                self.state.server_pools.insert(port, ids);
            }
            Events::PeerRoute {
                peer_id,
                address,
                relayed,
            } => {
                let source = if relayed {
                    AddrSource::Relayed
                } else {
                    AddrSource::Direct
                };
                self.state.note_peer_address(peer_id, address, source);
            }
            Events::ActiveDestination { port, peer, source } => {
                match peer {
                    Some(p) => {
                        self.state.peers.insert(p);
                        self.state.active_destinations.insert(port, p);
                        // A sticky-reuse reconnect adopts a peer without
                        // running discovery, so no ServerPool event places
                        // it in the rotation pool. Insert it ourselves so
                        // the active row is always visible (and obviously
                        // matches sticky_peers.json).
                        let pool = self.state.server_pools.entry(port).or_default();
                        if !pool.contains(&p) {
                            pool.push(p);
                        }
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
            Events::StickyPool { port, peers } => {
                // The remembered standby pool, so the NETWORK tab shows every
                // exit in sticky_peers.json — not just the active one.
                for pp in &peers {
                    if let Some(addr) = pp.addresses.first() {
                        self.state
                            .note_peer_address(pp.peer_id, addr.clone(), AddrSource::Candidate);
                    }
                }
                self.state
                    .sticky_pools
                    .insert(port, peers.into_iter().map(|pp| pp.peer_id).collect());
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

    /// Pre-fill for the export prompt: `p2proxy-logs-<ts>.txt` in the current
    /// working directory. Called fresh each time the user presses `e` so the
    /// timestamp is current; the user can edit the path before confirming.
    pub(crate) fn default_export_path() -> String {
        let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S");
        format!("p2proxy-logs-{stamp}.txt")
    }

    /// Full-screen "shutting down" frame, shown briefly on quit before the
    /// terminal is restored and `main` prints the rest of the cleanup.
    fn render_shutting_down(frame: &mut Frame<'_>) {
        let area = frame.area();
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(SECONDARY))
            .title(" P2PROXY ")
            .title_alignment(Alignment::Center)
            .style(Style::default().bg(BACKGROUND));
        let body = Paragraph::new("\n\nShutting down…\n\ndisconnecting peers and closing listeners")
            .alignment(Alignment::Center)
            .style(Style::default().fg(FOREGROUND))
            .block(block);
        frame.render_widget(body, area);
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

#[cfg(test)]
mod tests {
    use super::*;
    use proxy_core::events::{BandwidthEvents, ConnectionEvents, Events};
    use proxy_core::testing::builders::peer;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn test_ui() -> Ui {
        Ui::new(crate::runtime::testutil::test_config())
    }

    fn rendered(ui: &mut Ui) -> String {
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).expect("terminal");
        terminal.draw(|f| ui.render(f)).expect("draw");
        let buffer = terminal.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..40u16 {
            for x in 0..120u16 {
                if let Some(cell) = buffer.cell((x, y)) {
                    out.push_str(cell.symbol());
                }
            }
        }
        out
    }

    #[test]
    fn reducer_tracks_peers_status_and_errors() {
        let mut ui = test_ui();
        let p = peer();
        ui.handle_swarm_events(Events::Connection(ConnectionEvents::Connected(p)));
        assert!(ui.state.peers.contains(&p));
        assert!(matches!(
            ui.state.connection_status,
            ConnectionStatus::Connected(_)
        ));

        ui.handle_swarm_events(Events::Error("no peers match filter".into()));
        assert_eq!(ui.state.last_error.as_deref(), Some("no peers match filter"));

        ui.handle_swarm_events(Events::Connection(ConnectionEvents::Disconnected(p)));
        assert!(!ui.state.peers.contains(&p));
    }

    #[test]
    fn bandwidth_events_accumulate_totals() {
        let mut ui = test_ui();
        let id = uuid::Uuid::new_v4();
        ui.handle_swarm_events(Events::Bandwidth(BandwidthEvents::Download(id, 4096)));
        ui.handle_swarm_events(Events::Bandwidth(BandwidthEvents::Upload(id, 1024)));
        assert_eq!(ui.state.total_download, 4096);
        assert_eq!(ui.state.total_upload, 1024);
    }

    #[test]
    fn renders_overview_and_network_tabs() {
        let mut ui = test_ui();
        let overview = rendered(&mut ui);
        assert!(overview.contains("OVERVIEW"), "overview tab header present");

        ui.next_tab();
        let network = rendered(&mut ui);
        assert!(
            network.contains("1080"),
            "network tab shows the configured server port"
        );
    }
}
