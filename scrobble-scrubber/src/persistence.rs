use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use crate::rewrite::RewriteRule;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TimestampState {
    /// Timestamp of the most recent processed scrobble
    pub last_processed_timestamp: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RewriteRulesState {
    /// Set of regex rewrite rules for cleaning track/artist names
    pub rewrite_rules: Vec<RewriteRule>,
}


#[async_trait]
pub trait StateStorage: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Clear all stored state
    #[allow(dead_code)]
    async fn clear_state(&mut self) -> Result<(), Self::Error>;

    /// Load the timestamp state
    async fn load_timestamp_state(&self) -> Result<TimestampState, Self::Error>;

    /// Save the timestamp state
    async fn save_timestamp_state(&mut self, state: &TimestampState) -> Result<(), Self::Error>;

    /// Load the rewrite rules state
    async fn load_rewrite_rules_state(&self) -> Result<RewriteRulesState, Self::Error>;

    /// Save the rewrite rules state
    async fn save_rewrite_rules_state(&mut self, state: &RewriteRulesState) -> Result<(), Self::Error>;
}

/// File-based storage implementation using pickledb
pub struct FileStorage {
    db: pickledb::PickleDb,
}

impl FileStorage {
    pub fn new(path: &str) -> Result<Self, pickledb::error::Error> {
        let db = pickledb::PickleDb::load_json(path, pickledb::PickleDbDumpPolicy::AutoDump)
            .unwrap_or_else(|_| {
                pickledb::PickleDb::new_json(path, pickledb::PickleDbDumpPolicy::AutoDump)
            });
        Ok(Self { db })
    }
}

#[async_trait]
impl StateStorage for FileStorage {
    type Error = pickledb::error::Error;

    async fn clear_state(&mut self) -> Result<(), Self::Error> {
        self.db.rem("timestamp_state").ok();
        self.db.rem("rewrite_rules_state").ok();
        Ok(())
    }

    async fn load_timestamp_state(&self) -> Result<TimestampState, Self::Error> {
        match self.db.get("timestamp_state") {
            Some(state) => Ok(state),
            None => Ok(TimestampState::default()),
        }
    }

    async fn save_timestamp_state(&mut self, state: &TimestampState) -> Result<(), Self::Error> {
        self.db.set("timestamp_state", state)?;
        Ok(())
    }

    async fn load_rewrite_rules_state(&self) -> Result<RewriteRulesState, Self::Error> {
        match self.db.get("rewrite_rules_state") {
            Some(state) => Ok(state),
            None => Ok(RewriteRulesState::default()),
        }
    }

    async fn save_rewrite_rules_state(&mut self, state: &RewriteRulesState) -> Result<(), Self::Error> {
        self.db.set("rewrite_rules_state", state)?;
        Ok(())
    }
}

/// In-memory storage implementation for testing
pub struct MemoryStorage {
    timestamp_state: tokio::sync::RwLock<TimestampState>,
    rewrite_rules_state: tokio::sync::RwLock<RewriteRulesState>,
}

impl MemoryStorage {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            timestamp_state: tokio::sync::RwLock::new(TimestampState::default()),
            rewrite_rules_state: tokio::sync::RwLock::new(RewriteRulesState::default()),
        }
    }
}

#[async_trait]
impl StateStorage for MemoryStorage {
    type Error = std::convert::Infallible;

    async fn clear_state(&mut self) -> Result<(), Self::Error> {
        *self.timestamp_state.write().await = TimestampState::default();
        *self.rewrite_rules_state.write().await = RewriteRulesState::default();
        Ok(())
    }

    async fn load_timestamp_state(&self) -> Result<TimestampState, Self::Error> {
        Ok(self.timestamp_state.read().await.clone())
    }

    async fn save_timestamp_state(&mut self, state: &TimestampState) -> Result<(), Self::Error> {
        *self.timestamp_state.write().await = state.clone();
        Ok(())
    }

    async fn load_rewrite_rules_state(&self) -> Result<RewriteRulesState, Self::Error> {
        Ok(self.rewrite_rules_state.read().await.clone())
    }

    async fn save_rewrite_rules_state(&mut self, state: &RewriteRulesState) -> Result<(), Self::Error> {
        *self.rewrite_rules_state.write().await = state.clone();
        Ok(())
    }
}


impl RewriteRulesState {
    pub fn with_default_rules() -> Self {
        Self {
            rewrite_rules: crate::rewrite::default_rules(),
        }
    }
}
