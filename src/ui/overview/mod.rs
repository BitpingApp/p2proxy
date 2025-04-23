use std::time::Instant;

use libp2p::bytes;
use ratatui::{
    prelude::*,
    widgets::{Axis, Block, Borders, Chart, Dataset, Gauge, GraphType, Paragraph, Wrap},
};
use tracing::debug;

use super::{
    ui_state::{ConnectionStatus, UIState},
    Ui, ACCENT, BACKGROUND, BORDER, ERROR, FOREGROUND, PRIMARY, SECONDARY, SUCCESS, WARN,
};

impl Ui {
    pub(crate) fn render_overview_tab(&mut self, frame: &mut Frame<'_>, area: Rect) {
        // Apply animation to the layout
        let animation_progress = self.ease_out_expo(self.animation_progress());
        if animation_progress >= 1.0 {
            self.needs_render = false;
        }

        let animated_height = ((area.height as f32) * animation_progress) as u16;
        let animated_area = Rect::new(
            area.x,
            area.y
                .saturating_add(area.height.saturating_sub(animated_height)),
            area.width,
            animated_height,
        );

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
            .split(animated_area);

        // Top section with connection status and stats
        let top_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(chunks[0]);

        self.render_connection_status(frame, top_chunks[0]);
        self.render_overall_stats(frame, top_chunks[1]);

        // Bottom section with bandwidth chart
        self.render_overall_bandwidth(frame, chunks[1]);
    }

    fn render_connection_status(&self, frame: &mut Frame<'_>, area: Rect) {
        let status_color = match &self.state.connection_status {
            ConnectionStatus::Connected(_) => SUCCESS,
            ConnectionStatus::Connecting => WARN,
            ConnectionStatus::Disconnected => SECONDARY,
        };

        let status_text = format!("STATUS: {}", self.state.connection_status.as_str());

        let local_id = if let Some(id) = self.state.local_peer_id {
            format!("LOCAL ID: {}", id)
        } else {
            "LOCAL ID: Not assigned".to_string()
        };

        let relay_id = if let ConnectionStatus::Connected(id) = &self.state.connection_status {
            format!("RELAY ID: {}", id)
        } else {
            "RELAY ID: Not connected".to_string()
        };

        let text = vec![
            Line::from(vec![
                Span::styled("❯ ", Style::default().fg(BORDER)),
                Span::styled(
                    status_text,
                    Style::default()
                        .fg(status_color)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("❯ ", Style::default().fg(ACCENT)),
                Span::styled(local_id, Style::default().fg(FOREGROUND)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("❯ ", Style::default().fg(SECONDARY)),
                Span::styled(relay_id, Style::default().fg(FOREGROUND)),
            ]),
        ];

        let paragraph = Paragraph::new(text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(BORDER))
                    .title(" CONNECTION STATUS ")
                    .title_alignment(Alignment::Center),
            )
            .style(Style::default().bg(BACKGROUND))
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: true });

