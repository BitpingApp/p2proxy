use std::time::Duration;
use std::time::Instant;

use crate::gauge;
use crate::AppState;
use crate::BandwidthHistory;
use crate::SocksSession;
use crate::APP_STATE;
use color_eyre::eyre::Result;
use crossterm::event;
use crossterm::event::Event;
use crossterm::event::KeyCode;
use layout::*;
use libp2p::PeerId;
use metrics::histogram;
use ratatui::*;
use style::*;
use widgets::*;

pub mod thread;

// Update metrics from app state
fn update_metrics(state: &AppState) {
    // Update gauges
    gauge!("p2proxy_peers_connected").set(state.peers.len() as f64);
    gauge!("p2proxy_socks_sessions_active").set(state.socks_sessions.len() as f64);

    // Calculate total bytes
    let total_sent: u64 = state.socks_sessions.iter().map(|s| s.bytes_sent).sum();
    let total_received: u64 = state.socks_sessions.iter().map(|s| s.bytes_received).sum();

    // Update counters (using the difference from last update would be more accurate)
    gauge!("p2proxy_bytes_sent").set(total_sent as f64);
    gauge!("p2proxy_bytes_received").set(total_received as f64);

    // Session durations
    for session in &state.socks_sessions {
        histogram!("p2proxy_session_duration_seconds")
            .record(session.created_at.elapsed().as_secs_f64());
    }
}

fn render(frame: &mut Frame) {
    let mut state = APP_STATE.lock().unwrap();

    // Update metrics
    update_metrics(&state);

    // Create main layout
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Length(7),  // Header with logo and status
                Constraint::Length(1),  // Separator
                Constraint::Length(10), // Sessions list
                Constraint::Length(1),  // Separator
                Constraint::Min(10),    // Bandwidth chart
                Constraint::Length(1),  // Separator
            ]
            .as_ref(),
        )
        .split(frame.size());

    // Header with logo and status
    render_header(frame, chunks[0], &state);

    // Separator
    render_separator(frame, chunks[1]);

    // Sessions list
    render_sessions_list(frame, chunks[2], &mut state);

    // Separator
    render_separator(frame, chunks[3]);

    // Bandwidth chart for selected session
    render_bandwidth_chart(frame, chunks[4], &state);

    // Separator
    render_separator(frame, chunks[5]);
}

fn render_header(frame: &mut Frame, area: Rect, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(
            [
                Constraint::Percentage(70), // Logo
                Constraint::Percentage(30), // Status
            ]
            .as_ref(),
        )
        .split(area);

    // ASCII Art Logo
    let logo = r#"
 ??????? ??????? ??????? ???????  ??????? ???  ??????   ???
 ????????????????????????????????????????????????????? ????
 ???????? ??????????????????????????   ??? ??????  ??????? 
 ??????? ??????? ??????????????? ???   ??? ??????   ?????  
 ???     ???????????  ??????     ????????????? ???   ???   
 ???     ???????????  ??????      ??????? ???  ???   ???   
"#;

    let logo_widget = Paragraph::new(logo)
        .style(Style::default().fg(Color::Rgb(0, 255, 255)))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(0, 255, 255)))
                .style(Style::default().bg(Color::Rgb(20, 20, 40))),
        );

    // Status widget
    let status_text = format!(
        "\nStatus: {}\n\nPeers: {}\nSessions: {}",
        state.connection_status.as_str(),
        state.peers.len(),
        state.socks_sessions.len()
    );

    let status_widget = Paragraph::new(status_text)
        .style(Style::default().fg(Color::Rgb(255, 255, 255)))
        .block(
            Block::default()
                .title("Connection Info")
                .title_style(
                    Style::default()
                        .fg(Color::Rgb(255, 0, 128))
                        .add_modifier(Modifier::BOLD),
                )
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(0, 255, 255)))
                .style(Style::default().bg(Color::Rgb(20, 20, 40))),
        );

    frame.render_widget(logo_widget, chunks[0]);
    frame.render_widget(status_widget, chunks[1]);
}

fn render_separator(frame: &mut Frame, area: Rect) {
    let separator = Paragraph::new("?".repeat(area.width as usize))
        .style(Style::default().fg(Color::Rgb(0, 255, 255)));

    frame.render_widget(separator, area);
}

fn render_sessions_list(frame: &mut Frame, area: Rect, state: &mut AppState) {
    // Create list items
    let items: Vec<ListItem> = state
        .socks_sessions
        .iter()
        .map(|session| {
            let duration = session.created_at.elapsed();
            let duration_text =
                format!("{}m {}s", duration.as_secs() / 60, duration.as_secs() % 60);

            let text = format!(
                "Session: {} | Peer: {} | Duration: {} | Sent: {} KB | Received: {} KB",
                session.id,
                session.peer_id.to_string(),
                duration_text,
                session.bytes_sent / 1024,
                session.bytes_received / 1024
            );

            ListItem::new(text)
        })
        .collect();

    // Create list widget
    let sessions_list = List::new(items)
        .block(
            Block::default()
                .title("Active SOCKS5 Sessions")
                .title_style(
                    Style::default()
                        .fg(Color::Rgb(255, 0, 128))
                        .add_modifier(Modifier::BOLD),
                )
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(0, 255, 255)))
                .style(Style::default().bg(Color::Rgb(20, 20, 40))),
        )
        .highlight_style(
            Style::default()
                .bg(Color::Rgb(40, 40, 80))
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">> ");

    // Render the list with state
    frame.render_stateful_widget(sessions_list, area, &mut state.sessions_state);

    // Update selected session index based on list state
    state.selected_session_index = state.sessions_state.selected();
}

