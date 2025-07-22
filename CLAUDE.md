# lastfm-edit Rust Crate

## Project Overview
Building a Rust crate for programmatic access to Last.fm's scrobble editing functionality via web scraping using the `http-client` abstraction library.

## Environment
- Uses direnv with nix flake for development environment
- Environment variables: `LASTFM_EDIT_USERNAME`, `LASTFM_EDIT_PASSWORD` set in `.envrc`
- Run with: `direnv exec . cargo run --example <name>`

* Remember that 'variables can be used directly in the `format!` string'
* Make sure to run `cargo fmt --all` after finishing making changes
* Make sure to fix any clippy issues `cargo clippy --all-targets --all-features -- -D warnings` after making changes
