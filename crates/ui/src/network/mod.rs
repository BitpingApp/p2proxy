use std::time::Instant;

use crate::state::AppState;

use super::{Ui, EVA_BLUE, EVA_FOREGROUND, EVA_ORANGE, EVA_TEAL, EVA_YELLOW};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph, Wrap},
};

impl Ui {
    pub(crate) fn render_network_tab(&self, frame: &mut Frame<'_>, area: Rect, state: &AppState) {
        // Apply animation - slide in from right
        let animation_progress = self.ease_out_expo(self.animation_progress());
        let animated_width = ((area.width as f32) * animation_progress) as u16;
        let animated_area = Rect::new(
            area.x
                .saturating_add(area.width.saturating_sub(animated_width)),
            area.y,
            animated_width,
            area.height,
        );

        let peers: Vec<(&libp2p::PeerId, &crate::state::PeerInfo)> = state.peers.iter().collect();

        if peers.is_empty() {
            let text = vec![
                Line::from(vec![Span::styled(
                    "No peers connected",
                    Style::default().fg(EVA_FOREGROUND),
                )]),
                Line::from(vec![Span::styled(
                    "Waiting for connections...",
                    Style::default().fg(EVA_FOREGROUND),
                )]),
            ];

            let paragraph = Paragraph::new(text)
                .block(
                    Block::default()
                        .borders(ratatui::widgets::Borders::ALL)
                        .border_style(Style::default().fg(EVA_ORANGE))
                        .title(" NETWORK PEERS ")
                        .title_alignment(Alignment::Center),
                )
                .alignment(Alignment::Center);

            frame.render_widget(paragraph, animated_area);
            return;
        }

        // Create a list of peers with their details
        let mut peer_text = Vec::new();

        for (i, (peer_id, info)) in peers.iter().enumerate() {
            let connected_duration = Instant::now().duration_since(info.connected_at);
            let hours = connected_duration.as_secs() / 3600;
            let minutes = (connected_duration.as_secs() % 3600) / 60;
            let seconds = connected_duration.as_secs() % 60;

            peer_text.push(Line::from(vec![
                Span::styled(
                    format!("PEER #{}: ", i + 1),
                    Style::default().fg(EVA_YELLOW).add_modifier(Modifier::BOLD),
                ),
                Span::styled(peer_id.to_string(), Style::default().fg(EVA_FOREGROUND)),
            ]));

            peer_text.push(Line::from(vec![
                Span::styled("  ADDRESS: ", Style::default().fg(EVA_TEAL)),
                Span::styled(
                    info.address.to_string(),
                    Style::default().fg(EVA_FOREGROUND),
                ),
            ]));

            peer_text.push(Line::from(vec![
                Span::styled("  CONNECTED: ", Style::default().fg(EVA_TEAL)),
                Span::styled(
                    format!("{:02}:{:02}:{:02}", hours, minutes, seconds),
                    Style::default().fg(EVA_FOREGROUND),
                ),
            ]));

            peer_text.push(Line::from(vec![
                Span::styled("  ROLE: ", Style::default().fg(EVA_TEAL)),
                Span::styled(
                    if info.is_relay { "RELAY" } else { "PEER" },
                    Style::default()
                        .fg(if info.is_relay { EVA_ORANGE } else { EVA_BLUE })
                        .add_modifier(Modifier::BOLD),
                ),
            ]));

            // Add a separator between peers
            if i < peers.len() - 1 {
                peer_text.push(Line::from(""));
                peer_text.push(Line::from(vec![Span::styled(
                    "----------------------------------------",
                    Style::default().fg(Color::DarkGray),
                )]));
                peer_text.push(Line::from(""));
            }
        }

        let paragraph = Paragraph::new(peer_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(EVA_ORANGE))
                    .title(" NETWORK PEERS ")
                    .title_alignment(Alignment::Center),
            )
            .scroll((0, 0))
            .wrap(Wrap { trim: true });

        frame.render_widget(paragraph, animated_area);
    }
}
