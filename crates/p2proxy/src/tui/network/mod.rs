use libp2p::{Multiaddr, multiaddr::Protocol};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap},
};

use super::{ACCENT, BACKGROUND, BORDER, FOREGROUND, PRIMARY, SECONDARY, SUCCESS, Ui, WARN};
use crate::tui::ui_state::{AddrSource, PeerAddr};

impl Ui {
    /// Per-server breakdown of the proxy fleet. One block per server in
    /// `Config.yaml`. The selected server is highlighted; expanded
    /// servers (toggle with Enter/Space) show the rotation pool table
    /// below their header. Collapsed servers are rendered as a single
    /// summary line, leaving more vertical space for the expanded
    /// ones.
    ///
    /// Navigation: Up/Down or j/k cycles the selection; Enter or Space
    /// toggles expand/collapse. Defaults to "first server expanded,
    /// rest collapsed" so the tab is informative on first open without
    /// being overwhelming on configs with many servers.
    pub(crate) fn render_network_tab(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let servers = self.config.servers.clone();
        if servers.is_empty() {
            let placeholder = Paragraph::new(Line::from(Span::styled(
                "no servers configured — add entries to Config.yaml",
                Style::default().fg(WARN),
            )))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(BORDER))
                    .title(Span::styled(
                        " NETWORK ",
                        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                    )),
            )
            .style(Style::default().bg(BACKGROUND));
            frame.render_widget(placeholder, area);
            return;
        }

        // Constraint per server: collapsed → 1-line summary
        // (Length(3) for borders+row), expanded → take remaining
        // proportionally (Min(8) floor so it actually shows the table).
        let constraints: Vec<Constraint> = servers
            .iter()
            .map(|s| {
                if self.network_expanded.contains(&s.port) {
                    Constraint::Min(8)
                } else {
                    Constraint::Length(3)
                }
            })
            .collect();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(area);

        for (i, server) in servers.iter().enumerate() {
            let is_selected = i == self.network_selected_idx;
            let is_expanded = self.network_expanded.contains(&server.port);
            if is_expanded {
                self.render_server_block_expanded(frame, chunks[i], server, is_selected);
            } else {
                self.render_server_block_collapsed(frame, chunks[i], server, is_selected);
            }
        }
    }

    fn render_server_block_expanded(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        server: &proxy_core::config::Server,
        is_selected: bool,
    ) {
        // Pinned servers render one status row per rank inside the header,
        // so it grows with the preference list; discovery servers keep the
        // fixed 5-row header.
        let pinned_rows = self
            .state
            .pinned_statuses
            .get(&server.port)
            .map(|s| s.len() as u16)
            .unwrap_or(0);
        let header_height = if pinned_rows > 0 { 5 + pinned_rows } else { 5 };
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(header_height), Constraint::Min(0)])
            .split(area);

        self.render_server_header(frame, chunks[0], server, is_selected);
        self.render_server_pool_table(frame, chunks[1], server);
    }

    /// Compact one-line summary used when a server isn't expanded.
    /// Encodes everything the operator typically wants at a glance:
    /// caret indicating selection, expansion state (▾ open / ▸ closed),
    /// port, country filter, active-peer presence, pool size.
    fn render_server_block_collapsed(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        server: &proxy_core::config::Server,
        is_selected: bool,
    ) {
        let active = self.state.active_destinations.get(&server.port);
        let pool_size = self
            .state
            .server_pools
            .get(&server.port)
            .map(|v| v.len())
            .unwrap_or(0);

        let country = server
            .peer_options
            .country
            .clone()
            .unwrap_or_else(|| "any".to_string());

        let active_label = match active {
            Some(_) => Span::styled(
                "active",
                Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD),
            ),
            None => Span::styled("idle", Style::default().fg(WARN)),
        };

        let port_span = Span::styled(
            format!(":{}", server.port),
            Style::default()
                .fg(if is_selected { ACCENT } else { PRIMARY })
                .add_modifier(Modifier::BOLD),
        );

        let line = Line::from(vec![
            Span::styled(
                if is_selected { " ▸ " } else { "   " },
                Style::default().fg(ACCENT),
            ),
            port_span,
            Span::raw("  "),
            Span::styled(
                format!("country: {country}"),
                Style::default().fg(FOREGROUND),
            ),
            Span::raw("  ·  "),
            active_label,
            Span::raw("  ·  "),
            Span::styled(format!("pool: {pool_size}"), Style::default().fg(ACCENT)),
            Span::raw("  ·  "),
            Span::styled("[Enter] expand", Style::default().fg(BORDER)),
        ]);

        let border_color = if is_selected { ACCENT } else { BORDER };
        let paragraph = Paragraph::new(line)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(border_color)),
            )
            .style(Style::default().bg(BACKGROUND));
        frame.render_widget(paragraph, area);
    }

    fn render_server_header(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        server: &proxy_core::config::Server,
        is_selected: bool,
    ) {
        // Filter summary line: country / min_bandwidth. `min_bandwidth`
        // is a human_bandwidth value with a tidy Display impl, so we
        // just format with `:#}` for the SI-suffixed form (10 Mbps).
        let mut filter_parts: Vec<String> = Vec::new();
        if let Some(ref country) = server.peer_options.country {
            filter_parts.push(format!("country: {country}"));
        }
        // `Bandwidth` is a `human_bandwidth::Bandwidth` — no Display
        // impl, so format the bits-per-second manually. Use bps for
        // the smallest tier (matches Config.yaml's "100mbps" syntax).
        let bps = server.peer_options.min_bandwidth.as_bps();
        let bw_str = if bps >= 1_000_000_000 {
            format!("{:.0}Gbps", bps as f64 / 1_000_000_000.0)
        } else if bps >= 1_000_000 {
            format!("{:.0}Mbps", bps as f64 / 1_000_000.0)
        } else if bps >= 1_000 {
            format!("{:.0}Kbps", bps as f64 / 1_000.0)
        } else {
            format!("{bps}bps")
        };
        filter_parts.push(format!("min bw: {bw_str}"));
        let pinned = server.peer_options.pinned();
        if !pinned.is_empty() {
            filter_parts.push(format!(
                "pinned: {} peer{}",
                pinned.len(),
                if pinned.len() == 1 { "" } else { "s" }
            ));
        } else if server.peer_options.sticky {
            filter_parts.push("sticky".to_string());
        }
        let filter_text = if filter_parts.is_empty() {
            "no filters".to_string()
        } else {
            filter_parts.join("  ·  ")
        };

        // Active-destination summary: full PeerId (so it's verifiably the
        // same id persisted in sticky_peers.json), plus total bytes
        // attributed to that peer so far and how it was selected
        // (pinned rank / sticky / discovered).
        let active_line = match self.state.active_destinations.get(&server.port) {
            Some(peer_id) => {
                let bytes = self
                    .state
                    .peer_bandwidth
                    .get(peer_id)
                    .copied()
                    .unwrap_or((0, 0));
                let mut spans = vec![
                    Span::styled("active: ", Style::default().fg(BORDER)),
                    Span::styled(
                        fit_peer_id(
                            &peer_id.to_string(),
                            (area.width as usize).saturating_sub(56),
                        ),
                        Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD),
                    ),
                ];
                if let Some(source) = self.state.destination_sources.get(&server.port) {
                    let badge = match source {
                        proxy_core::events::DestinationSource::Pinned { rank } => {
                            format!("pinned[{rank}]")
                        }
                        proxy_core::events::DestinationSource::Sticky => "sticky".to_string(),
                        proxy_core::events::DestinationSource::Discovered => "discovered".to_string(),
                    };
                    spans.push(Span::styled("  ·  ", Style::default().fg(BORDER)));
                    spans.push(Span::styled(badge, Style::default().fg(PRIMARY)));
                }
                spans.push(Span::styled("  ·  ", Style::default().fg(BORDER)));
                spans.push(Span::styled(
                    format!("↑ {}  ↓ {}", human_bytes(bytes.0), human_bytes(bytes.1)),
                    Style::default().fg(ACCENT),
                ));
                Line::from(spans)
            }
            None => Line::from(vec![
                Span::styled("active: ", Style::default().fg(BORDER)),
                Span::styled(
                    "discovering…",
                    Style::default().fg(WARN).add_modifier(Modifier::BOLD),
                ),
            ]),
        };

        let pool_size = self
            .state
            .server_pools
            .get(&server.port)
            .map(|v| v.len())
            .unwrap_or(0);

        let mut lines = vec![
            Line::from(vec![Span::styled(
                filter_text,
                Style::default().fg(FOREGROUND),
            )]),
            Line::from(""),
            active_line,
        ];
        // Pinned servers list every rank with its health; discovery
        // servers show the candidate-pool size instead.
        if let Some(statuses) = self
            .state
            .pinned_statuses
            .get(&server.port)
            .filter(|s| !s.is_empty())
        {
            let active = self.state.active_destinations.get(&server.port);
            for status in statuses {
                let (badge, badge_style) = if Some(&status.peer_id) == active {
                    (
                        "active",
                        Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD),
                    )
                } else if status.resolvable {
                    ("ok", Style::default().fg(ACCENT))
                } else {
                    (
                        "STALE",
                        Style::default().fg(WARN).add_modifier(Modifier::BOLD),
                    )
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("  [{}] ", status.rank), Style::default().fg(BORDER)),
                    Span::styled(
                        fit_peer_id(
                            &status.peer_id.to_string(),
                            (area.width as usize).saturating_sub(18),
                        ),
                        Style::default().fg(FOREGROUND),
                    ),
                    Span::styled("  ", Style::default()),
                    Span::styled(badge, badge_style),
                ]));
            }
        } else {
            lines.push(Line::from(vec![
                Span::styled("pool: ", Style::default().fg(BORDER)),
                Span::styled(
                    format!(
                        "{pool_size} candidate{}",
                        if pool_size == 1 { "" } else { "s" }
                    ),
                    Style::default().fg(ACCENT),
                ),
            ]));
        }

        let title = format!(
            " ▾ :{}  ({:?}) {} ",
            server.port,
            server.protocol,
            if is_selected {
                "· [Enter] collapse"
            } else {
                ""
            }
        );
        let border_color = if is_selected { ACCENT } else { BORDER };
        let paragraph = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(border_color))
                    .title(Span::styled(
                        title,
                        Style::default()
                            .fg(if is_selected { ACCENT } else { PRIMARY })
                            .add_modifier(Modifier::BOLD),
                    )),
            )
            .style(Style::default().bg(BACKGROUND))
            .wrap(Wrap { trim: true });
        frame.render_widget(paragraph, area);
    }

    fn render_server_pool_table(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        server: &proxy_core::config::Server,
    ) {
        let pool = self
            .state
            .server_pools
            .get(&server.port)
            .cloned()
            .unwrap_or_default();

        let active = self.state.active_destinations.get(&server.port).copied();

        let header = Row::new(
            ["", "Peer ID", "Address", "Status", "↑ bytes", "↓ bytes"]
                .iter()
                .map(|h| {
                    Cell::from(*h).style(Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD))
                }),
        )
        .height(1)
        .bottom_margin(1);

        let rows: Vec<Row> = if pool.is_empty() {
            vec![
                Row::new(vec![
                    Cell::from("—"),
                    Cell::from("—"),
                    Cell::from("—"),
                    Cell::from("waiting for FindNodes response…")
                        .style(Style::default().fg(SECONDARY)),
                    Cell::from("—"),
                    Cell::from("—"),
                ])
                .height(1),
            ]
        } else {
            pool.into_iter()
                .map(|peer_id| {
                    let is_active = active == Some(peer_id);
                    let bytes = self
                        .state
                        .peer_bandwidth
                        .get(&peer_id)
                        .copied()
                        .unwrap_or((0, 0));
                    let (marker, status_label, status_style) = if is_active {
                        (
                            "▶",
                            "active",
                            Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD),
                        )
                    } else {
                        ("·", "standby", Style::default().fg(BORDER))
                    };
                    let id_style = if is_active {
                        Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(FOREGROUND)
                    };
                    let (addr_text, addr_style) =
                        addr_cell(self.state.peer_addresses.get(&peer_id), is_active);
                    Row::new(vec![
                        Cell::from(marker).style(status_style),
                        Cell::from(peer_id.to_string()).style(id_style),
                        Cell::from(addr_text).style(addr_style),
                        Cell::from(status_label).style(status_style),
                        Cell::from(human_bytes(bytes.0)).style(Style::default().fg(ACCENT)),
                        Cell::from(human_bytes(bytes.1)).style(Style::default().fg(ACCENT)),
                    ])
                    .height(1)
                })
                .collect()
        };

        // Peer ID gets a 52-col floor so the full base58 id never
        // truncates; the byte/status columns are kept tight so the floor
        // is satisfied on narrower terminals before the id has to clip.
        let widths = [
            Constraint::Length(2),
            Constraint::Min(52),
            Constraint::Min(20),
            Constraint::Length(9),
            Constraint::Length(10),
            Constraint::Length(10),
        ];
        let table = Table::new(rows, widths)
            .header(header)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(BORDER))
                    .title(Span::styled(" rotation pool ", Style::default().fg(ACCENT))),
            )
            .style(Style::default().bg(BACKGROUND).fg(FOREGROUND));
        frame.render_widget(table, area);
    }
}

