use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Cell, Row, Table},
};

use super::{
    ui_state::UIState,
    Ui, ACCENT, BACKGROUND, BORDER, FOREGROUND, PRIMARY, SECONDARY, SUCCESS,
};

impl Ui {
    pub(crate) fn render_sessions_tab(&mut self, frame: &mut Frame<'_>, area: Rect) {
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
            .constraints([Constraint::Length(3), Constraint::Min(0)])
            .split(animated_area);

        // Header with session count
        self.render_sessions_header(frame, chunks[0]);
        
        // Sessions table
        self.render_sessions_table(frame, chunks[1]);
    }

    fn render_sessions_header(&self, frame: &mut Frame<'_>, area: Rect) {
        let session_count = self.state.sessions.len();
        let text = vec![Line::from(vec![
            Span::styled("❯ ", Style::default().fg(BORDER)),
            Span::styled(
                format!("ACTIVE SESSIONS: {}", session_count),
                Style::default()
                    .fg(if session_count > 0 { SUCCESS } else { SECONDARY })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" | ", Style::default().fg(BORDER)),
            Span::styled(
                format!("CONNECTED PEERS: {}", self.state.peers.len()),
                Style::default()
                    .fg(if self.state.peers.len() > 0 { ACCENT } else { SECONDARY })
                    .add_modifier(Modifier::BOLD),
            ),
        ])];

        let paragraph = ratatui::widgets::Paragraph::new(text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(BORDER))
                    .title(" SESSION OVERVIEW ")
                    .title_alignment(Alignment::Center),
            )
            .style(Style::default().bg(BACKGROUND))
            .alignment(Alignment::Left);

        frame.render_widget(paragraph, area);
    }

    fn render_sessions_table(&self, frame: &mut Frame<'_>, area: Rect) {
        let header_cells = ["Session ID", "Peer ID", "Target", "Status"]
            .iter()
            .map(|h| Cell::from(*h).style(Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD)));
        
        let header = Row::new(header_cells)
            .style(Style::default().bg(BACKGROUND))
            .height(1)
            .bottom_margin(1);

        let rows: Vec<Row> = self.state.sessions
            .iter()
            .map(|(session_id, session)| {
                let cells = vec![
                    Cell::from(format!("{:.8}...", session_id.to_string())),
                    Cell::from(format!("{:.12}...", session.peer_id.to_string())),
                    Cell::from(format!("{}", session.endpoint)),
                    Cell::from("Active").style(Style::default().fg(SUCCESS)),
                ];
                Row::new(cells).height(1).bottom_margin(0)
            })
            .collect();

        let table = Table::new(rows, [
            Constraint::Length(12),
            Constraint::Length(16), 
            Constraint::Min(20),
            Constraint::Length(8),
        ])
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(BORDER))
                .title(" ACTIVE PROXY SESSIONS ")
                .title_alignment(Alignment::Center),
        )
        .style(Style::default().fg(FOREGROUND))
        .row_highlight_style(Style::default().bg(PRIMARY).fg(BACKGROUND))
        .highlight_symbol("❯ ");

        // For now, render without state (no selection)
        frame.render_widget(table, area);
    }
} 