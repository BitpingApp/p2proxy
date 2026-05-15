use std::{io::Stdout, time::Duration};

use ratatui::{
    layout::{Constraint, Direction, Layout},
    prelude::CrosstermBackend,
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Paragraph},
    Frame, Terminal,
};
use tokio::time::interval;
use tui_components::theme;

use super::Ui;

const SPLASH_DURATION: Duration = Duration::from_millis(2500);
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Figlet-style "p2proxy" logo. Matches the shape of bitpingd's splash
/// (clean slant figlet, no decorative borders) so the two apps feel like
/// one product when launched side-by-side.
const LOGO: &str = r#"          ___
    ____ |__ \ ____  _________  _  ____  __
   / __ \__/ // __ \/ ___/ __ \| |/_/ / / /
  / /_/ / __// /_/ / /  / /_/ />  </ /_/ /
 / .___/____/ .___/_/   \____/_/|_|\__, /
/_/        /_/                    /____/"#;

const LOGO_MIN_WIDTH: u16 = 50;
const SHIMMER_BAND: i32 = 10;
const SHIMMER_SPEED: i32 = 1;

impl Ui {
    pub async fn run_splash_screen_animation(
        &self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) {
        let mut ticker = interval(Duration::from_millis(33));
        let mut frame_no: usize = 0;
        loop {
            let _ = ticker.tick().await;
            let done = self.splash_start_time.elapsed() >= SPLASH_DURATION;
            let _ = terminal.draw(|f| draw_loading(f, frame_no));
            if done {
                break;
            }
            frame_no = frame_no.wrapping_add(1);
        }
    }
}

fn draw_loading(frame: &mut Frame<'_>, frame_no: usize) {
    let area = frame.area();

    frame.render_widget(Block::default().style(Style::new().bg(theme::BG)), area);

    if area.width < LOGO_MIN_WIDTH || area.height < (LOGO.lines().count() as u16) + 5 {
        let dots = ".".repeat(((frame_no / 12) % 3) + 1);
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1), Constraint::Min(0)])
            .split(area);
        let line = Line::from(Span::styled(
            format!("p2proxy v{VERSION}  loading{dots}"),
            Style::new().fg(theme::WARNING).bg(theme::BG),
        ))
        .centered();
        frame.render_widget(
            Paragraph::new(vec![line]).style(Style::new().bg(theme::BG)),
            layout[1],
        );
        return;
    }

    let logo_height: u16 = (LOGO.lines().count() as u16) + 4;
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(logo_height),
            Constraint::Min(0),
        ])
        .split(area);

    let logo_width = LOGO.lines().map(|l| l.chars().count()).max().unwrap_or(0);
    let mut lines: Vec<Line> = LOGO
        .lines()
        .map(|l| build_shimmer_line(l, logo_width, frame_no))
        .collect();
    lines.push(Line::from(""));
    lines.push(
        Line::from(Span::styled(
            format!("v{VERSION}"),
            Style::new().fg(theme::DIM).bg(theme::BG),
        ))
        .centered(),
    );

    frame.render_widget(
        Paragraph::new(lines).style(Style::new().bg(theme::BG)),
        layout[1],
    );
}

fn build_shimmer_line(line_text: &str, logo_width: usize, frame_no: usize) -> Line<'static> {
    let pad_right = logo_width.saturating_sub(line_text.chars().count());
    let padded: Vec<char> = line_text
        .chars()
        .chain(std::iter::repeat(' ').take(pad_right))
        .collect();
    let spans: Vec<Span> = padded
        .iter()
        .enumerate()
        .map(|(x, &c)| {
            if c == ' ' {
                Span::styled(c.to_string(), Style::new().bg(theme::BG))
            } else {
                let intensity = shimmer_intensity(x, logo_width, frame_no);
                Span::styled(c.to_string(), shimmer_style(intensity))
            }
        })
        .collect();
    Line::from(spans).centered()
}

fn shimmer_intensity(x: usize, total_width: usize, frame_no: usize) -> f32 {
    let cycle = (total_width as i32) + (SHIMMER_BAND * 2);
    let raw = ((frame_no as i32) * SHIMMER_SPEED) % cycle;
    let peak = raw - SHIMMER_BAND;
    let dist = (x as i32 - peak).abs();
    if dist >= SHIMMER_BAND {
        0.0
    } else {
        let t = 1.0 - (dist as f32 / SHIMMER_BAND as f32);
        t * t
    }
}

fn shimmer_style(intensity: f32) -> Style {
    let base = (0x93u8, 0x52u8, 0xffu8);
    let peak = (0x29u8, 0xffu8, 0x98u8);
    let fg = Color::Rgb(
        lerp_u8(base.0, peak.0, intensity),
        lerp_u8(base.1, peak.1, intensity),
        lerp_u8(base.2, peak.2, intensity),
    );
    Style::new().fg(fg).bg(theme::BG).bold()
}

fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    let af = a as f32;
    let bf = b as f32;
    (af + (bf - af) * t.clamp(0.0, 1.0)) as u8
}