fn render_bandwidth_chart(frame: &mut Frame, area: Rect, state: &AppState) {
    // Get selected session
    let selected_session = state
        .selected_session_index
        .and_then(|i| state.socks_sessions.get(i));

    let chart_block = Block::default()
        .title(if selected_session.is_some() {
            format!("Bandwidth Usage - Session {}", selected_session.unwrap().id)
        } else {
            "Bandwidth Usage - No Session Selected".to_string()
        })
        .title_style(
            Style::default()
                .fg(Color::Rgb(255, 0, 128))
                .add_modifier(Modifier::BOLD),
        )
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(0, 255, 255)))
        .style(Style::default().bg(Color::Rgb(20, 20, 40)));

    if let Some(session) = selected_session {
        let upload_dataset = Dataset::default()
            .name("Upload")
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(Color::Rgb(0, 255, 128)))
            .data(&session.bandwidth_history.upload_data);

        let download_dataset = Dataset::default()
            .name("Download")
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(Color::Rgb(255, 0, 128)))
            .data(&session.bandwidth_history.download_data);

        // Find the max value for y-axis scaling
        let max_upload = session
            .bandwidth_history
            .upload_data
            .iter()
            .map(|&(_, y)| y)
            .fold(0.0, f64::max);

        let max_download = session
            .bandwidth_history
            .download_data
            .iter()
            .map(|&(_, y)| y)
            .fold(0.0, f64::max);

        let max_value = f64::max(max_upload, max_download).max(1.0); // At least 1.0 to avoid empty chart

        let bandwidth_chart = Chart::new([upload_dataset, download_dataset].to_vec())
            .block(chart_block)
            .x_axis(
                Axis::default()
                    .title("Time")
                    // .title_style(Style::default().fg(Color::Rgb(255, 255, 0)))
                    .style(Style::default().fg(Color::Rgb(255, 255, 0)))
                    .bounds([
                        session.bandwidth_history.time_counter
                            - session.bandwidth_history.max_points as f64,
                        session.bandwidth_history.time_counter,
                    ])
                    .labels(
                        ["", "now"]
                            .iter()
                            .map(|s| s.to_string())
                            .collect::<Vec<_>>(),
                    ),
            )
            .y_axis(
                Axis::default()
                    .title("KB/s")
                    // .title_style(Style::default().fg(Color::Rgb(255, 255, 0)))
                    .style(Style::default().fg(Color::Rgb(255, 255, 0)))
                    .bounds([0.0, max_value * 1.1]) // Add 10% padding at the top
                    .labels(
                        ["0", &format!("{:.1}", max_value)]
                            .iter()
                            .map(|s| s.to_string())
                            .collect::<Vec<_>>(),
                    ),
            );

        frame.render_widget(bandwidth_chart, area);
    } else {
        // No session selected, just show the empty block
        frame.render_widget(chart_block, area);
    }
}

// Update the run function to handle session selection and render at 300ms intervals
pub fn run(mut terminal: DefaultTerminal) -> Result<()> {
    // Set up a ticker for the render rate
    let tick_rate = Duration::from_millis(300);
    let mut last_tick = Instant::now();

    loop {
        // Render at the specified interval
        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        // Draw the UI if it's time to render
        if last_tick.elapsed() >= tick_rate {
            terminal.draw(render)?;
            last_tick = Instant::now();
        }

        // Poll for events with the remaining time until next render
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => break Ok(()),
                    KeyCode::Down => {
                        let mut state = APP_STATE.lock().unwrap();
                        let sessions_len = state.socks_sessions.len();

                        if sessions_len > 0 {
                            let new_index = match state.sessions_state.selected() {
                                Some(i) => {
                                    if i >= sessions_len - 1 {
                                        0
                                    } else {
                                        i + 1
                                    }
                                }
                                None => 0,
                            };
                            state.sessions_state.select(Some(new_index));
                        }
                    }
                    KeyCode::Up => {
                        let mut state = APP_STATE.lock().unwrap();
                        let sessions_len = state.socks_sessions.len();

                        if sessions_len > 0 {
                            let new_index = match state.sessions_state.selected() {
                                Some(i) => {
                                    if i == 0 {
                                        sessions_len - 1
                                    } else {
                                        i - 1
                                    }
                                }
                                None => 0,
                            };
                            state.sessions_state.select(Some(new_index));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

// Add this function to track SOCKS5 sessions
pub fn track_socks_session(peer_id: PeerId, session_id: String) {
    let mut state = APP_STATE.lock().unwrap();
    state.socks_sessions.push(SocksSession {
        id: session_id,
        peer_id,
        created_at: Instant::now(),
        bytes_sent: 0,
        bytes_received: 0,
        bandwidth_history: BandwidthHistory::default(),
    });

    // Select the first session if none is selected
    if state.sessions_state.selected().is_none() && !state.socks_sessions.is_empty() {
        state.sessions_state.select(Some(0));
    }
}

// Add this function to update SOCKS5 session stats
pub fn update_socks_stats(session_id: &str, bytes_sent: u64, bytes_received: u64) {
    let mut state = APP_STATE.lock().unwrap();
    if let Some(session) = state.socks_sessions.iter_mut().find(|s| s.id == session_id) {
        session.bytes_sent += bytes_sent;
        session.bytes_received += bytes_received;

        // Calculate bandwidth in KB/s (assuming this is called every second)
        let upload_speed = bytes_sent as f64 / 1024.0;
        let download_speed = bytes_received as f64 / 1024.0;

        // Update bandwidth history
        session
            .bandwidth_history
            .add_sample(upload_speed, download_speed);
    }
}
