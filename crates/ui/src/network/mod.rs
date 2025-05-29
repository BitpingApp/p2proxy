use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Cell, Row, Table, Paragraph, Gauge},
};

use super::{
    ui_state::{UIState, ConnectionStatus},
    Ui, ACCENT, BACKGROUND, BORDER, FOREGROUND, PRIMARY, SECONDARY, SUCCESS, WARN, ERROR,
};

impl Ui {
    pub(crate) fn render_network_tab(&mut self, frame: &mut Frame<'_>, area: Rect) {
        // Apply animation to the layout
        let animation_progress = self.ease_out_expo(self.animation_progress());
        if animation_progress >= 1.0 {
            self.needs_render = false;
        }

        let animated_height = ((area.height as f32) * animation_progress) as u16;
        let animated_area = Rect::new(
            area.x,
            area.y.saturating_add(area.height.saturating_sub(animated_height)),
            area.width,
            animated_height,
        );

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(8),  // Network status
                Constraint::Min(0),     // Peer list
            ])
            .split(animated_area);

        // Network status section
        self.render_network_status(frame, chunks[0]);
        
        // Peer connections table
        self.render_peer_connections(frame, chunks[1]);
    }

    fn render_network_status(&self, frame: &mut Frame<'_>, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);

        // Left side - Connection info
        self.render_connection_info(frame, chunks[0]);
        
        // Right side - Network stats
        self.render_network_stats(frame, chunks[1]);
    }

    fn render_connection_info(&self, frame: &mut Frame<'_>, area: Rect) {
        let (status_text, status_color) = match &self.state.connection_status {
            ConnectionStatus::Connected(peer_id) => {
                (format!("Connected to: {:.12}...", peer_id.to_string()), SUCCESS)
            }
            ConnectionStatus::Connecting => ("Connecting to network...".to_string(), WARN),
            ConnectionStatus::Disconnected => ("Disconnected".to_string(), ERROR),
        };

        let local_peer_text = if let Some(peer_id) = &self.state.local_peer_id {
            format!("Local Peer: {:.12}...", peer_id.to_string())
        } else {
            "Local Peer: Not assigned".to_string()
        };

        let text = vec![
            Line::from(vec![
                Span::styled("❯ ", Style::default().fg(BORDER)),
                Span::styled("NETWORK STATUS", Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("❯ ", Style::default().fg(ACCENT)),
                Span::styled(status_text, Style::default().fg(status_color)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("❯ ", Style::default().fg(SECONDARY)),
                Span::styled(local_peer_text, Style::default().fg(FOREGROUND)),
            ]),
        ];

        let paragraph = Paragraph::new(text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(BORDER))
                    .title(" CONNECTION INFO ")
                    .title_alignment(Alignment::Center),
            )
            .style(Style::default().bg(BACKGROUND));

        frame.render_widget(paragraph, area);
    }

    fn render_network_stats(&self, frame: &mut Frame<'_>, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(3),
            ])
            .margin(1)
            .split(area);

        let animation_progress = self.ease_out_expo(self.animation_progress());
        
        // Connected peers gauge
        let peer_count = self.state.peers.len();
        let max_peers = 50; // Assume max 50 peers for gauge
        let peer_percent = ((peer_count as f64 / max_peers as f64) * 100.0).min(100.0) as u16;
        
        let peers_gauge = Gauge::default()
            .block(Block::default().title(" CONNECTED PEERS "))
            .gauge_style(Style::default().fg(ACCENT).bg(Color::Rgb(20, 20, 40)))
            .percent(((peer_percent as f32 * animation_progress) as u16).min(100))
            .label(format!("{}/{}", peer_count, max_peers));

        // Active sessions gauge
        let session_count = self.state.sessions.len();
        let max_sessions = 100; // Assume max 100 sessions for gauge
        let session_percent = ((session_count as f64 / max_sessions as f64) * 100.0).min(100.0) as u16;
        
        let sessions_gauge = Gauge::default()
            .block(Block::default().title(" ACTIVE SESSIONS "))
            .gauge_style(Style::default().fg(SUCCESS).bg(Color::Rgb(20, 40, 20)))
            .percent(((session_percent as f32 * animation_progress) as u16).min(100))
            .label(format!("{}/{}", session_count, max_sessions));

        frame.render_widget(peers_gauge, chunks[0]);
        frame.render_widget(sessions_gauge, chunks[1]);
    }

    fn render_peer_connections(&self, frame: &mut Frame<'_>, area: Rect) {
        let header_cells = ["Peer ID", "Status", "Connection Type"]
            .iter()
            .map(|h| Cell::from(*h).style(Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD)));
        
        let header = Row::new(header_cells)
            .style(Style::default().bg(BACKGROUND))
            .height(1)
            .bottom_margin(1);

        let rows: Vec<Row> = self.state.peers
            .iter()
            .map(|peer_id| {
                let cells = vec![
                    Cell::from(format!("{:.20}...", peer_id.to_string())),
                    Cell::from("Connected").style(Style::default().fg(SUCCESS)),
                    Cell::from("P2P").style(Style::default().fg(ACCENT)),
                ];
                Row::new(cells).height(1).bottom_margin(0)
            })
            .collect();

        let table = Table::new(rows, [
            Constraint::Min(25),
            Constraint::Length(12),
            Constraint::Length(15),
        ])
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(BORDER))
                .title(" PEER CONNECTIONS ")
                .title_alignment(Alignment::Center),
        )
        .style(Style::default().fg(FOREGROUND))
        .row_highlight_style(Style::default().bg(PRIMARY).fg(BACKGROUND))
        .highlight_symbol("❯ ");

        frame.render_widget(table, area);
    }
}
