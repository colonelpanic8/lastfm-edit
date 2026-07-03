//! Loader for the embedded default rule corpus (`assets/default_rewrite_rules.json`).

use crate::rewrite::{RewriteRule, SdRule};
use serde::{Deserialize, Serialize};

/// Structures for default rule import
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DefaultRulePattern {
    pub find: String,
    pub replace: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DefaultRule {
    pub name: String,
    pub description: String,
    pub examples: Vec<String>,
    pub track_name: Option<DefaultRulePattern>,
    pub artist_name: Option<DefaultRulePattern>,
    pub album_name: Option<DefaultRulePattern>,
    pub album_artist_name: Option<DefaultRulePattern>,
    pub requires_confirmation: bool,
    /// Optional flag: require MusicBrainz confirmation of the rewritten metadata
    #[serde(default)]
    pub requires_musicbrainz_confirmation: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DefaultRuleSet {
    pub title: String,
    pub description: String,
    pub version: String,
    pub rules: Vec<DefaultRule>,
}

/// Default rules apply case-insensitively.
fn to_sd_rule(pattern: DefaultRulePattern) -> SdRule {
    SdRule::new(&pattern.find, &pattern.replace).with_flags("i")
}

/// Convert default rule to RewriteRule
impl From<DefaultRule> for RewriteRule {
    fn from(default_rule: DefaultRule) -> Self {
        RewriteRule {
            name: Some(default_rule.name),
            track_name: default_rule.track_name.map(to_sd_rule),
            artist_name: default_rule.artist_name.map(to_sd_rule),
            album_name: default_rule.album_name.map(to_sd_rule),
            album_artist_name: default_rule.album_artist_name.map(to_sd_rule),
            requires_confirmation: default_rule.requires_confirmation,
            requires_musicbrainz_confirmation: default_rule.requires_musicbrainz_confirmation,
            musicbrainz_release_filters: None, // Default rules use default MusicBrainz filters
        }
    }
}

/// Load default rewrite rules from embedded JSON
pub fn load_default_rewrite_rules() -> Result<DefaultRuleSet, Box<dyn std::error::Error>> {
    let json_content = include_str!("../assets/default_rewrite_rules.json");
    let rule_set: DefaultRuleSet = serde_json::from_str(json_content)?;
    Ok(rule_set)
}

/// Load all default rules from the single consolidated file
pub fn load_all_default_rules() -> Result<Vec<DefaultRule>, Box<dyn std::error::Error>> {
    let rule_set = load_default_rewrite_rules()?;
    Ok(rule_set.rules)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_loads_and_converts() {
        let rule_set = load_default_rewrite_rules().expect("embedded corpus must parse");
        assert!(
            rule_set.rules.len() >= 70,
            "expected the full corpus, got {}",
            rule_set.rules.len()
        );
        let rewrite_rules: Vec<RewriteRule> =
            rule_set.rules.into_iter().map(RewriteRule::from).collect();
        // Every converted SdRule must compile and default to case-insensitive.
        for rule in &rewrite_rules {
            for sd in [
                &rule.track_name,
                &rule.artist_name,
                &rule.album_name,
                &rule.album_artist_name,
            ]
            .into_iter()
            .flatten()
            {
                assert_eq!(sd.flags.as_deref(), Some("i"));
                sd.matches("probe").expect("pattern must compile");
            }
        }
    }
}
