use chrono::Utc;
use lastfm_edit::{iterator::AsyncPaginatedIterator, LastFmClient, Result};
use log::{info, warn};
use std::collections::HashSet;

use crate::persistence::{RewriteRulesState, StateStorage, TimestampState};
use crate::rewrite;

// Use Args from lib
use crate::Args;

pub struct ScrobbleScrubber<S: StateStorage> {
    client: LastFmClient,
    args: Args,
    storage: S,
    timestamp_state: TimestampState,
    rules_state: RewriteRulesState,
    seen_tracks: HashSet<String>,
}

impl<S: StateStorage> ScrobbleScrubber<S> {
    pub async fn new(args: Args, mut storage: S, client: LastFmClient) -> Result<Self> {
        // Load existing states or create with defaults
        let timestamp_state = storage.load_timestamp_state().await.map_err(|e| {
            lastfm_edit::LastFmError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to load timestamp state: {}", e),
            ))
        })?;

        let mut rules_state = storage.load_rewrite_rules_state().await.map_err(|e| {
            lastfm_edit::LastFmError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to load rules state: {}", e),
            ))
        })?;

        if rules_state.rewrite_rules.is_empty() {
            info!("No existing rules found, initializing with default rules");
            rules_state = RewriteRulesState::with_default_rules();
            storage
                .save_rewrite_rules_state(&rules_state)
                .await
                .map_err(|e| {
                    lastfm_edit::LastFmError::Io(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!("Failed to save initial rules: {}", e),
                    ))
                })?;
        } else {
            info!(
                "Loaded {} rewrite rules from state",
                rules_state.rewrite_rules.len()
            );
        }

        Ok(Self {
            client,
            args,
            storage,
            timestamp_state,
            rules_state,
            seen_tracks: HashSet::new(),
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        loop {
            info!("Starting track monitoring cycle...");

            if let Err(e) = self.check_and_process_tracks().await {
                warn!("Error during track processing: {}", e);
            }

            // Update timestamp and save timestamp state
            self.timestamp_state.last_processed_timestamp = Some(Utc::now());
            if let Err(e) = self
                .storage
                .save_timestamp_state(&self.timestamp_state)
                .await
            {
                warn!("Failed to save timestamp state: {}", e);
            }

            info!("Sleeping for {} seconds...", self.args.interval);
            tokio::time::sleep(std::time::Duration::from_secs(self.args.interval)).await;
        }
    }

    async fn check_and_process_tracks(&mut self) -> Result<()> {
        let mut recent_iterator = self.client.recent_tracks();
        let mut processed = 0;
        let mut tracks_to_process = Vec::new();

        // First, collect all tracks to process
        while let Some(track) = recent_iterator.next().await? {
            if processed >= self.args.max_tracks {
                info!(
                    "Reached maximum track limit ({}), stopping",
                    self.args.max_tracks
                );
                break;
            }

            let track_id = format!("{}|{}", track.artist, track.name);

            if !self.seen_tracks.contains(&track_id) {
                tracks_to_process.push((track, track_id));
                processed += 1;
            }
        }

        // Now process each track
        for (track, track_id) in tracks_to_process {
            info!("Processing new track: {} - {}", track.artist, track.name);

            if let Some(action) = self.analyze_track(&track).await {
                if self.args.dry_run {
                    info!("DRY RUN: Would apply action: {:?}", action);
                } else {
                    self.apply_action(&track, &action).await?;
                }
            }

            self.seen_tracks.insert(track_id);
        }

        info!("Processed {} new tracks", processed);
        Ok(())
    }

    async fn analyze_track(&self, track: &lastfm_edit::Track) -> Option<ScrubAction> {
        // Check if any rules would apply before creating an edit
        match rewrite::any_rules_apply(&self.rules_state.rewrite_rules, track) {
            Ok(false) => return None, // No rules apply, short circuit
            Err(e) => {
                warn!("Error checking if rules apply: {}", e);
                return None;
            }
            Ok(true) => {
                // At least one rule applies, proceed with creating and applying edits
            }
        }

        // Create a no-op edit and apply all rules to it
        let mut edit = rewrite::create_no_op_edit(track);

        match rewrite::apply_all_rules(&self.rules_state.rewrite_rules, &mut edit) {
            Ok(true) => {
                info!(
                    "Rules applied to track '{}' by '{}':",
                    track.name, track.artist
                );

                // Convert ScrobbleEdit back to ScrubAction for compatibility
                // For now, prioritize track name changes, then artist changes
                if edit.track_name != edit.track_name_original {
                    info!(
                        "  Track: '{}' -> '{}'",
                        edit.track_name_original, edit.track_name
                    );
                    return Some(ScrubAction::RenameTrack {
                        new_name: edit.track_name,
                    });
                }
                if edit.artist_name != edit.artist_name_original {
                    info!(
                        "  Artist: '{}' -> '{}'",
                        edit.artist_name_original, edit.artist_name
                    );
                    return Some(ScrubAction::RenameArtist {
                        new_artist: edit.artist_name,
                    });
                }
                // TODO: Handle album and album_artist changes when ScrubAction supports them
            }
            Ok(false) => {
                // No changes were made (shouldn't happen given the check above, but handle gracefully)
                return None;
            }
            Err(e) => {
                warn!("Error applying rules: {}", e);
                return None;
            }
        }

        None
    }

    async fn apply_action(
        &mut self,
        track: &lastfm_edit::Track,
        action: &ScrubAction,
    ) -> Result<()> {
        match action {
            ScrubAction::RenameTrack { new_name } => {
                info!("Renaming track '{}' to '{}'", track.name, new_name);
                // TODO: Implement track name editing in lastfm-edit library
                warn!(
                    "Track renaming not yet implemented: '{}' -> '{}'",
                    track.name, new_name
                );
            }
            ScrubAction::RenameArtist { new_artist } => {
                info!(
                    "Renaming artist '{}' to '{}' for track '{}'",
                    track.artist, new_artist, track.name
                );
                self.client
                    .edit_artist_for_track(&track.name, &track.artist, new_artist)
                    .await?;
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
enum ScrubAction {
    RenameTrack { new_name: String },
    RenameArtist { new_artist: String },
}
