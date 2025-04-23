use std::time::Instant;

use crate::state::{AppState, ProxySession, SessionId};

use super::{Ui, EVA_BLUE, EVA_FOREGROUND, EVA_ORANGE, EVA_TEAL, EVA_YELLOW};
use ratatui::{
    prelude::*,
    widgets::{Axis, Block, Borders, Chart, Dataset, List, ListItem, Paragraph},
};

impl Ui {
    pub(crate) fn render_sessions_tab(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        state: &AppState,
    ) {
        // Apply animation to the layout
        let animation_progress = self.ease_out_expo(self.animation_progress());
        let animated_width = ((area.width as f32) * animation_progress) as u16;
        let animated_area = Rect::new(
            area.x
                .saturating_add((area.width.saturating_sub(animated_width)) / 2),
            area.y,
            animated_width,
            area.height,
        );

        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
            .split(animated_area);

        // Render sessions list
        self.render_sessions_list(frame, chunks[0], state);

        // Render selected session details
        if let Some(index) = state.selected_session_index {
            if let Some((_, (session_id, session))) = state
                .socks_sessions
                .iter()
                .enumerate()
                .find(|(i, _)| *i == index)
            {
                self.render_session_details(frame, chunks[1], session_id, session);
            } else {
                self.render_no_session_selected(frame, chunks[1]);
            }
        } else {
            self.render_no_session_selected(frame, chunks[1]);
        }
    }

    fn render_sessions_list(&self, frame: &mut Frame<'_>, area: Rect, state: &AppState) {
        let sessions: Vec<ListItem> = state
            .socks_sessions
            .iter()
            .enumerate()
            .map(|(i, (session_id, session))| {
                let is_selected = state.selected_session_index == Some(i);
                let style = if is_selected {
                    Style::default().fg(EVA_ORANGE).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(EVA_FOREGROUND)
                };

                let total_bytes = session
                    .latest_bytes_sent
                    .saturating_add(session.latest_bytes_received);
                let formatted_bytes = if total_bytes < 1024 {
                    format!("{} B", total_bytes)
                } else if total_bytes < 1024 * 1024 {
                    format!("{:.1} KB", total_bytes as f64 / 1024.0)
                } else {
                    format!("{:.1} MB", total_bytes as f64 / (1024.0 * 1024.0))
                };

                // Safely truncate ID to prevent panic
                let id_display = if session_id.to_string().len() > 8 {
                    session_id.to_string()[..8].to_string()
                } else {
                    session_id.to_string()
                };

                ListItem::new(Line::from(vec![
                    Span::styled(format!("#{}: ", i), style),
                    Span::styled(id_display, style),
                    Span::styled(
                        format!(" ({})", formatted_bytes),
                        Style::default().fg(EVA_TEAL),
                    ),
                ]))
            })
            .collect();

        let sessions_list = List::new(sessions)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(EVA_ORANGE))
                    .title(" ACTIVE SESSIONS ")
                    .title_alignment(Alignment::Center),
            )
            .highlight_style(
                Style::default()
                    .bg(Color::Rgb(50, 30, 20))
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(">> ");

        frame.render_widget(sessions_list, area);
    }

    fn render_session_details(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        session_id: &SessionId,
        session: &ProxySession,
    ) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(5), Constraint::Min(0)])
            .split(area);

        // Session info
        let created_duration = Instant::now().duration_since(session.created_at);
        let hours = created_duration.as_secs() / 3600;
        let minutes = (created_duration.as_secs() % 3600) / 60;
        let seconds = created_duration.as_secs() % 60;

        let session_info = vec![
            Line::from(vec![
                Span::styled("SESSION ID: ", Style::default().fg(EVA_YELLOW)),
                Span::styled(session_id.to_string(), Style::default().fg(EVA_FOREGROUND)),
            ]),
            Line::from(vec![
                Span::styled("PEER ID: ", Style::default().fg(EVA_YELLOW)),
                Span::styled(
                    session.peer_id.to_string(),
                    Style::default().fg(EVA_FOREGROUND),
                ),
            ]),
            Line::from(vec![
                Span::styled("DURATION: ", Style::default().fg(EVA_YELLOW)),
                Span::styled(
                    format!("{:02}:{:02}:{:02}", hours, minutes, seconds),
                    Style::default().fg(EVA_FOREGROUND),
                ),
            ]),
        ];

        let info_paragraph = Paragraph::new(session_info).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(EVA_ORANGE))
                .title(" SESSION DETAILS ")
                .title_alignment(Alignment::Center),
        );

        frame.render_widget(info_paragraph, chunks[0]);

        // Apply animation to data points
        let animation_progress = self.ease_out_expo(self.animation_progress());
        let animated_upload: Vec<(f64, f64)> = session
            .bandwidth_history
            .upload_data
            .iter()
            .map(|(t, v)| (*t, *v * animation_progress as f64))
            .collect();

        let animated_download: Vec<(f64, f64)> = session
            .bandwidth_history
            .download_data
            .iter()
            .map(|(t, v)| (*t, *v * animation_progress as f64))
            .collect();

        // Session bandwidth chart
        let upload_dataset = Dataset::default()
            .name("Upload")
            .marker(symbols::Marker::Braille)
            .style(Style::default().fg(EVA_ORANGE))
            .data(&animated_upload);

        let download_dataset = Dataset::default()
            .name("Download")
            .marker(symbols::Marker::Braille)
            .style(Style::default().fg(EVA_BLUE))
            .data(&animated_download);

        // Find max value for y-axis scaling
        let max_value = session
            .bandwidth_history
            .upload_data
            .iter()
            .chain(session.bandwidth_history.download_data.iter())
            .map(|(_, v)| *v)
            .fold(1.0, f64::max);

        // Get the time bounds
        let min_time = session
            .bandwidth_history
            .upload_data
            .first()
            .map(|(t, _)| *t)
            .unwrap_or(0.0);

        let max_time = session
            .bandwidth_history
            .upload_data
            .last()
            .map(|(t, _)| *t)
            .unwrap_or(100.0);

        // Create the chart
        let chart = Chart::new(vec![upload_dataset, download_dataset])
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(EVA_ORANGE))
                    .title(" SESSION BANDWIDTH ")
                    .title_alignment(Alignment::Center),
            )
            .x_axis(
                Axis::default()
                    .title("Time")
                    .style(Style::default().fg(EVA_FOREGROUND))
                    .bounds([min_time, max_time.max(min_time + 1.0)]),
            )
            .y_axis(
                Axis::default()
                    .title("Bandwidth (KB/s)")
                    .style(Style::default().fg(EVA_FOREGROUND))
                    .bounds([0.0, max_value.max(1.0)]),
            );

        frame.render_widget(chart, chunks[1]);
    }

    fn render_no_session_selected(&self, frame: &mut Frame<'_>, area: Rect) {
        let text = vec![
            Line::from(vec![Span::styled(
                "No session selected",
                Style::default().fg(EVA_FOREGROUND),
            )]),
            Line::from(vec![Span::styled(
                "Select a session from the list to view details",
                Style::default().fg(EVA_FOREGROUND),
            )]),
        ];

        let paragraph = Paragraph::new(text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(EVA_ORANGE))
                    .title(" SESSION DETAILS ")
                    .title_alignment(Alignment::Center),
            )
            .alignment(Alignment::Center);

        frame.render_widget(paragraph, area);
    }
}
