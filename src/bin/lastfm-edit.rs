use clap::Parser;
use log::LevelFilter;

mod commands;
use commands::{execute_command, utils::get_credentials, utils::load_or_create_client, Commands};

/// Last.fm scrobble metadata editor
#[derive(Parser)]
#[command(
    name = "lastfm-edit",
    about = "Last.fm scrobble metadata editor",
    long_about = None
)]
struct Cli {
    /// Increase verbosity level (use multiple times for more verbose output)
    /// -v: info for lastfm-edit, -vv: debug for lastfm-edit, -vvv: trace for lastfm-edit
    /// -vvvv: trace for lastfm-edit + info for all, -vvvvv: trace for lastfm-edit + debug for all, -vvvvvv: trace for all
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    /// Last.fm username (overrides LASTFM_EDIT_USERNAME environment variable)
    #[arg(short, long, global = true)]
    username: Option<String>,

    /// Last.fm password (overrides LASTFM_EDIT_PASSWORD environment variable)
    #[arg(short, long, global = true)]
    password: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Cli::parse();

    // Configure logging based on verbosity level (cumulative)
    let mut builder = env_logger::Builder::from_default_env();
    builder.filter_level(LevelFilter::Off); // Start with everything off

    match args.verbose {
        0 => {
            // Default: only warnings and errors for all
            builder.filter_level(LevelFilter::Warn);
        }
        1 => {
            // Info for lastfm-edit
            builder.filter_module("lastfm_edit", LevelFilter::Info);
        }
        2 => {
            // Info + Debug for lastfm-edit
            builder.filter_module("lastfm_edit", LevelFilter::Debug);
        }
        3 => {
            // Info + Debug + Trace for lastfm-edit
            builder.filter_module("lastfm_edit", LevelFilter::Trace);
        }
        4 => {
            // Trace for lastfm-edit + Info for all others
            builder.filter_module("lastfm_edit", LevelFilter::Trace);
            builder.filter_level(LevelFilter::Info);
        }
        5 => {
            // Trace for lastfm-edit + Debug for all others
            builder.filter_module("lastfm_edit", LevelFilter::Trace);
            builder.filter_level(LevelFilter::Debug);
        }
        _ => {
            // Trace for everything (6+)
            builder.filter_level(LevelFilter::Trace);
        }
    }

    builder.init();

    if args.verbose > 0 {
        log::info!("🔍 Verbose mode enabled (level {})", args.verbose);
    }

    // Get credentials from command line args or environment
    let (username, password) = if let (Some(u), Some(p)) = (&args.username, &args.password) {
        (u.clone(), p.clone())
    } else if args.username.is_some() || args.password.is_some() {
        eprintln!("❌ Error: Both username and password must be provided together");
        eprintln!("Either provide both --username and --password, or set environment variables");
        std::process::exit(1);
    } else {
        match get_credentials() {
            Ok(creds) => creds,
            Err(e) => {
                eprintln!("❌ Error: {e}");
                eprintln!();
                eprintln!("Please provide credentials via:");
                eprintln!("  1. Command line: --username USERNAME --password PASSWORD");
                eprintln!("  2. Environment variables:");
                eprintln!("     LASTFM_EDIT_USERNAME=your_lastfm_username");
                eprintln!("     LASTFM_EDIT_PASSWORD=your_lastfm_password");
                eprintln!();
                eprintln!("You can set environment variables in your shell profile or use direnv:");
                eprintln!("  echo 'export LASTFM_EDIT_USERNAME=\"your_username\"' >> ~/.bashrc");
                eprintln!("  echo 'export LASTFM_EDIT_PASSWORD=\"your_password\"' >> ~/.bashrc");
                std::process::exit(1);
            }
        }
    };

    log::info!("🔐 Using username: {username}");

    // Load or create client with session management
    let client = match load_or_create_client(&username, &password).await {
        Ok(client) => client,
        Err(e) => {
            eprintln!("❌ Failed to create client: {e}");
            std::process::exit(1);
        }
    };

    log::info!("✅ Client ready");

    // Execute the command
    if let Err(e) = execute_command(args.command, &client).await {
        eprintln!("❌ Command failed: {e}");
        std::process::exit(1);
    }

    Ok(())
}
