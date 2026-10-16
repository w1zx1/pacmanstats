mod app;
mod logparse;
mod model;

use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "pacmanstats", about = "TUI browser for /var/log/pacman.log by day")]
struct Args {
    /// Path to pacman.log
    #[arg(short, long, default_value = "/var/log/pacman.log")]
    log: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let text =
        std::fs::read_to_string(&args.log).with_context(|| format!("cannot read {}", args.log.display()))?;
    let events = logparse::parse_log(&text);
    if events.is_empty() {
        eprintln!("no ALPM events found in {}", args.log.display());
    }
    let days = model::group_by_day(events);
    app::run(days, &args.log)?;
    Ok(())
}
