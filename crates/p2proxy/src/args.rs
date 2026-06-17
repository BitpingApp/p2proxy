use clap::Parser;

/// Bitping P2P proxy daemon. Everything else is configured in `Config.yaml`
/// (path below); these flags only choose the config and the UI mode.
#[derive(Parser, Debug)]
#[command(name = "p2proxy", version)]
pub struct Cli {
    /// Path to Config.yaml.
    #[arg(short, long, env = "P2PROXY_CONFIG", default_value = "Config.yaml")]
    pub config: String,

    /// Run headless without the TUI (for systemd / Docker / no TTY).
    #[arg(long, env = "NO_UI")]
    pub no_ui: bool,
}
