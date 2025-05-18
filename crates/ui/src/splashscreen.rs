use std::{io::Stdout, time::Duration};

use ratatui::{
    layout::{Alignment, Rect},
    prelude::CrosstermBackend,
    style::{Color, Style},
    widgets::{Clear, Paragraph},
    Frame, Terminal,
};
use tokio::time::interval;

use super::{Ui, BACKGROUND, BORDER, FOREGROUND, SUCCESS};

const SPLASH_DURATION: Duration = Duration::from_secs(3);
const SPLASH_LOGO: &str = r#"
  ·  · ˚  *  ˚  ·  * ˚  *  ˚  ·  * ˚  *  ˚  ·  * ˚  *  ˚  ·  * ˚  *  ˚  ·  * ˚  *  ˚  ·  * ˚  *  ˚  ·  * ˚  *  ˚  ·  
 * ░▒▓███████▓▒░ ˚ ░▒▓███████▓▒░ ˚ ░▒▓███████▓▒░ ˚ ░▒▓███████▓▒░ ·░▒▓██████▓▒░ ˚░▒▓█▓▒ ˚▒▓█▓▒░ ░▒▓█▓▒ ˚▒▓█▓▒░ *
 · ░▒▓█▓▒ ˚·▒▓█▓▒░ ˚ · ˚˚  ░▒▓█▓▒░ ░▒▓█▓▒ ˚▒▓█▓▒░ ░▒▓█▓▒ ˚·▒▓█▓▒░░▒▓█▓▒ ˚▒▓█▓▒░ ░▒▓█▓▒ ˚▒▓█▓▒░ ░▒▓█▓▒ ˚▒▓█▓▒░ ·
 · ░▒▓█▓▒ ˚▒▓█▓▒░ ˚· · ˚˚˚ ░▒▓█▓▒░ ░▒▓█▓▒ ˚▒▓█▓▒░ ░▒▓█▓▒ ˚ ▒▓█▓▒░░▒▓█▓▒ ˚▒▓█▓▒░ ░▒▓█▓▒ ˚▒▓█▓▒░ ░▒▓█▓▒ ˚▒▓█▓▒░ ·
 ˚  ░▒▓███████▓▒░ · · ░▒▓██████▓▒░ ˚░▒▓███████▓▒░ ˚░▒▓███████▓▒ · ░▒▓█▓▒ ˚▒▓█▓▒░ ˚░▒▓██████▓▒░ · ░▒▓██████▓▒░ ˚ ˚
 · ░▒▓█▓▒░ · · · ˚ ░▒▓█▓▒░ · · · ˚ ░▒▓█▓▒░ · · ·  ░▒▓█▓▒ ˚▒▓█▓▒░ ░▒▓█▓▒ ˚▒▓█▓▒░ ░▒▓█▓▒ ˚▒▓█▓▒░ · ˚░▒▓█▓▒░ ˚ · ·
 · ░▒▓█▓▒░ · · · · ░▒▓█▓▒░ · · · ˚ ░▒▓█▓▒░ · · ·˚ ░▒▓█▓▒ ˚▒▓█▓▒░ ░▒▓█▓▒ ˚▒▓█▓▒░ ░▒▓█▓▒ ˚▒▓█▓▒░ · ˚░▒▓█▓▒░ ˚ · ·
 · ░▒▓█▓▒░ · · · · ░▒▓████████▓▒░  ░▒▓█▓▒░· · · ˚ ░▒▓█▓▒ ˚▒▓█▓▒░ ˚░▒▓██████▓▒░ ˚░▒▓█▓▒ ˚▒▓█▓▒░ · ˚░▒▓█▓▒░ ˚ · ·
  ·  · ˚  *  ˚  ·  * ˚  *  ˚  ·  * ˚  *  ˚  ·  * ˚  *  ˚  ·  * ˚  *  ˚  ·  * ˚  *  ˚  ·  * ˚  *  ˚  ·  * ˚  *  ˚  ·  
"#;

