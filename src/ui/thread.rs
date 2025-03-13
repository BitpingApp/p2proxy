use super::run;

pub fn run_thread() {
    let terminal = ratatui::init();

    // // Start the UI in a separate task
    let ui_handle = tokio::spawn(async move {
        let result = run(terminal);
        ratatui::restore();
        result
    });
}