/// Fit a peer id into `max` columns for the header summary lines, keeping
/// the head and tail so it stays recognisable when the terminal is too
/// narrow for the full 52-char id. Returns the full id whenever it fits,
/// so wide terminals show it in full and the header line never wraps.
fn fit_peer_id(id: &str, max: usize) -> String {
    if id.len() <= max {
        return id.to_string();
    }
    if max <= 1 {
        return id.chars().take(max).collect();
    }
    // base58 peer ids are ASCII, so byte slicing is on char boundaries.
    let keep = max - 1;
    let head = keep.div_ceil(2);
    let tail = keep - head;
    format!("{}…{}", &id[..head], &id[id.len() - tail..])
}

/// Rotation-pool Address cell. The active peer's confirmed direct route
/// is the headline — its real egress IP, bright. Standby peers show a
/// dimmer last-known/advertised address; peers we only reach through a
/// relay (or whose address carries no host) show `—` rather than a
/// misleading relay host or raw multiaddr.
fn addr_cell(entry: Option<&PeerAddr>, is_active: bool) -> (String, Style) {
    let dash = ("—".to_string(), Style::default().fg(BORDER));
    let Some(entry) = entry else {
        return dash;
    };
    // Only an extractable IP/DNS host is this peer's egress. A relay
    // circuit's host belongs to the relay, and a bare /p2p/<id> has none —
    // both yield None and fall through to the relayed/dash arms.
    match (entry.source, multiaddr_host(&entry.addr)) {
        (AddrSource::Direct, Some(host)) => {
            let style = if is_active {
                Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(FOREGROUND)
            };
            (host, style)
        }
        (AddrSource::Candidate, Some(host)) => (host, Style::default().fg(SECONDARY)),
        (AddrSource::Relayed, _) => (
            "relayed".to_string(),
            Style::default().fg(if is_active { WARN } else { BORDER }),
        ),
        _ => dash,
    }
}