const WORLD_MAP: &str = r#"    
⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢀⣀⣄⣠⣀⡀⣀⣠⣤⣤⣤⣀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣄⢠⣠⣼⣿⣿⣿⣟⣿⣿⣿⣿⣿⣿⣿⣿⡿⠋⠀⠀⠀⢠⣤⣦⡄⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠰⢦⣄⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
⠀⠀⠀⠀⠀⠀⠀⠀⣼⣿⣟⣾⣿⣽⣿⣿⣅⠈⠉⠻⣿⣿⣿⣿⣿⡿⠇⠀⠀⠀⠀⠀⠉⠀⠀⠀⠀⠀⢀⡶⠒⢉⡀⢠⣤⣶⣶⣿⣷⣆⣀⡀⠀⢲⣖⠒⠀⠀⠀⠀⠀⠀⠀
⢀⣤⣾⣶⣦⣤⣤⣶⣿⣿⣿⣿⣿⣿⣽⡿⠻⣷⣀⠀⢻⣿⣿⣿⡿⠟⠀⠀⠀⠀⠀⠀⣤⣶⣶⣤⣀⣀⣬⣷⣦⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣶⣦⣤⣦⣼⣀⠀
⠈⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡿⠛⠓⣿⣿⠟⠁⠘⣿⡟⠁⠀⠘⠛⠁⠀⠀⢠⣾⣿⢿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡿⠏⠙⠁
⠀⠸⠟⠋⠀⠈⠙⣿⣿⣿⣿⣿⣿⣷⣦⡄⣿⣿⣿⣆⠀⠀⠀⠀⠀⠀⠀⠀⣼⣆⢘⣿⣯⣼⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡉⠉⢱⡿⠀⠀⠀⠀⠀
⠀⠀⠀⠀⠀⠀⠀⠘⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣟⡿⠦⠀⠀⠀⠀⠀⠀⠀⠙⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡿⡗⠀⠈⠀⠀⠀⠀⠀⠀
⠀⠀⠀⠀⠀⠀⠀⠀⢻⣿⣿⣿⣿⣿⣿⣿⣿⠋⠁⠀⠀⠀⠀⠀⠀⠀⠀⠀⢿⣿⣉⣿⡿⢿⢷⣾⣾⣿⣞⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⠋⣠⠟⠀⠀⠀⠀⠀⠀⠀⠀
⠀⠀⠀⠀⠀⠀⠀⠀⠀⠹⣿⣿⣿⠿⠿⣿⠁⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣀⣾⣿⣿⣷⣦⣶⣦⣼⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣷⠈⠛⠁⠀⠀⠀⠀⠀⠀⠀⠀⠀
⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠉⠻⣿⣤⡖⠛⠶⠤⡀⠀⠀⠀⠀⠀⠀⠀⢰⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡿⠁⠙⣿⣿⠿⢻⣿⣿⡿⠋⢩⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠈⠙⠧⣤⣦⣤⣄⡀⠀⠀⠀⠀⠀⠘⢿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡇⠀⠀⠀⠘⣧⠀⠈⣹⡻⠇⢀⣿⡆⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢠⣿⣿⣿⣿⣿⣤⣀⡀⠀⠀⠀⠀⠀⠀⠈⢽⣿⣿⣿⣿⣿⠋⠀⠀⠀⠀⠀⠀⠀⠀⠹⣷⣴⣿⣷⢲⣦⣤⡀⢀⡀⠀⠀⠀⠀⠀⠀
⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠈⢿⣿⣿⣿⣿⣿⣿⠟⠀⠀⠀⠀⠀⠀⠀⢸⣿⣿⣿⣿⣷⢀⡄⠀⠀⠀⠀⠀⠀⠀⠀⠈⠉⠂⠛⣆⣤⡜⣟⠋⠙⠂⠀⠀⠀⠀⠀
⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢹⣿⣿⣿⣿⠟⠀⠀⠀⠀⠀⠀⠀⠀⠘⣿⣿⣿⣿⠉⣿⠃⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣤⣾⣿⣿⣿⣿⣆⠀⠰⠄⠀⠉⠀⠀
⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣸⣿⣿⡿⠃⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢹⣿⡿⠃⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢻⣿⠿⠿⣿⣿⣿⠇⠀⠀⢀⠀⠀⠀
⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢀⣿⡿⠛⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠈⢻⡇⠀⠀⢀⣼⠗⠀⠀
⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢸⣿⠃⣀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠙⠁⠀⠀⠀
⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠙⠒⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
"#;

impl Ui {
    pub async fn run_splash_screen_animation(
        &self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) {
        let mut interval = interval(Duration::from_millis(16));
        let mut done = false;

        while !done {
            let _tick = interval.tick().await;
            terminal.draw(|f| {
                done = self.render_splash_screen(f);
            });
        }
    }

