use clap::Parser;
use log::LevelFilter;

mod commands;
use commands::{
    execute_command, utils::get_credentials, utils::load_or_create_client,
    utils::prompt_for_credentials, utils::try_restore_most_recent_session, Commands,
};

/// Last.fm scrobble metadata editor
#[derive(Parser)]
#[command(
    name = "lastfm-edit",
    about = "Last.fm scrobble metadata editor",
    long_about = None
)]
struct Cli {
    /// Decrease verbosity level (use multiple times for less verbose output)
    /// Default is info level. -q: warn only, -qq: error only, -qqq: off
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    quiet: u8,

    /// Increase verbosity level (use multiple times for more verbose output)
    /// -v: debug for lastfm-edit, -vv: trace for lastfm-edit, -vvv: trace for lastfm-edit + info for all
    /// -vvvv: trace for lastfm-edit + debug for all, -vvvvv: trace for all
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

    // Configure logging based on verbosity level
    // Default is Info for lastfm-edit, Warn for everything else
    // -q decreases verbosity, -v increases it
    let mut builder = env_logger::Builder::from_default_env();

    // Calculate effective verbosity: positive = more verbose, negative = less verbose
    let effective_level = args.verbose as i8 - args.quiet as i8;

    match effective_level {
        i8::MIN..=-3 => {
            // -qqq or more: completely silent
            builder.filter_level(LevelFilter::Off);
        }
        -2 => {
            // -qq: errors only
            builder.filter_level(LevelFilter::Error);
        }
        -1 => {
            // -q: warnings only
            builder.filter_level(LevelFilter::Warn);
        }
        0 => {
            // Default: info for lastfm-edit, warn for others
            builder.filter_level(LevelFilter::Warn);
            builder.filter_module("lastfm_edit", LevelFilter::Info);
        }
        1 => {
            // -v: debug for lastfm-edit
            builder.filter_level(LevelFilter::Warn);
            builder.filter_module("lastfm_edit", LevelFilter::Debug);
        }
        2 => {
            // -vv: trace for lastfm-edit
            builder.filter_level(LevelFilter::Warn);
            builder.filter_module("lastfm_edit", LevelFilter::Trace);
        }
        3 => {
            // -vvv: trace for lastfm-edit + info for all others
            builder.filter_level(LevelFilter::Info);
            builder.filter_module("lastfm_edit", LevelFilter::Trace);
        }
        4 => {
            // -vvvv: trace for lastfm-edit + debug for all others
            builder.filter_level(LevelFilter::Debug);
            builder.filter_module("lastfm_edit", LevelFilter::Trace);
        }
        _ => {
            // -vvvvv or more: trace for everything
            builder.filter_level(LevelFilter::Trace);
        }
    }

    builder.init();

    // Try to get credentials from command line args or environment first
    let (username, password) = if let (Some(u), Some(p)) = (&args.username, &args.password) {
        (Some(u.clone()), Some(p.clone()))
    } else if args.username.is_some() || args.password.is_some() {
        log::error!("Both username and password must be provided together");
        log::error!("Either provide both --username and --password, or set environment variables");
        std::process::exit(1);
    } else {
        match get_credentials() {
            Ok((u, p)) => (Some(u), Some(p)),
            Err(_) => (None, None), // No credentials provided
        }
    };

    // First, try to restore the most recent session if no credentials were provided
    let client = if username.is_none() && password.is_none() {
        match try_restore_most_recent_session().await {
            Some(client) => {
                log::info!("Restored most recent session");
                client
            }
            None => {
                // No valid session found, prompt for credentials
                log::info!("No valid saved session found. Please provide credentials:");
                let (prompted_username, prompted_password) = prompt_for_credentials();
                log::info!("Using username: {prompted_username}");

                match load_or_create_client(&prompted_username, &prompted_password).await {
                    Ok(client) => client,
                    Err(e) => {
                        log::error!("Failed to create client: {e}");
                        std::process::exit(1);
                    }
                }
            }
        }
    } else {
        // Credentials were provided, use them directly
        let username = username.unwrap();
        let password = password.unwrap();
        log::info!("Using username: {username}");

        match load_or_create_client(&username, &password).await {
            Ok(client) => client,
            Err(e) => {
                log::error!("Failed to create client: {e}");
                std::process::exit(1);
            }
        }
    };

    log::info!("Client ready");

    // Execute the command
    if let Err(e) = execute_command(args.command, &client).await {
        log::error!("Command failed: {e}");
        std::process::exit(1);
    }

    Ok(())
}
