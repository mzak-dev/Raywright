//! `raywright-mcp`: Raywright as an MCP server on stdio.
//!
//! ```text
//! claude mcp add raywright -- raywright-mcp --allow-exe C:\path\to\app.exe
//! ```

use std::path::PathBuf;

use clap::Parser;
use raywright::RaywrightExt;
use raywright::config::InputMode;
use raywright_mcp::{Policy, RaywrightServer};

#[derive(Parser)]
#[command(name = "raywright-mcp", version, about = "Drive desktop apps from an MCP client")]
struct Args {
    /// `platform` (default) or `mock` (an in-memory demo app, for trying it out anywhere).
    #[arg(long, default_value = "platform")]
    driver: String,
    /// A program app_launch may start (repeat for more). Profiles are always allowed.
    #[arg(long = "allow-exe")]
    allow_exe: Vec<PathBuf>,
    /// Let app_attach take over running apps.
    #[arg(long)]
    allow_attach: bool,
    /// message (default: leaves your mouse alone), os (real input) or native.
    #[arg(long, default_value = "message")]
    input_mode: String,
    /// raywright.toml with [profiles.*].
    #[arg(long)]
    profile_file: Option<PathBuf>,
    /// Where pictures, recordings and session.log go.
    #[arg(long, default_value = "raywright-mcp-output")]
    output_dir: PathBuf,
    /// Longer side of pictures sent back, px.
    #[arg(long, default_value_t = 1568)]
    max_image_px: u32,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    if let Some(p) = &args.profile_file {
        // SAFETY: set before anything reads the environment on other threads.
        unsafe { std::env::set_var("RAYWRIGHT_CONFIG", p) };
    }
    let input_mode: InputMode = match args.input_mode.parse() {
        Ok(m) => m,
        Err(e) => fail(e),
    };
    let mut allow_exe = args.allow_exe.clone();
    let builder = match args.driver.as_str() {
        "mock" => {
            allow_exe.push("demo-app".into());
            raywright::Raywright::builder().driver(raywright::mock::MockDriver::new().into_arc())
        }
        "platform" => raywright::Raywright::platform(),
        other => fail(format!("unknown driver `{other}`")),
    };
    let rw = match builder.input_mode(input_mode).build() {
        Ok(rw) => rw,
        Err(e) => fail(e),
    };
    let policy = Policy { allow_exe, allow_attach: args.allow_attach, input_mode, max_image_px: args.max_image_px, output_dir: args.output_dir };
    if let Err(e) = RaywrightServer::new(rw, policy).serve_stdio().await {
        fail(e);
    }
}

fn fail(e: impl std::fmt::Display) -> ! {
    eprintln!("raywright-mcp: {e}");
    std::process::exit(2);
}
