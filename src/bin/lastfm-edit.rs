use clap::Parser;
use lastfm_edit::commands::{
    execute_command, utils::get_credentials, utils::load_or_create_client, Commands,
};

/// Last.fm scrobble metadata editor
#[derive(Parser)]
#[command(
    name = "lastfm-edit",
    about = "Last.fm scrobble metadata editor",
    long_about = None
)]
struct Cli {
    /// Show detailed debug information
    #[arg(long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Cli::parse();

    // Enable debug logging if verbose flag is set
    if args.verbose {
        println!("🔍 Verbose mode enabled");
    }

    // Get credentials from environment
    let (username, password) = match get_credentials() {
        Ok(creds) => creds,
        Err(e) => {
            eprintln!("❌ Error: {e}");
            eprintln!();
            eprintln!("Please set the following environment variables:");
            eprintln!("  LASTFM_EDIT_USERNAME=your_lastfm_username");
            eprintln!("  LASTFM_EDIT_PASSWORD=your_lastfm_password");
            eprintln!();
            eprintln!("You can set these in your shell profile or use direnv:");
            eprintln!("  echo 'export LASTFM_EDIT_USERNAME=\"your_username\"' >> ~/.bashrc");
            eprintln!("  echo 'export LASTFM_EDIT_PASSWORD=\"your_password\"' >> ~/.bashrc");
            std::process::exit(1);
        }
    };

    if args.verbose {
        println!("🔐 Using username: {username}");
    }

    // Load or create client with session management
    let client = match load_or_create_client(&username, &password).await {
        Ok(client) => client,
        Err(e) => {
            eprintln!("❌ Failed to create client: {e}");
            std::process::exit(1);
        }
    };

    if args.verbose {
        println!("✅ Client ready");
    }

    // Execute the command
    if let Err(e) = execute_command(args.command, &client).await {
        eprintln!("❌ Command failed: {e}");
        std::process::exit(1);
    }

    Ok(())
}
