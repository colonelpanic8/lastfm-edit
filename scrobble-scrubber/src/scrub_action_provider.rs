use crate::persistence::RewriteRulesState;
use crate::rewrite::{RewriteError, RewriteRule};
use async_trait::async_trait;
use lastfm_edit::{ScrobbleEdit, Track};
use std::error::Error;
use std::fmt;

/// Generic error type for action providers
#[derive(Debug)]
pub struct ActionProviderError(pub String);

impl fmt::Display for ActionProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Action provider error: {}", self.0)
    }
}

impl Error for ActionProviderError {}

impl From<RewriteError> for ActionProviderError {
    fn from(err: RewriteError) -> Self {
        ActionProviderError(format!("Rewrite error: {}", err))
    }
}

impl From<String> for ActionProviderError {
    fn from(msg: String) -> Self {
        ActionProviderError(msg)
    }
}

impl From<&str> for ActionProviderError {
    fn from(msg: &str) -> Self {
        ActionProviderError(msg.to_string())
    }
}

/// Represents a suggested action from an external source (LLM, API, etc.)
#[derive(Debug, Clone)]
pub enum ScrubActionSuggestion {
    /// Suggest an immediate scrobble edit
    Edit(ScrobbleEdit),
    /// Propose a new rewrite rule
    ProposeRule {
        rule: RewriteRule,
        motivation: String,
    },
    /// No action needed
    NoAction,
}

/// Trait for external providers that can suggest scrobble actions
#[async_trait]
pub trait ScrubActionProvider: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Analyze a track and provide suggestions for improvements
    async fn analyze_track(&self, track: &Track) -> Result<ScrubActionSuggestion, Self::Error>;

    /// Get a human-readable name for this provider
    fn provider_name(&self) -> &str;
}


/// Rewrite rules-based action provider
pub struct RewriteRulesScrubActionProvider {
    rules: Vec<RewriteRule>,
}

impl RewriteRulesScrubActionProvider {
    pub fn new(rules_state: &RewriteRulesState) -> Self {
        Self {
            rules: rules_state.rewrite_rules.clone(),
        }
    }

    pub fn from_rules(rules: Vec<RewriteRule>) -> Self {
        Self { rules }
    }
}

#[async_trait]
impl ScrubActionProvider for RewriteRulesScrubActionProvider {
    type Error = ActionProviderError;

    async fn analyze_track(&self, track: &Track) -> Result<ScrubActionSuggestion, Self::Error> {
        // Check if any rules would apply
        let rules_apply = crate::rewrite::any_rules_apply(&self.rules, track)?;

        if !rules_apply {
            return Ok(ScrubActionSuggestion::NoAction);
        }

        // Apply all rules to see what changes would be made
        let mut edit = crate::rewrite::create_no_op_edit(track);
        let changes_made = crate::rewrite::apply_all_rules(&self.rules, &mut edit)?;

        if !changes_made {
            return Ok(ScrubActionSuggestion::NoAction);
        }

        // Check if any of the applicable rules require confirmation
        let needs_confirmation = self
            .rules
            .iter()
            .any(|rule| rule.applies_to(track).unwrap_or(false) && rule.requires_confirmation);

        // If confirmation needed, propose a rule instead of immediate action
        if needs_confirmation {
            // For now, return a simple rule proposal
            // TODO: Create a more sophisticated rule from the applied changes
            return Ok(ScrubActionSuggestion::ProposeRule {
                rule: RewriteRule::new(), // This should be constructed based on the actual applied rules
                motivation: "One or more rules require confirmation".to_string(),
            });
        }

        // Return the ScrobbleEdit directly
        Ok(ScrubActionSuggestion::Edit(edit))
    }

    fn provider_name(&self) -> &str {
        "RewriteRules"
    }
}


/// Combines multiple providers, trying each one in order until one returns a non-NoAction result
pub struct OrScrubActionProvider {
    providers: Vec<Box<dyn ScrubActionProvider<Error = ActionProviderError>>>,
    provider_names: Vec<String>,
}

impl OrScrubActionProvider {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
            provider_names: Vec::new(),
        }
    }

    pub fn add_provider<P>(mut self, provider: P) -> Self
    where
        P: ScrubActionProvider + 'static,
        P::Error: Into<ActionProviderError>,
    {
        let name = provider.provider_name().to_string();
        self.provider_names.push(name);

        // Wrap the provider to match our error type
        let wrapped_provider = ErrorAdapter { inner: provider };
        self.providers.push(Box::new(wrapped_provider));
        self
    }

    pub fn with_providers<P>(providers: Vec<P>) -> Self
    where
        P: ScrubActionProvider + 'static,
        P::Error: Into<ActionProviderError>,
    {
        let mut or_provider = Self::new();
        for provider in providers {
            or_provider = or_provider.add_provider(provider);
        }
        or_provider
    }
}

// Adapter to convert different error types to our unified error type
struct ErrorAdapter<P> {
    inner: P,
}

#[async_trait]
impl<P> ScrubActionProvider for ErrorAdapter<P>
where
    P: ScrubActionProvider + Send + Sync,
    P::Error: Into<ActionProviderError>,
{
    type Error = ActionProviderError;

    async fn analyze_track(&self, track: &Track) -> Result<ScrubActionSuggestion, Self::Error> {
        self.inner
            .analyze_track(track)
            .await
            .map_err(|e| e.into())
    }

    fn provider_name(&self) -> &str {
        self.inner.provider_name()
    }
}

#[async_trait]
impl ScrubActionProvider for OrScrubActionProvider {
    type Error = ActionProviderError;

    async fn analyze_track(&self, track: &Track) -> Result<ScrubActionSuggestion, Self::Error> {
        for (i, provider) in self.providers.iter().enumerate() {
            match provider.analyze_track(track).await {
                Ok(ScrubActionSuggestion::NoAction) => {
                    // Try next provider
                    continue;
                }
                Ok(suggestion) => {
                    // Found a suggestion, return it
                    return Ok(suggestion);
                }
                Err(e) => {
                    // Log error but continue to next provider
                    log::warn!(
                        "Error from provider '{}': {}",
                        self.provider_names.get(i).unwrap_or(&"unknown".to_string()),
                        e
                    );
                    continue;
                }
            }
        }

        // All providers returned NoAction or failed
        Ok(ScrubActionSuggestion::NoAction)
    }

    fn provider_name(&self) -> &str {
        "OrProvider"
    }
}
