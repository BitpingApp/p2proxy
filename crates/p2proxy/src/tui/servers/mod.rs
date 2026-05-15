use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Cell, Row, Table, Paragraph},
};

use super::{
    Ui, ACCENT, BACKGROUND, BORDER, FOREGROUND, PRIMARY, SECONDARY, SUCCESS,
};

impl Ui {
    pub(crate) fn render_servers_tab(&mut self, frame: &mut Frame<'_>, area: Rect) {
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

        // Header with server count and instructions
        self.render_servers_header(frame, chunks[0]);
        
        // Servers table
        self.render_servers_table(frame, chunks[1]);
    }

    fn render_servers_header(&self, frame: &mut Frame<'_>, area: Rect) {
        let server_count = self.state.servers.len();
        let text = vec![Line::from(vec![
            Span::styled("❯ ", Style::default().fg(BORDER)),
            Span::styled(
                format!("CONFIGURED SERVERS: {}", server_count),
                Style::default()
                    .fg(if server_count > 0 { SUCCESS } else { SECONDARY })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" | ", Style::default().fg(BORDER)),
            Span::styled(
                "Press [N] to create new server",
                Style::default()
                    .fg(ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
        ])];

        let paragraph = Paragraph::new(text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(BORDER))
                    .title(" SERVER MANAGEMENT ")
                    .title_alignment(Alignment::Center),
            )
            .style(Style::default().bg(BACKGROUND))
            .alignment(Alignment::Left);

        frame.render_widget(paragraph, area);
    }

    fn render_servers_table(&self, frame: &mut Frame<'_>, area: Rect) {
        let header_cells = ["Protocol", "Port", "Status", "Sessions", "Bandwidth"]
            .iter()
            .map(|h| Cell::from(*h).style(Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD)));
        
        let header = Row::new(header_cells)
            .style(Style::default().bg(BACKGROUND))
            .height(1)
            .bottom_margin(1);

        let rows: Vec<Row> = self.state.servers
            .iter()
            .map(|server| {
                let status_style = if server.active_sessions > 0 {
                    Style::default().fg(SUCCESS)
                } else {
                    Style::default().fg(SECONDARY)
                };

                let cells = vec![
                    Cell::from(server.protocol.clone()),
                    Cell::from(server.port.to_string()),
                    Cell::from(if server.active_sessions > 0 { "Active" } else { "Idle" })
                        .style(status_style),
                    Cell::from(server.active_sessions.to_string()),
                    Cell::from(format!("↑{:.1}KB/s ↓{:.1}KB/s", 
                                     server.upload_rate, server.download_rate)),
                ];
                Row::new(cells).height(1).bottom_margin(0)
            })
            .collect();

        let table = Table::new(rows, [
            Constraint::Length(10),
            Constraint::Length(8), 
            Constraint::Length(8),
            Constraint::Length(10),
            Constraint::Min(20),
        ])
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(BORDER))
                .title(" PROXY SERVERS ")
                .title_alignment(Alignment::Center),
        )
        .style(Style::default().fg(FOREGROUND))
        .row_highlight_style(Style::default().bg(PRIMARY).fg(BACKGROUND))
        .highlight_symbol("❯ ");

        // For now, render without state (no selection)
        frame.render_widget(table, area);
    }
} 