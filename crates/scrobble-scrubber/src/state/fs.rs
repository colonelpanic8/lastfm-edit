//! Filesystem-backed scrubber state: JSONL event logs + atomic JSON snapshots under a
//! state directory (canonically `<store_root>/scrubber/`).

use super::{DismissedEntry, ProviderCoverage, ScrubberState};
use crate::error::Result;
use crate::queue::{
    fold_pending_rules, fold_queue, EditIntent, PendingRule, QueueEvent, RuleEvent,
};
use crate::rewrite::RewriteRule;
use crate::subject::Subject;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::HashSet;
use std::io::Write as _;
use std::path::{Path, PathBuf};

const GITATTRIBUTES: &str = "*.jsonl merge=union\n";

pub struct FsScrubberState {
    root: PathBuf,
}

impl FsScrubberState {
    /// Open the state directory, creating the skeleton if needed. Safe on existing state.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(root.join("coverage"))?;
        let gitattributes = root.join(".gitattributes");
        if !gitattributes.exists() {
            atomic_write(&gitattributes, GITATTRIBUTES.as_bytes())?;
        }
        Ok(Self { root })
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
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        file.write_all(lines.as_bytes())?;
        file.sync_all()?;
        Ok(())
    }

    fn read_jsonl<T: DeserializeOwned>(&self, name: &str) -> Result<Vec<T>> {
        let path = self.root.join(name);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let content = std::fs::read_to_string(&path)?;
        let mut items = Vec::new();
        for line in content.lines().filter(|l| !l.trim().is_empty()) {
            match serde_json::from_str(line) {
                Ok(item) => items.push(item),
                Err(err) => {
                    // Torn tail or hand-mangled line: tolerate, same as the store.
                    log::warn!("skipping unparseable line in {}: {err}", path.display());
                }
            }
        }
        Ok(items)
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

/// Write a file atomically: temp sibling + rename.
fn atomic_write(path: &Path, contents: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(contents)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
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
        atomic_write(&self.root.join("rules.json"), contents.as_bytes())
    }

    async fn append_queue_events(&self, events: &[QueueEvent]) -> Result<()> {
        self.append_jsonl("queue.jsonl", events)
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
        Ok(self
            .read_jsonl::<DismissedEntry>("dismissed.jsonl")?
            .into_iter()
            .map(|entry| entry.subject)
            .collect())
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
        atomic_write(&self.coverage_path(provider), contents.as_bytes())
    }
}
