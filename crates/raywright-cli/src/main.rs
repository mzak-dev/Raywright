//! `raywright`: the inspector TUI, and commands for scripts and CI.

mod commands;
mod session;
#[cfg(feature = "tui")]
mod tui;

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

pub type Res<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Parser)]
#[command(name = "raywright", version, about = "Find, click, screenshot and record desktop apps")]
struct Cli {
    /// `platform` (default) or `mock` (an in-memory demo app, any OS).
    #[arg(long, global = true, default_value = "platform")]
    driver: String,
    /// A raywright.toml to use instead of looking for one.
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Cmd>,
}

/// Which app and window to work on.
#[derive(Args, Clone, Debug, Default)]
pub struct Target {
    /// Attach to the app owning a window that matches (`title=Notepad`).
    #[arg(long)]
    pub window: Option<String>,
    /// Attach to a process.
    #[arg(long)]
    pub pid: Option<u32>,
    /// Launch a `[profiles.<name>]` of raywright.toml.
    #[arg(long)]
    pub profile: Option<String>,
    /// Launch this program.
    #[arg(long)]
    pub exe: Option<PathBuf>,
    /// Arguments for `--exe`.
    #[arg(long = "arg", allow_hyphen_values = true)]
    pub args: Vec<String>,
}

#[derive(Subcommand)]
enum Cmd {
    /// The interactive inspector (also the default without a command).
    Ui {
        #[command(flatten)]
        target: Target,
        /// Open a trace.zip in the trace tab.
        #[arg(long)]
        trace: Option<PathBuf>,
    },
    /// Print a window's elements with a selector for each.
    Inspect {
        #[command(flatten)]
        target: Target,
        #[arg(long)]
        json: bool,
    },
    /// Save a picture of a window or an element.
    Screenshot {
        #[command(flatten)]
        target: Target,
        /// An element instead of the whole window.
        #[arg(long)]
        selector: Option<String>,
        /// screen, window or app.
        #[arg(long)]
        source: Option<String>,
        #[arg(long, default_value = "screenshot.png")]
        out: PathBuf,
    },
    /// Record a window: frames, manifest.json, contact sheet, GIF.
    Record {
        #[command(flatten)]
        target: Target,
        #[arg(long, default_value_t = 30.0)]
        fps: f64,
        /// Stop after this long (`2s`, `500ms`).
        #[arg(long)]
        duration: Option<String>,
        /// Stop once nothing changed for this long.
        #[arg(long)]
        until_settled: Option<String>,
        /// Step the app's animation clock instead (bridge apps): `16ms` per frame.
        #[arg(long)]
        manual_clock: Option<String>,
        #[arg(long, default_value = "recording")]
        out: PathBuf,
    },
    /// List what a trace.zip recorded; `--html` writes the report.
    ShowTrace {
        path: PathBuf,
        #[arg(long)]
        html: Option<PathBuf>,
    },
    /// The inspector, with the generated test in view.
    Codegen {
        #[command(flatten)]
        target: Target,
    },
    /// Run the MCP server (`raywright-mcp`), passing the rest on.
    Mcp {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

fn main() {
    let cli = Cli::parse();
    if let Some(c) = &cli.config {
        // SAFETY: set before any thread starts.
        unsafe { std::env::set_var("RAYWRIGHT_CONFIG", c) };
    }
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("tokio runtime");
    let code = match rt.block_on(run(cli)) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("raywright: {e}");
            1
        }
    };
    rt.shutdown_timeout(std::time::Duration::from_secs(2));
    std::process::exit(code);
}

async fn run(cli: Cli) -> Res<()> {
    let driver = cli.driver.clone();
    match cli.command {
        None => ui(&driver, Target::default(), None, false).await,
        Some(Cmd::Ui { target, trace }) => ui(&driver, target, trace, false).await,
        Some(Cmd::Codegen { target }) => ui(&driver, target, None, true).await,
        Some(Cmd::Inspect { target, json }) => commands::inspect(&driver, &target, json).await,
        Some(Cmd::Screenshot { target, selector, source, out }) => commands::screenshot(&driver, &target, selector.as_deref(), source.as_deref(), &out).await,
        Some(Cmd::Record { target, fps, duration, until_settled, manual_clock, out }) => {
            commands::record(&driver, &target, fps, duration.as_deref(), until_settled.as_deref(), manual_clock.as_deref(), &out).await
        }
        Some(Cmd::ShowTrace { path, html }) => commands::show_trace(&path, html.as_deref()),
        Some(Cmd::Mcp { args }) => commands::mcp(&cli.driver, &args),
    }
}

#[cfg(feature = "tui")]
async fn ui(driver: &str, target: Target, trace: Option<PathBuf>, code: bool) -> Res<()> {
    tui::run(driver, target, trace, code).await
}

#[cfg(not(feature = "tui"))]
async fn ui(_: &str, _: Target, _: Option<PathBuf>, _: bool) -> Res<()> {
    Err("this raywright was built without the TUI (feature `tui`)".into())
}
