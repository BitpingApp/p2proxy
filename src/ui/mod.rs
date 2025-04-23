use crate::events::Events;
use color_eyre::eyre::Result;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use libp2p::quic::Connecting;
use ratatui::{
    prelude::*,
    widgets::{
        Axis, Block, Borders, Chart, Clear, Dataset, Gauge, List, ListItem, Paragraph, Tabs, Wrap,
    },
};
use std::{
    io, process,
    time::{Duration, Instant},
};
use tokio::{select, sync::mpsc::Receiver, time::interval};
use tracing::{debug, info};
use ui_state::{ConnectionStatus, ProxySession, UIState};

mod logs;
// mod network;
mod overview;
// mod sessions;
mod splashscreen;
mod ui_state;

// Colors inspired by Evangelion UI
// NERV Command Center Purple - like the main HUD screens
const PRIMARY: Color = Color::from_u32(0x523874); // Deep Purple

// Eva Unit-01 Green - secondary color from Eva-01's armor
const SECONDARY: Color = Color::from_u32(0xadf182); // Lime Green

// Eva Unit-02 Red - accent color from Asuka's Eva
const ACCENT: Color = Color::from_u32(0xdc7d68); // Coral/Red

// Terminal Dogma Gray - border color inspired by NERV HQ
const BORDER: Color = Color::from_u32(0x916cad); // Lavender/Muted Purple

// NERV Success Green - like positive sync ratio displays
const SUCCESS: Color = Color::from_u32(0xc7fba5); // Mint Green

// Angel Alert Red - emergency warning color
const ERROR: Color = Color::from_u32(0xff5555); // Bright Red

// LCL Orange - warning color like the LCL fluid
const WARN: Color = Color::from_u32(0xffaa00); // Orange

// SEELE Monolith Blue - miscellaneous color
const MISC: Color = Color::from_u32(0x66ccff); // Light Blue
const BACKGROUND: Color = Color::Reset;
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
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            show_splash: true,
            splash_start_time: Instant::now(),
            show_logs: false,
            tab_index: 0,
            tabs: vec!["OVERVIEW", "SESSIONS", "NETWORK", "LOGS"],
            animation_start: Instant::now(),
            needs_render: true,
            is_animating: true,
            state: UIState::new(),
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

    pub fn next_tab(&mut self) {
        self.tab_index = (self.tab_index + 1) % self.tabs.len();
        self.start_animation();
    }

    // Main function that sets up and runs the UI
    pub async fn run_ui(mut state_events: Receiver<Events>) -> Result<()> {
        // Set up terminal
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // Create UI state
        let mut ui = Ui::new();

        let mut idle_interval = interval(Duration::from_millis(500)); // Slow background polling
        let mut event_stream = EventStream::new();

        ui.run_splash_screen_animation(&mut terminal).await;

        // Main loop
        loop {
            select! {
                _ = idle_interval.tick(), if !ui.is_animating => {
                    ui.mark_dirty();
                },
                Some(event) = state_events.recv() => ui.handle_swarm_events(event),
                Some(Ok(event)) = event_stream.next() => {
                    if let Event::Key(key) = event {
                        match key.code {
                            KeyCode::Char('q') => {
                                break;
                            },
                            KeyCode::Char('l') => ui.toggle_logs(),
                            KeyCode::Tab => ui.next_tab(),
                            KeyCode::BackTab => ui.previous_tab(),
                            KeyCode::Up => {
                                // Handle selection up - avoid holding lock for too long
                                // APP_STATE.update(|state| {
                                //     if let Some(index) = state.selected_session_index {
                                //         if index > 0 && !state.socks_sessions.is_empty() {
                                //             state.selected_session_index = Some(index - 1);
                                //         }
                                //     } else if !state.socks_sessions.is_empty() {
                                //         state.selected_session_index = Some(0);
                                //     }
                                // }).await;
                                ui.mark_dirty();
                            }
                            KeyCode::Down => {
                                // Handle selection down - avoid holding lock for too long
                                // APP_STATE.update(|state| {
                                //     if let Some(index) = state.selected_session_index {
                                //         if index + 1 < state.socks_sessions.len() {
                                //             state.selected_session_index = Some(index + 1);
                                //         }
                                //     } else if !state.socks_sessions.is_empty() {
                                //         state.selected_session_index = Some(0);
                                //     }
                                // }).await;
                                ui.mark_dirty();
                            }
                            _ => {}
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

        // Restore terminal
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;

        process::exit(0)
    }

    pub fn handle_swarm_events(&mut self, event: Events) {
        debug!(?event, "Got new event");
        match event {
            Events::LocalPeerId(peer_id) => {
                self.state.local_peer_id.replace(peer_id);
            }
            Events::Connection(connection_events) => match connection_events {
                crate::events::ConnectionEvents::Connecting => {
                    self.state.connection_status = ConnectionStatus::Connecting;
                }
                crate::events::ConnectionEvents::Connected(peer_id) => {
                    self.state.connection_status = ConnectionStatus::Connected(peer_id);
                }
                crate::events::ConnectionEvents::Disconnected => {
                    self.state.connection_status = ConnectionStatus::Disconnected;
                }
            },
            Events::Session(session_events) => match session_events {
                crate::events::SessionEvents::New(id, endpoint, peer_id) => {
                    self.state.sessions.insert(
                        id,
                        ProxySession {
                            id,
                            peer_id,
                            endpoint,
                        },
                    );
                }
                crate::events::SessionEvents::End(uuid) => {
                    self.state.sessions.remove(&uuid);
                }
            },
            Events::Bandwidth(bandwidth_events) => match bandwidth_events {
                crate::events::BandwidthEvents::Upload(session_id, u) => {
                    self.state.add_upload(u);
                }
                crate::events::BandwidthEvents::Download(session_id, d) => {
                    self.state.add_download(d);
                }
            },
        };

        // self.mark_dirty();
    }

    pub fn previous_tab(&mut self) {
        if self.tab_index > 0 {
            self.tab_index -= 1;
        } else {
            self.tab_index = self.tabs.len().saturating_sub(1);
        }
        self.start_animation();
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

        // Render main content based on selected tab
        match self.tab_index {
            0 => self.render_overview_tab(frame, chunks[1]),
            // 1 => self.render_sessions_tab(frame, chunks[1], &state),
            // 2 => self.render_network_tab(frame, chunks[1], &state),
            3 => self.render_logs_tab(frame, chunks[1]),
            _ => {}
        }

        // Render footer or logs
        self.render_footer_or_logs(frame, chunks[2]);
        self.mark_clean();
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
