pub mod rewrite;
pub mod persistence;
pub mod scrubber;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "scrobble-scrubber")]
#[command(about = "Automated Last.fm track monitoring and scrubbing system")]
pub struct Args {
    /// Check interval in seconds
    #[arg(short, long, default_value = "300")]
    pub interval: u64,

    /// Maximum number of tracks to check per run
    #[arg(short, long, default_value = "100")]
    pub max_tracks: usize,

    /// Dry run mode - don't actually make any edits
    #[arg(long)]
    pub dry_run: bool,

    /// Path to state file for persistence
    #[arg(short, long, default_value = "scrobble_state.db")]
    pub state_file: String,
}