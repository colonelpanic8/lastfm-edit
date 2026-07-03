# lastfm-edit Rust Crate

## Project Overview
Rust crate for programmatic access to last.fm's scrobble editing functionality

## Environment
- Uses direnv with nix flake for development environment
- Environment variables: `LASTFM_EDIT_USERNAME`, `LASTFM_EDIT_PASSWORD` set in `.envrc`
- Run with: `direnv exec . cargo run --example <name>`

## Browser and MFA
- When a login flow needs SMS-based two-factor authentication, use Chrome to open `https://messages.google.com/web/conversations` and check Google Messages for the current verification text.
- Prefer this Google Messages route whenever possible before asking Ivan for a one-time code. Read only the relevant verification-message thread or conversation preview needed for the current login.

* Remember that 'variables can be used directly in the `format!` string'
* Make sure to run `cargo fmt --all` after finishing making changes
* Make sure to fix any clippy issues `cargo clippy --all-targets --all-features -- -D warnings` after making changes