    fn render_splash_screen(&self, frame: &mut Frame<'_>) -> bool {
        let area = frame.area();
        let elapsed = self.splash_start_time.elapsed();
        let progress = elapsed
            .as_millis()
            .saturating_mul(100)
            .checked_div(SPLASH_DURATION.as_millis())
            .unwrap_or(0)
            .min(100) as u16;

        // Calculate fade-in and breathing effect
        let fade_duration = Duration::from_millis(1000);
        let breath_period = Duration::from_millis(2000);

        let opacity = if elapsed <= fade_duration {
            ((elapsed.as_millis() as f64 / fade_duration.as_millis() as f64) * 255.0) as u8
        } else {
            255
        };

        let breath_alpha = if elapsed > fade_duration {
            let breath_time = (elapsed - fade_duration).as_millis() as f64;
            let breath_cycle =
                (breath_time / breath_period.as_millis() as f64 * std::f64::consts::PI * 2.0).sin();
            0.8 + (breath_cycle * 0.2).max(0.0)
        } else {
            1.0
        };

        let current_orange = Self::apply_opacity(BORDER, opacity, breath_alpha);

        let current_green = Self::apply_opacity(SUCCESS, opacity, breath_alpha);
        let current_foreground = Self::apply_opacity(FOREGROUND, opacity, breath_alpha);

        // Calculate logo dimensions dynamically
        let logo_lines: Vec<&str> = SPLASH_LOGO.lines().collect();
        let logo_height = logo_lines.len();
        let logo_width = logo_lines
            .iter()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(0);

        // Create a centered area for the splash logo
        let logo_area = Rect::new(
            (area.width.saturating_sub(logo_width as u16)) / 2,
            (area.height.saturating_sub(logo_height as u16)) / 2,
            logo_width.min(area.width as usize) as u16,
            logo_height.min(area.height as usize) as u16,
        );

        // Render the NERV logo
        let splash_paragraph = Paragraph::new(SPLASH_LOGO)
            .style(Style::default().fg(current_orange).bg(BACKGROUND))
            .alignment(Alignment::Center);

        frame.render_widget(Clear, area);
        frame.render_widget(splash_paragraph, logo_area);

        // Add the world map instead of a progress bar
        let map_lines: Vec<&str> = WORLD_MAP.lines().collect();
        let map_height = map_lines.len();
        let map_width = map_lines
            .iter()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(0);

        let map_area = Rect::new(
            (area.width.saturating_sub(map_width as u16)) / 2,
            logo_area
                .bottom()
                .saturating_add(2)
                .min(area.height.saturating_sub(map_height as u16)),
            map_width.min(area.width as usize) as u16,
            map_height.min(area.height as usize) as u16,
        );

        // Create a partially revealed world map based on progress
        let revealed_map = Self::reveal_map(WORLD_MAP, progress);

        let map_paragraph = Paragraph::new(revealed_map)
            .style(Style::default().fg(current_green).bg(BACKGROUND))
            .alignment(Alignment::Center);

        frame.render_widget(map_paragraph, map_area);

        // Add a status message
        let status_area = Rect::new(
            (area.width.saturating_sub(map_width as u16)) / 2,
            map_area
                .bottom()
                .saturating_add(1)
                .min(area.height.saturating_sub(1)),
            map_width.min(area.width as usize) as u16,
            1,
        );

        let status_text = match progress {
            0..=25 => "Initializing Bitping Swarm...",
            26..=50 => "Loading proxy protocols...",
            51..=75 => "Establishing network connections...",
            _ => "Ready to launch...",
        };

        let status_paragraph = Paragraph::new(status_text)
            .style(Style::default().fg(current_foreground))
            .alignment(Alignment::Center);

        frame.render_widget(status_paragraph, status_area);

        elapsed >= SPLASH_DURATION
    }

    // Helper function to create a scanning effect on the world map
    fn reveal_map(map: &str, progress: u16) -> String {
        let lines: Vec<&str> = map.lines().collect();

        // Find the maximum line width
        let max_width = lines
            .iter()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(0);

        // Calculate how many characters to reveal horizontally
        let chars_to_reveal = ((progress as f64 / 100.0) * max_width as f64) as usize;

        let mut result = String::new();

        for line in lines {
            let mut line_result = String::new();
            for (i, ch) in line.chars().enumerate() {
                if i < chars_to_reveal {
                    line_result.push(ch);
                } else {
                    // Replace with space to maintain layout but hide content
                    if ch.is_whitespace() {
                        line_result.push(ch);
                    } else {
                        line_result.push(' ');
                    }
                }
            }
            result.push_str(&line_result);
            result.push('\n');
        }

        result
    }

    fn apply_opacity(color: Color, opacity: u8, breath_alpha: f64) -> Color {
        match color {
            Color::Rgb(r, g, b) => {
                let factor = (opacity as f64 / 255.0) * breath_alpha;
                Color::Rgb(
                    (r as f64 * factor) as u8,
                    (g as f64 * factor) as u8,
                    (b as f64 * factor) as u8,
                )
            }
            _ => color,
        }
    }
}
