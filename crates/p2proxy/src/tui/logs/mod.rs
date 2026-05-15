use ratatui::prelude::*;

use super::Ui;

impl Ui {
    /// Render the logs pane using the shared `tui_components::logs`
    /// builder — keeps styling and timestamp format in lockstep with
    /// bitpingd. The `Ui::log_state` field tracks scroll position so
    /// j/k/wheel scrolling and Esc-to-follow-tail work.
    pub(crate) fn render_logs_tab(&self, frame: &mut Frame<'_>, area: Rect) {
        let widget = tui_components::logs::logs_widget(
            " logs  (j/k or wheel to scroll · Esc to follow tail) ",
            &self.log_state,
        );
        frame.render_widget(widget, area);
    }
}
