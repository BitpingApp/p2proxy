use ratatui::{
    prelude::*,
    widgets::{Axis, Block, Borders, Chart, Dataset, Gauge, GraphType, Paragraph, Wrap},
};

use super::{
    ACCENT, BACKGROUND, BORDER, ERROR, FOREGROUND, PRIMARY, SECONDARY, SUCCESS, Ui, WARN,
    ui_state::ConnectionStatus,
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

        // Bottom section: error banner (if any) above the bandwidth chart.
        // Sized as 4 rows when an error is present, 0 when not — keeps the
        // chart at full height during happy path.
        if self.state.last_error.is_some() {
            let bottom = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(4), Constraint::Min(0)])
                .split(chunks[1]);
            self.render_error_banner(frame, bottom[0]);
            self.render_overall_bandwidth(frame, bottom[1]);
        } else {
            self.render_overall_bandwidth(frame, chunks[1]);
        }
    }

    pub(crate) fn render_error_banner(&self, frame: &mut Frame<'_>, area: Rect) {
        let Some(msg) = self.state.last_error.as_deref() else {
            return;
        };
        let text = vec![Line::from(vec![
            Span::styled(
                "⚠  ",
                Style::default().fg(WARN).add_modifier(Modifier::BOLD),
            ),
            Span::styled(msg, Style::default().fg(ERROR)),
        ])];
        let paragraph = Paragraph::new(text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(ERROR))
                    .title(Span::styled(
                        " PROXY ALERT ",
                        Style::default().fg(ERROR).add_modifier(Modifier::BOLD),
                    ))
                    .title_alignment(Alignment::Center),
            )
            .style(Style::default().bg(BACKGROUND))
            .wrap(Wrap { trim: true });
        frame.render_widget(paragraph, area);
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
        // Bucket the last 30 seconds of samples into one-second bins and
        // convert each bin's total bytes to Mbps. The raw graphs are
        // sampled per `DataTransferred` event (one per read-syscall
        // worth of bytes through the proxy), which is wildly non-uniform
        // — plotting raw samples produces nonsense numbers and a chart
        // axis label that says "Mbps" but actually shows kB-per-chunk.
        //
        // X axis: seconds-ago in [-30.0, 0.0] so the points sit inside
        // ratatui's drawable area (the previous code left the axis at
        // its default [0, 1] bounds while X values were ~1.7e9 Unix
        // timestamps — the points existed but rendered far off-screen,
        // which is why the chart looked permanently empty).
        let now = chrono::Utc::now();
        const WINDOW_SECS: i64 = 30;

        // Returns `Vec<(seconds_ago, mbps)>` for the configured window.
        // Empty bins are emitted as zero so the line returns to baseline
        // when traffic stops, rather than being invisible.
        let bucketize = |graph: &[(chrono::DateTime<chrono::Utc>, f64)]| -> Vec<(f64, f64)> {
            let mut buckets = [0.0_f64; WINDOW_SECS as usize];
            for (ts, kib) in graph {
                let age = (now - *ts).num_seconds();
                if (0..WINDOW_SECS).contains(&age) {
                    // The graph stores `bytes_per_sample / 1024.0`
                    // (kibibytes per sample). Convert back to bytes,
                    // then to Mbps for this 1-second bucket.
                    let bytes = kib * 1024.0;
                    buckets[age as usize] += bytes;
                }
            }
            (0..WINDOW_SECS)
                .map(|i| {
                    let bytes_in_bucket = buckets[i as usize];
                    let mbps = (bytes_in_bucket * 8.0) / 1_000_000.0;
                    // X axis runs left-to-right as -30 → 0, so older
                    // samples land further left.
                    ((-i) as f64, mbps)
                })
                .collect()
        };

        let upload_points = bucketize(&self.state.upload_graph);
        let download_points = bucketize(&self.state.download_graph);

        // Y bound: max of either dataset (×1.1 headroom), with a floor
        // of 1.0 Mbps so an idle proxy doesn't render a flat zero-axis
        // chart that looks broken.
        let peak = upload_points
            .iter()
            .chain(download_points.iter())
            .map(|(_, y)| *y)
            .fold(0.0_f64, f64::max);
        let y_max = (peak * 1.1).max(1.0);

        let upload_dataset = Dataset::default()
            .name("Upload Mbps")
            .marker(symbols::Marker::Braille)
            .style(Style::default().fg(PRIMARY))
            .graph_type(GraphType::Line)
            .data(&upload_points);

        let download_dataset = Dataset::default()
            .name("Download Mbps")
            .marker(symbols::Marker::Braille)
            .style(Style::default().fg(SECONDARY))
            .graph_type(GraphType::Line)
            .data(&download_points);

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
                    .title("Seconds ago")
                    .style(Style::default().fg(ACCENT))
                    .bounds([-(WINDOW_SECS as f64), 0.0])
                    .labels(vec![
                        Span::styled(format!("-{WINDOW_SECS}s"), Style::default().fg(ACCENT)),
                        Span::styled("-15s", Style::default().fg(ACCENT)),
                        Span::styled("now", Style::default().fg(ACCENT)),
                    ]),
            )
            .y_axis(
                Axis::default()
                    .title("Mbps")
                    .style(Style::default().fg(ACCENT))
                    .bounds([0.0, y_max])
                    .labels(vec![
                        Span::styled("0", Style::default().fg(ACCENT)),
                        Span::styled(format!("{:.1}", y_max / 2.0), Style::default().fg(ACCENT)),
                        Span::styled(format!("{:.1}", y_max), Style::default().fg(ACCENT)),
                    ]),
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
                    " [E] ",
                    Style::default().fg(BORDER).add_modifier(Modifier::BOLD),
                ),
                Span::styled("Export Logs", Style::default().fg(FOREGROUND)),
                Span::styled(" | ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    " [↑/↓] ",
                    Style::default().fg(BORDER).add_modifier(Modifier::BOLD),
                ),
                Span::styled("Navigate", Style::default().fg(FOREGROUND)),
                Span::styled(" | ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    " [⏎] ",
                    Style::default().fg(BORDER).add_modifier(Modifier::BOLD),
                ),
                Span::styled("Expand", Style::default().fg(FOREGROUND)),
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