        frame.render_widget(paragraph, area);
    }

    fn render_overall_stats(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let state = &self.state;

        let total_sessions = state.sessions.len();
        let active_peers = state.peers.len();

        let format_bytes = |bytes: u64| -> String {
            if bytes < 1024 {
                format!("{} B", bytes)
            } else if bytes < 1024 * 1024 {
                format!("{:.2} KB", bytes as f64 / 1024.0)
            } else if bytes < 1024 * 1024 * 1024 {
                format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
            } else {
                format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
            }
        };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Length(3),
            ])
            .margin(1)
            .split(area);

        // Apply animation to gauge percentages
        let animation_progress = self.ease_out_expo(self.animation_progress());
        if animation_progress >= 1.0 {
            self.needs_render = false;
        }

        // Active sessions gauge
        let sessions_gauge = Gauge::default()
            .block(Block::default().title(" ACTIVE SESSIONS "))
            .gauge_style(Style::default().fg(SUCCESS).bg(Color::Rgb(20, 40, 20)))
            .percent(if total_sessions > 0 {
                ((100.0 * animation_progress) as u16).min(100)
            } else {
                0
            })
            .label(format!("{}", total_sessions));

        // Connected peers gauge
        let peers_gauge = Gauge::default()
            .block(Block::default().title(" CONNECTED PEERS "))
            .gauge_style(Style::default().fg(ACCENT).bg(Color::Rgb(20, 20, 40)))
            .percent(if active_peers > 0 {
                ((100.0 * animation_progress) as u16).min(100)
            } else {
                0
            })
            .label(format!("{}", active_peers));

        // Upload gauge - calculate percentage based on some max value (e.g., 100MB)
        let max_bytes = 100 * 1024 * 1024; // 100MB as reference
        let upload_percent =
            ((self.state.total_upload as f64 / max_bytes as f64) * 100.0).min(100.0) as u16;
        let upload_gauge = Gauge::default()
            .block(Block::default().title(" TOTAL UPLOAD "))
            .gauge_style(Style::default().fg(BORDER).bg(Color::Rgb(40, 20, 20)))
            .percent(((upload_percent as f32 * animation_progress) as u16).min(100))
            .label(format_bytes(self.state.total_upload));

        // Download gauge
        let download_percent =
            ((self.state.total_download as f64 / max_bytes as f64) * 100.0).min(100.0) as u16;
        let download_gauge = Gauge::default()
            .block(Block::default().title(" TOTAL DOWNLOAD "))
            .gauge_style(Style::default().fg(SECONDARY).bg(Color::Rgb(30, 20, 30)))
            .percent(((download_percent as f32 * animation_progress) as u16).min(100))
            .label(format_bytes(self.state.total_download));

        frame.render_widget(sessions_gauge, chunks[0]);
        frame.render_widget(peers_gauge, chunks[1]);
        frame.render_widget(upload_gauge, chunks[2]);
        frame.render_widget(download_gauge, chunks[3]);
    }

    fn render_overall_bandwidth(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let state = &self.state;

        // Create simple datasets directly from our data
        // Note: Ratatui expects data as Vec<(f64, f64)> where each tuple is (x, y)
        let mapped_upload = state
            .upload_graph
            .iter()
            .map(|(time, bytes)| (time.timestamp() as f64, *bytes))
            .collect::<Vec<(f64, f64)>>();
        let mapped_download = state
            .download_graph
            .iter()
            .map(|(time, bytes)| (time.timestamp() as f64, *bytes))
            .collect::<Vec<(f64, f64)>>();

        // Create the upload dataset
        let upload_dataset = Dataset::default()
            .name("Upload")
            .marker(symbols::Marker::Braille)
            .style(Style::default().fg(PRIMARY))
            .graph_type(GraphType::Line)
            .data(&mapped_upload);

        // Create the download dataset
        let download_dataset = Dataset::default()
            .name("Download")
            .marker(symbols::Marker::Braille)
            .style(Style::default().fg(SECONDARY))
            .graph_type(GraphType::Line)
            .data(&mapped_download);

        // let Some((min_upload_timestamp, max_upload_timestamp, min_upload, max_upload)) =
        //     state.get_upload_stats()
        // else {
        //     return;
        // };

        // let Some((min_download_timestamp, max_download_timestamp, min_download, max_download)) =
        //     state.get_download_stats()
        // else {
        //     return;
        // };

        // // Find combined min/max values
        // let min_timestamp = min_upload_timestamp.min(min_download_timestamp);
        // let max_timestamp = max_upload_timestamp.max(max_download_timestamp);
        // let min_value = min_upload.min(min_download);
        // let max_value = max_upload.max(max_download);

        // Create the chart
        let chart = Chart::new(vec![upload_dataset, download_dataset])
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(BORDER))
                    .title(" SYSTEM BANDWIDTH (LAST 30 SECONDS) ")
                    .title_alignment(Alignment::Center),
            )
            .x_axis(
                Axis::default()
                    .title("Time")
                    .style(Style::default().fg(ACCENT)), // .bounds(x_bounds)
                                                         // .labels(vec![
                                                         //     Span::styled(min_timestamp.to_string(), Style::default().fg(FOREGROUND)),
                                                         //     Span::styled(max_timestamp.to_string(), Style::default().fg(FOREGROUND)),
                                                         // ]),
            )
            .y_axis(
                Axis::default()
                    .title("Bandwidth (Mbps)")
                    .style(Style::default().fg(ACCENT)), // .bounds(y_bounds)
                                                         // .labels(vec![
                                                         //     Span::styled("0", Style::default().fg(FOREGROUND)),
                                                         //     Span::styled(
                                                         //         format!("{:.1}", max_value / 2.0),
                                                         //         Style::default().fg(FOREGROUND),
                                                         //     ),
                                                         //     Span::styled(format!("{:.1}", max_value), Style::default().fg(FOREGROUND)),
                                                         // ]),
            );

        frame.render_widget(chart, area);
    }

    pub(crate) fn render_footer_or_logs(&self, frame: &mut Frame<'_>, area: Rect) {
        if self.show_logs {
            // Use tui-logger for the logs panel as well
            let tui_widget = tui_logger::TuiLoggerWidget::default()
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(BORDER))
                        .title(" LOGS ")
                        .title_alignment(Alignment::Center),
                )
                .style_error(Style::default().fg(ERROR))
                .style_warn(Style::default().fg(WARN))
                .style_info(Style::default().fg(SUCCESS))
                .style_debug(Style::default().fg(ACCENT))
                .style_trace(Style::default().fg(SECONDARY))
                .output_separator('|')
                .output_timestamp(Some("%H:%M:%S".to_string()))
                .output_target(true)
                .output_file(false)
                .output_line(false);

            frame.render_widget(tui_widget, area);
        } else {
            // Render footer with help text
            let text = vec![Line::from(vec![
                Span::styled(
                    " [Tab] ",
                    Style::default().fg(BORDER).add_modifier(Modifier::BOLD),
                ),
                Span::styled("Switch Tabs", Style::default().fg(FOREGROUND)),
                Span::styled(" | ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    " [L] ",
                    Style::default().fg(BORDER).add_modifier(Modifier::BOLD),
                ),
                Span::styled("Toggle Logs", Style::default().fg(FOREGROUND)),
                Span::styled(" | ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    " [↑/↓] ",
                    Style::default().fg(BORDER).add_modifier(Modifier::BOLD),
                ),
                Span::styled("Navigate", Style::default().fg(FOREGROUND)),
                Span::styled(" | ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    " [Q] ",
                    Style::default().fg(BORDER).add_modifier(Modifier::BOLD),
                ),
                Span::styled("Quit", Style::default().fg(FOREGROUND)),
            ])];

            let paragraph = Paragraph::new(text)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(BORDER))
                        .title(" P2PROXY COMMAND INTERFACE ")
                        .title_alignment(Alignment::Center),
                )
                .style(Style::default().bg(BACKGROUND))
                .alignment(Alignment::Center);

            frame.render_widget(paragraph, area);
        }
    }
}