/// Compact `host[:port]` for display — the IP/DNS host and transport port
/// pulled out of a multiaddr. Returns `None` for relay-circuit routes
/// (the host there is the relay's, not this peer's) and for addresses
/// with no host at all, so the caller can render `—` instead of noise.
fn multiaddr_host(addr: &Multiaddr) -> Option<String> {
    let mut host: Option<String> = None;
    let mut port: Option<u16> = None;
    for proto in addr.iter() {
        match proto {
            Protocol::P2pCircuit => return None,
            Protocol::Ip4(ip) => host = Some(ip.to_string()),
            Protocol::Ip6(ip) => host = Some(format!("[{ip}]")),
            Protocol::Dns(h) | Protocol::Dns4(h) | Protocol::Dns6(h) | Protocol::Dnsaddr(h) => {
                host = Some(h.to_string());
            }
            Protocol::Tcp(p) | Protocol::Udp(p) => port = Some(p),
            _ => {}
        }
    }
    let host = host?;
    Some(match port {
        Some(p) => format!("{host}:{p}"),
        None => host,
    })
}

/// Pretty-print bytes with SI suffix. Tight 10-char-or-so output that
/// fits the per-peer columns without truncation.
fn human_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut idx = 0;
    while value >= 1024.0 && idx < UNITS.len() - 1 {
        value /= 1024.0;
        idx += 1;
    }
    format!("{value:.1} {}", UNITS[idx])
}
