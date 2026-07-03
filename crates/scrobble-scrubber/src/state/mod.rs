//! Durable scrubber-side state: active rules, the edit-intent queue, proposed rules,
//! subject dismissals, and per-provider planning coverage.
//!
//! Lives in `<store_root>/scrubber/` using the same durability idiom as scrobble-store:
//! append-only JSONL event logs (git `merge=union` safe) plus atomically-rewritten JSON
//! snapshots. Distinct from the store's edit log — that records *decided* operations;
//! this records decisions in flight.

mod fs;
mod memory;

pub use fs::FsScrubberState;
pub use memory::MemoryScrubberState;

use crate::error::Result;
use crate::queue::{EditIntent, PendingRule, QueueEvent, RuleEvent};
use crate::rewrite::RewriteRule;
use crate::subject::Subject;
use scrobble_store::CoverageMap;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

/// A subject the user never wants suggestions for again.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DismissedEntry {
    pub subject: Subject,
    pub at: u64,
    pub reason: String,
}

/// One provider's planning coverage: which time ranges its analysis has fully covered.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProviderCoverage {
    pub coverage: CoverageMap,
    /// For the rules provider: hash of the rule set the coverage was computed under.
    /// A mismatch on load means the rules changed and coverage must reset (the planner
    /// enforces this; storage just persists it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules_hash: Option<String>,
}

/// Hash of a rule set, for coverage invalidation when rules change.
pub fn rules_hash(rules: &[RewriteRule]) -> String {
    let canonical = serde_json::to_string(rules).unwrap_or_default();
    let digest = Sha256::digest(canonical.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// The storage contract for scrubber state.
#[async_trait::async_trait]
pub trait ScrubberState: Send + Sync {
    // ---- active rules -------------------------------------------------------------
    /// The active rule set; empty when none configured yet.
    async fn load_rules(&self) -> Result<Vec<RewriteRule>>;
    async fn save_rules(&self, rules: &[RewriteRule]) -> Result<()>;

    // ---- edit-intent queue ----------------------------------------------------------
    async fn append_queue_events(&self, events: &[QueueEvent]) -> Result<()>;
    /// The queue folded into per-intent state, in first-created order.
    async fn load_queue(&self) -> Result<Vec<EditIntent>>;

    // ---- pending rule proposals -------------------------------------------------------
    async fn append_rule_events(&self, events: &[RuleEvent]) -> Result<()>;
    async fn load_pending_rules(&self) -> Result<Vec<PendingRule>>;

    // ---- dismissals ---------------------------------------------------------------
    async fn load_dismissed(&self) -> Result<HashSet<Subject>>;
    async fn append_dismissed(&self, entries: &[DismissedEntry]) -> Result<()>;

    // ---- planning coverage ------------------------------------------------------------
    async fn load_provider_coverage(&self, provider: &str) -> Result<ProviderCoverage>;
    async fn save_provider_coverage(
        &self,
        provider: &str,
        coverage: &ProviderCoverage,
    ) -> Result<()>;
}
