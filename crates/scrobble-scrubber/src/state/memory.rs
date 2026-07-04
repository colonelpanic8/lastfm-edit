//! In-memory scrubber state for tests and ephemeral use.

use super::{DismissedEntry, ProviderCoverage, ScrubberState};
use crate::error::Result;
use crate::queue::{
    fold_pending_rules, fold_queue, EditIntent, PendingRule, QueueEvent, RuleEvent,
};
use crate::rewrite::RewriteRule;
use crate::subject::Subject;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

#[derive(Default)]
struct Inner {
    rules: Vec<RewriteRule>,
    queue_events: Vec<QueueEvent>,
    rule_events: Vec<RuleEvent>,
    dismissed: Vec<DismissedEntry>,
    coverage: HashMap<String, ProviderCoverage>,
}

#[derive(Default)]
pub struct MemoryScrubberState {
    inner: Mutex<Inner>,
}

impl MemoryScrubberState {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl ScrubberState for MemoryScrubberState {
    async fn load_rules(&self) -> Result<Vec<RewriteRule>> {
        Ok(self.inner.lock().unwrap().rules.clone())
    }

    async fn save_rules(&self, rules: &[RewriteRule]) -> Result<()> {
        self.inner.lock().unwrap().rules = rules.to_vec();
        Ok(())
    }

    async fn append_queue_events(&self, events: &[QueueEvent]) -> Result<()> {
        self.inner
            .lock()
            .unwrap()
            .queue_events
            .extend_from_slice(events);
        Ok(())
    }

    async fn load_queue(&self) -> Result<Vec<EditIntent>> {
        Ok(fold_queue(self.inner.lock().unwrap().queue_events.clone()))
    }

    async fn append_rule_events(&self, events: &[RuleEvent]) -> Result<()> {
        self.inner
            .lock()
            .unwrap()
            .rule_events
            .extend_from_slice(events);
        Ok(())
    }

    async fn load_pending_rules(&self) -> Result<Vec<PendingRule>> {
        Ok(fold_pending_rules(
            self.inner.lock().unwrap().rule_events.clone(),
        ))
    }

    async fn load_dismissed(&self) -> Result<HashSet<Subject>> {
        Ok(super::fold_dismissed(
            self.inner.lock().unwrap().dismissed.clone(),
        ))
    }

    async fn append_dismissed(&self, entries: &[DismissedEntry]) -> Result<()> {
        self.inner
            .lock()
            .unwrap()
            .dismissed
            .extend_from_slice(entries);
        Ok(())
    }

    async fn load_provider_coverage(&self, provider: &str) -> Result<ProviderCoverage> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .coverage
            .get(provider)
            .cloned()
            .unwrap_or_default())
    }

    async fn save_provider_coverage(
        &self,
        provider: &str,
        coverage: &ProviderCoverage,
    ) -> Result<()> {
        self.inner
            .lock()
            .unwrap()
            .coverage
            .insert(provider.to_string(), coverage.clone());
        Ok(())
    }
}
