# Scrobble Scrubber

Automated Last.fm track monitoring and scrubbing system that continuously monitors your recent tracks and applies cleaning rules to fix common issues.

## Features

- **Continuous Monitoring**: Polls your recent tracks at configurable intervals
- **State Management**: Remembers which tracks have been processed to avoid duplicates
- **Automated Cleaning Rules**:
  - Removes "Remaster" suffixes from track names
  - Normalizes featuring artist formats (ft. → feat.)
- **Dry Run Mode**: Test changes without actually modifying your scrobbles
- **Configurable Limits**: Control how many tracks to process per cycle

## Usage

Set up environment variables:
```bash
export LASTFM_EDIT_USERNAME="your_username"
export LASTFM_EDIT_PASSWORD="your_password"
```

Run the scrubber:
```bash
# Basic usage (checks every 5 minutes)
cargo run

# Custom interval (check every 10 minutes)
cargo run -- --interval 600

# Dry run mode (see what would be changed)
cargo run -- --dry-run

# Limit tracks per cycle
cargo run -- --max-tracks 50
```

## Command Line Options

- `-i, --interval <SECONDS>`: Check interval in seconds (default: 300)
- `-m, --max-tracks <NUMBER>`: Maximum tracks to process per run (default: 100)
- `--dry-run`: Show what would be changed without making actual edits

## Cleaning Rules

### Remaster Removal
Removes various remaster patterns from track names:
- `Song Name - 2019 Remaster` → `Song Name`
- `Song Name (Remaster 2019)` → `Song Name`
- `Song Name (Remaster)` → `Song Name`

### Featuring Normalization
Standardizes featuring artist formats:
- `Artist ft. Other` → `Artist feat. Other`
- `Artist featuring Other` → `Artist feat. Other`

## Architecture

The scrubber uses the `lastfm-edit` library for all Last.fm interactions and implements:

1. **Track Iterator**: Uses `RecentTracksIterator` with timestamp-based stopping
2. **State Tracking**: Maintains a set of seen tracks to avoid reprocessing
3. **Rule Engine**: Modular system for adding new cleaning rules
4. **Action System**: Structured approach to track/artist modifications