use ratatui::{
    prelude::*,
    widgets::{Block, Borders},
};

use super::{Ui, BORDER, ERROR, MISC, PRIMARY, SECONDARY, WARN};

impl Ui {
    pub(crate) fn render_logs_tab(&self, frame: &mut Frame<'_>, area: Rect) {
        // Create the TuiLoggerWidget with Evangelion-inspired styling
        let tui_widget = tui_logger::TuiLoggerWidget::default()
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(BORDER))
                    .title(" SYSTEM LOGS ")
                    .title_alignment(Alignment::Center),
            )
            .style_error(Style::default().fg(ERROR))
            .style_warn(Style::default().fg(WARN))
            .style_info(Style::default().fg(MISC))
            .style_debug(Style::default().fg(SECONDARY))
            .style_trace(Style::default().fg(MISC))
            .output_separator('|')
            .output_timestamp(Some("%H:%M:%S".to_string()))
            .output_target(true)
            .output_file(false)
            .output_line(false);

        // Render the widget
        frame.render_widget(tui_widget, area);
    }
}
