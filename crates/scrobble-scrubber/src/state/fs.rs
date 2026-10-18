//! Filesystem-backed scrubber state: JSONL event logs + atomic JSON snapshots under a
//! state directory (canonically `<store_root>/scrubber/`).

use super::{DismissedEntry, ProviderCoverage, ScrubberState};
use crate::error::Result;
use crate::queue::{
    fold_pending_rules, fold_queue, EditIntent, PendingRule, QueueEvent, RuleEvent,
};
use crate::rewrite::RewriteRule;
use crate::subject::Subject;
use scrobble_store::durable::{append_jsonl, atomic_write, create_dir_all, read_jsonl};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const GITATTRIBUTES: &str = "*.jsonl merge=union\n";

pub struct FsScrubberState {
    root: PathBuf,
    queue_changed: tokio::sync::watch::Sender<()>,
}

impl FsScrubberState {
    /// Open the state directory, creating the skeleton if needed. Safe on existing state.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        create_dir_all(&root.join("coverage"))?;
        let gitattributes = root.join(".gitattributes");
        if !gitattributes.exists() {
            atomic_write(&gitattributes, GITATTRIBUTES.as_bytes())?;
        }
        Ok(Self {
            root,
            queue_changed: tokio::sync::watch::channel(()).0,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn append_jsonl<T: Serialize>(&self, name: &str, items: &[T]) -> Result<()> {
        if items.is_empty() {
            return Ok(());
        }
        let mut lines = String::new();
        for item in items {
            lines.push_str(&serde_json::to_string(item)?);
            lines.push('\n');
        }
        let path = self.root.join(name);
        append_jsonl(&path, lines.as_bytes())?;
        Ok(())
    }

    fn read_jsonl<T: DeserializeOwned>(&self, name: &str) -> Result<Vec<T>> {
        Ok(read_jsonl(&self.root.join(name))?)
    }

    fn coverage_path(&self, provider: &str) -> PathBuf {
        // Provider names are code-defined identifiers, but sanitize defensively.
        let safe: String = provider
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        self.root.join("coverage").join(format!("{safe}.json"))
    }
}

#[async_trait::async_trait]
impl ScrubberState for FsScrubberState {
    async fn load_rules(&self) -> Result<Vec<RewriteRule>> {
        let path = self.root.join("rules.json");
        if !path.exists() {
            return Ok(Vec::new());
        }
        Ok(serde_json::from_str(&std::fs::read_to_string(&path)?)?)
    }

    async fn save_rules(&self, rules: &[RewriteRule]) -> Result<()> {
        let mut contents = serde_json::to_string_pretty(rules)?;
        contents.push('\n');
        Ok(atomic_write(
            &self.root.join("rules.json"),
            contents.as_bytes(),
        )?)
    }

    async fn append_queue_events(&self, events: &[QueueEvent]) -> Result<()> {
        self.append_jsonl("queue.jsonl", events)?;
        self.queue_changed.send_replace(());
        Ok(())
    }

    fn watch_queue(&self) -> tokio::sync::watch::Receiver<()> {
        self.queue_changed.subscribe()
    }

    async fn load_queue(&self) -> Result<Vec<EditIntent>> {
        Ok(fold_queue(self.read_jsonl::<QueueEvent>("queue.jsonl")?))
    }

    async fn append_rule_events(&self, events: &[RuleEvent]) -> Result<()> {
        self.append_jsonl("pending_rules.jsonl", events)
    }

    async fn load_pending_rules(&self) -> Result<Vec<PendingRule>> {
        Ok(fold_pending_rules(
            self.read_jsonl::<RuleEvent>("pending_rules.jsonl")?,
        ))
    }

    async fn load_dismissed(&self) -> Result<HashSet<Subject>> {
        Ok(super::fold_dismissed(
            self.read_jsonl::<DismissedEntry>("dismissed.jsonl")?,
        ))
    }

    async fn append_dismissed(&self, entries: &[DismissedEntry]) -> Result<()> {
        self.append_jsonl("dismissed.jsonl", entries)
    }

    async fn load_provider_coverage(&self, provider: &str) -> Result<ProviderCoverage> {
        let path = self.coverage_path(provider);
        if !path.exists() {
            return Ok(ProviderCoverage::default());
        }
        Ok(serde_json::from_str(&std::fs::read_to_string(&path)?)?)
    }

    async fn save_provider_coverage(
        &self,
        provider: &str,
        coverage: &ProviderCoverage,
    ) -> Result<()> {
        let mut contents = serde_json::to_string_pretty(coverage)?;
        contents.push('\n');
        Ok(atomic_write(
            &self.coverage_path(provider),
            contents.as_bytes(),
        )?)
    }

    async fn append_resolution_events(
        &self,
        events: &[crate::identity::ResolutionEvent],
    ) -> Result<()> {
        self.append_jsonl("resolutions.jsonl", events)
    }

    async fn load_resolution_events(&self) -> Result<Vec<crate::identity::ResolutionEvent>> {
        self.read_jsonl("resolutions.jsonl")
    }
}
