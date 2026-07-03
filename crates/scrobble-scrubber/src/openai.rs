//! OpenAI-backed action provider (feature = `openai`).
//!
//! Ported from the original scrobble-scrubber. Uses OpenAI function calling to suggest
//! immediate track edits and new rewrite rules. Pending-queue context (open edit intents
//! and pending rule proposals) is serialized into the prompt so the model avoids
//! suggesting duplicates.

use async_trait::async_trait;
use lastfm_edit::Track;
use openai_api_rs::v1::api::OpenAIClient;
use openai_api_rs::v1::chat_completion::{
    self, ChatCompletionRequest, Tool, ToolChoiceType, ToolType,
};
use openai_api_rs::v1::common::GPT4_O;
use openai_api_rs::v1::types::{Function, FunctionParameters, JSONSchemaDefine, JSONSchemaType};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::provider::{
    ActionProviderError, ScrubActionProvider, ScrubActionSuggestion, SuggestionWithContext,
};
use crate::queue::{EditIntent, PendingRule};
use crate::rewrite::RewriteRule;

/// Default system prompt for the OpenAI provider (ported from the original
/// scrobble-scrubber's `DEFAULT_CLAUDE_SYSTEM_PROMPT`).
pub const DEFAULT_SYSTEM_PROMPT: &str = "You are a music metadata cleaning assistant with function calling tools available. You work alongside automated rewrite rules and have two main responsibilities:

1. SUGGEST IMMEDIATE CORRECTIONS for complex metadata issues
2. RECOMMEND NEW REWRITE RULES when you identify patterns that could be automated

AVAILABLE FUNCTIONS:
- suggest_track_edit: Propose immediate metadata corrections for this specific track
- suggest_rewrite_rule: Recommend new rewrite rules for patterns that could be automated

If no changes are needed, simply don't call any functions.

WHEN TO SUGGEST IMMEDIATE CORRECTIONS (suggest_track_edit):
- Complex typos requiring musical knowledge to identify
- Album name corrections from compilations to original albums
- Artist name standardization (e.g. \"The Beatles\" vs \"Beatles\")
- Context-dependent punctuation/capitalization fixes
- Album artist corrections for compilations vs. regular albums
- Complex featuring/collaboration format restructuring
- Issues that don't match existing automated rule patterns

WHEN TO RECOMMEND NEW REWRITE RULES:
If you notice patterns that could be automated, mention in your reasoning:
\"PATTERN DETECTED: This issue could be handled by a rewrite rule like [pattern] → [replacement]\"

PRIORITY CLEANUP TARGETS:
Always prioritize removing these types of extraneous information from track names:
- Remaster indicators: \"2009 Remaster\", \"Remastered\", \"2024 Remaster\", etc.
- Version indicators: \"Deluxe Version\", \"Anniversary Edition\", \"Special Edition\", etc.
- Year suffixes: \"- 2010 Version\", \"(2015 Remaster)\", etc.
- Edition markers: \"(Deluxe)\", \"(Extended)\", \"(Single Version)\", etc.
- Format indicators: \"(Radio Edit)\", \"(Album Version)\", \"(Clean)\", etc.
- Streaming artifacts: \"(feat. [artist])\" when it should be \"feat. [artist]\"

REWRITE RULE SYNTAX:
Rewrite rules support both regex and literal string replacement:

IMPORTANT: All regex patterns MUST use anchors (^ and $) to match the entire input string.
All replacements reconstruct the complete output string using capture groups.

REGEX RULES (most common):
- Pattern: r\"^(.*)([0-9]{4}) Remaster(.*)$\" → Replacement: \"${1}${2} Version${3}\"
- Pattern: r\"^(.*) - [0-9]{4} Remaster$\" → Replacement: \"${1}\" (removes suffix)
- Pattern: r\"^(.+) ft\\. (.+)$\" → Replacement: \"${1} feat. ${2}\" (capture groups)
- Pattern: r\"^(.*\\S)\\s+$\" → Replacement: \"${1}\" (trim trailing whitespace)

LITERAL RULES (exact string matching):
- Pattern: \"feat.\" → Replacement: \"featuring\" (simple replacement)
- Pattern: \" ft. \" → Replacement: \" feat. \" (normalize featuring)

REGEX FLAGS (optional):
- 'i' = case insensitive
- 'w' = word boundaries (\\b...\\b)
- 's' = dot matches newline

FIELD TARGETS:
Rules can target: track_name, artist_name, album_name, album_artist_name

Examples of rule-worthy patterns (PRIORITIZE THESE):
- Remaster removal: r\"^(.*) - [0-9]{4} (Remaster|Version)$\" → \"${1}\"
- Remaster removal: r\"^(.*)\\s*\\([0-9]{4} Remaster\\)$\" → \"${1}\"
- Version removal: r\"^(.*)\\s*\\((Deluxe|Special|Anniversary) (Edition|Version)\\)$\" → \"${1}\"
- Edition removal: r\"^(.*)\\s*\\((Deluxe|Extended|Single)\\)$\" → \"${1}\"
- Format removal: r\"^(.*)\\s*\\((Radio Edit|Album Version|Clean)\\)$\" → \"${1}\"
- Year suffix removal: r\"^(.*) - [0-9]{4}$\" → \"${1}\"
- Featuring normalization: r\"^(.*) ft\\. (.*)$\" → \"${1} feat. ${2}\"
- Parenthetical featuring fix: r\"^(.*)\\s*\\(feat\\. (.*)\\)$\" → \"${1} feat. ${2}\"
- Whitespace cleanup: r\"^(.*)\\s{2,}(.*)$\" → \"${1} ${2}\"

GUIDELINES:
- Always use available functions - don't just provide text responses
- CHECK PENDING ITEMS: Review existing rewrite rules, pending edits, and pending rules to avoid duplicates
- DO NOT suggest edits for tracks that already have pending edits awaiting approval
- DO NOT propose rewrite rules that are already pending or similar to pending rules
- PRIORITIZE CLEANUP: Always suggest rules to remove remaster/version/edition information when found
- Focus on issues requiring musical knowledge or complex judgment for immediate fixes
- Suggest new rules for any consistent patterns you identify (only if not already pending)
- Only suggest changes when confident they improve metadata quality
- Consider original album/single releases when correcting compilations
- CLEAN TRACK NAMES: The goal is clean, canonical track names without extraneous suffixes or parentheticals

REWRITE RULE BEST PRACTICES:
- GENERIC RULES: Create rules that work across all artists, not artist-specific ones
- REPRESENTATIVE EXAMPLES: When suggesting rules, provide examples that clearly show the transformation
- GOOD EXAMPLE: \"Bohemian Rhapsody - 2011 Remaster\" → \"Bohemian Rhapsody\" (demonstrates remaster removal)
- BAD EXAMPLE: \"Hey Jude\" → \"Hey Jude\" (shows no change, not helpful)
- AVOID ARTIST-SPECIFIC: Don't create rules like \"Beatles\" → \"The Beatles\" unless specifically correcting misspellings
- PATTERN FOCUS: Rules should target formatting patterns (remasters, editions, etc.) not content-specific changes
- MOTIVATION CLARITY: Explain WHY the rule helps (\"Removes distracting remaster suffixes for cleaner track names\")

EXAMPLE QUALITY:
When providing examples in your motivation, choose tracks that actually demonstrate the rule's effect:
- Show the BEFORE and AFTER transformation clearly
- Pick common scenarios where the rule would apply
- Use diverse examples (different genres/eras) to show broad applicability
- Make it obvious why the change improves the metadata

Help build a smarter cleaning system by identifying both immediate fixes AND patterns for future automation!";

/// Configuration for the OpenAI provider.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpenAIProviderConfig {
    /// OpenAI API key
    pub api_key: String,
    /// Model to use (defaults to gpt-4o-mini)
    pub model: Option<String>,
    /// Custom system prompt (defaults to [`DEFAULT_SYSTEM_PROMPT`])
    pub system_prompt: Option<String>,
}

#[derive(Deserialize)]
struct ScrobbleEditWithIndex {
    track_index: usize,
    track_name: Option<String>,
    artist_name: Option<String>,
    album_name: Option<String>,
    album_artist_name: Option<String>,
    #[allow(dead_code)]
    reason: String,
}

#[derive(Deserialize)]
struct RewriteRuleSuggestionWithIndex {
    track_index: usize,
    track_name: Option<SdRuleData>,
    album_name: Option<SdRuleData>,
    artist_name: Option<SdRuleData>,
    album_artist_name: Option<SdRuleData>,
    requires_confirmation: Option<bool>,
    motivation: String,
}

/// OpenAI-based action provider using function calling
pub struct OpenAIScrubActionProvider {
    client: Arc<Mutex<OpenAIClient>>,
    model: String,
    system_prompt: String,
    rewrite_rules: Vec<RewriteRule>,
    rule_focus_mode: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RewriteRuleSuggestion {
    /// Optional transformation for track name
    track_name: Option<SdRuleData>,
    /// Optional transformation for album name
    album_name: Option<SdRuleData>,
    /// Optional transformation for artist name
    artist_name: Option<SdRuleData>,
    /// Optional transformation for album artist name
    album_artist_name: Option<SdRuleData>,
    /// Whether this rule requires user confirmation before applying
    requires_confirmation: bool,
    /// Explanation of why this rule would be helpful
    motivation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SdRuleData {
    /// The pattern to search for (regex)
    find: String,
    /// The replacement string (supports $1, $2, ${named}, etc.)
    replace: String,
    /// Regex flags (e.g., "i" for case insensitive)
    flags: Option<String>,
}

impl From<SdRuleData> for crate::rewrite::SdRule {
    fn from(data: SdRuleData) -> Self {
        let mut sd_rule = crate::rewrite::SdRule::new(&data.find, &data.replace);

        if let Some(flags) = &data.flags {
            sd_rule = sd_rule.with_flags(flags);
        }

        sd_rule
    }
}

impl RewriteRuleSuggestion {
    /// Convert this suggestion into a `RewriteRule` and motivation pair
    fn into_rule_and_motivation(self) -> (RewriteRule, String) {
        let mut rule = RewriteRule::new();

        if let Some(track_rule) = self.track_name {
            rule = rule.with_track_name(track_rule.into());
        }

        if let Some(album_rule) = self.album_name {
            rule = rule.with_album_name(album_rule.into());
        }

        if let Some(artist_rule) = self.artist_name {
            rule = rule.with_artist_name(artist_rule.into());
        }

        if let Some(album_artist_rule) = self.album_artist_name {
            rule = rule.with_album_artist_name(album_artist_rule.into());
        }

        rule = rule.with_confirmation_required(self.requires_confirmation);

        (rule, self.motivation)
    }
}

impl OpenAIScrubActionProvider {
    pub fn new(
        api_key: String,
        model: Option<String>,
        system_prompt: Option<String>,
        rewrite_rules: Vec<RewriteRule>,
    ) -> Result<Self, ActionProviderError> {
        let client = OpenAIClient::builder()
            .with_api_key(api_key)
            .build()
            .map_err(|e| ActionProviderError(format!("Failed to create OpenAI client: {e}")))?;

        let model = match model.as_deref() {
            Some("gpt-4") => "gpt-4".to_string(),
            Some("gpt-4-turbo") => "gpt-4-turbo".to_string(),
            Some("gpt-4o") => GPT4_O.to_string(),
            Some("gpt-4o-mini") => "gpt-4o-mini".to_string(),
            Some("gpt-3.5-turbo") => "gpt-3.5-turbo".to_string(),
            _ => "gpt-4o-mini".to_string(), // default to GPT-4o mini
        };

        let system_prompt = system_prompt.unwrap_or_else(|| DEFAULT_SYSTEM_PROMPT.to_string());

        Ok(Self {
            client: Arc::new(Mutex::new(client)),
            model,
            system_prompt,
            rewrite_rules,
            rule_focus_mode: false,
        })
    }

    /// Build a provider from an [`OpenAIProviderConfig`].
    pub fn from_config(
        config: OpenAIProviderConfig,
        rewrite_rules: Vec<RewriteRule>,
    ) -> Result<Self, ActionProviderError> {
        Self::new(
            config.api_key,
            config.model,
            config.system_prompt,
            rewrite_rules,
        )
    }

    /// Enable rule focus mode for pattern analysis
    pub fn enable_rule_focus_mode(&mut self) {
        self.rule_focus_mode = true;
    }

    /// Get the effective system prompt based on current mode
    fn get_effective_system_prompt(&self) -> String {
        if self.rule_focus_mode {
            format!(
                "{}\n\nIMPORTANT: You are in PATTERN ANALYSIS MODE. Your primary goal is to identify patterns across many tracks and suggest rewrite rules that can systematically clean similar issues. Focus heavily on proposing rewrite rules rather than individual track edits. Look for common patterns like:\n- Remastered/version information that should be removed\n- Featuring/collaboration notation that should be standardized\n- Brackets, parentheses, or other formatting inconsistencies\n- Common misspellings or variations in artist/album names\n\nWhen you see the same type of issue across multiple tracks, always prefer suggesting a rewrite rule over individual edits.",
                self.system_prompt
            )
        } else {
            self.system_prompt.clone()
        }
    }

    fn create_edit_function_properties() -> HashMap<String, Box<JSONSchemaDefine>> {
        let mut properties = HashMap::new();

        properties.insert(
            "track_name".to_string(),
            Box::new(JSONSchemaDefine {
                schema_type: Some(JSONSchemaType::String),
                description: Some("The corrected track name".to_string()),
                enum_values: None,
                properties: None,
                required: None,
                items: None,
            }),
        );

        properties.insert(
            "artist_name".to_string(),
            Box::new(JSONSchemaDefine {
                schema_type: Some(JSONSchemaType::String),
                description: Some("The corrected artist name".to_string()),
                enum_values: None,
                properties: None,
                required: None,
                items: None,
            }),
        );

        properties.insert(
            "album_name".to_string(),
            Box::new(JSONSchemaDefine {
                schema_type: Some(JSONSchemaType::String),
                description: Some("The corrected album name".to_string()),
                enum_values: None,
                properties: None,
                required: None,
                items: None,
            }),
        );

        properties.insert(
            "album_artist_name".to_string(),
            Box::new(JSONSchemaDefine {
                schema_type: Some(JSONSchemaType::String),
                description: Some("The corrected album artist name".to_string()),
                enum_values: None,
                properties: None,
                required: None,
                items: None,
            }),
        );

        properties.insert(
            "reason".to_string(),
            Box::new(JSONSchemaDefine {
                schema_type: Some(JSONSchemaType::String),
                description: Some("Brief explanation of why this change is suggested".to_string()),
                enum_values: None,
                properties: None,
                required: None,
                items: None,
            }),
        );

        properties
    }

    fn create_sd_rule_properties() -> HashMap<String, Box<JSONSchemaDefine>> {
        let mut properties = HashMap::new();

        properties.insert(
            "find".to_string(),
            Box::new(JSONSchemaDefine {
                schema_type: Some(JSONSchemaType::String),
                description: Some("The pattern to search for (regex)".to_string()),
                enum_values: None,
                properties: None,
                required: None,
                items: None,
            }),
        );

        properties.insert(
            "replace".to_string(),
            Box::new(JSONSchemaDefine {
                schema_type: Some(JSONSchemaType::String),
                description: Some(
                    "The replacement string (supports $1, $2, ${named}, etc.)".to_string(),
                ),
                enum_values: None,
                properties: None,
                required: None,
                items: None,
            }),
        );

        properties.insert(
            "flags".to_string(),
            Box::new(JSONSchemaDefine {
                schema_type: Some(JSONSchemaType::String),
                description: Some("Regex flags (e.g., \"i\" for case insensitive)".to_string()),
                enum_values: None,
                properties: None,
                required: None,
                items: None,
            }),
        );

        properties.insert(
            "max_replacements".to_string(),
            Box::new(JSONSchemaDefine {
                schema_type: Some(JSONSchemaType::Number),
                description: Some("Maximum number of replacements (0 = unlimited)".to_string()),
                enum_values: None,
                properties: None,
                required: None,
                items: None,
            }),
        );

        properties
    }

    fn create_rule_function_properties() -> HashMap<String, Box<JSONSchemaDefine>> {
        let mut properties = HashMap::new();

        properties.insert(
            "track_name".to_string(),
            Box::new(JSONSchemaDefine {
                schema_type: Some(JSONSchemaType::Object),
                description: Some("Optional transformation for track name".to_string()),
                enum_values: None,
                properties: Some(Self::create_sd_rule_properties()),
                required: Some(vec!["find".to_string(), "replace".to_string()]),
                items: None,
            }),
        );

        properties.insert(
            "album_name".to_string(),
            Box::new(JSONSchemaDefine {
                schema_type: Some(JSONSchemaType::Object),
                description: Some("Optional transformation for album name".to_string()),
                enum_values: None,
                properties: Some(Self::create_sd_rule_properties()),
                required: Some(vec!["find".to_string(), "replace".to_string()]),
                items: None,
            }),
        );

        properties.insert(
            "artist_name".to_string(),
            Box::new(JSONSchemaDefine {
                schema_type: Some(JSONSchemaType::Object),
                description: Some("Optional transformation for artist name".to_string()),
                enum_values: None,
                properties: Some(Self::create_sd_rule_properties()),
                required: Some(vec!["find".to_string(), "replace".to_string()]),
                items: None,
            }),
        );

        properties.insert(
            "album_artist_name".to_string(),
            Box::new(JSONSchemaDefine {
                schema_type: Some(JSONSchemaType::Object),
                description: Some("Optional transformation for album artist name".to_string()),
                enum_values: None,
                properties: Some(Self::create_sd_rule_properties()),
                required: Some(vec!["find".to_string(), "replace".to_string()]),
                items: None,
            }),
        );

        properties.insert(
            "requires_confirmation".to_string(),
            Box::new(JSONSchemaDefine {
                schema_type: Some(JSONSchemaType::Boolean),
                description: Some(
                    "Whether this rule requires user confirmation before applying".to_string(),
                ),
                enum_values: None,
                properties: None,
                required: None,
                items: None,
            }),
        );

        properties.insert(
            "motivation".to_string(),
            Box::new(JSONSchemaDefine {
                schema_type: Some(JSONSchemaType::String),
                description: Some("Explanation of why this rule would be helpful".to_string()),
                enum_values: None,
                properties: None,
                required: None,
                items: None,
            }),
        );

        properties
    }

    fn format_existing_rules(&self) -> String {
        if self.rewrite_rules.is_empty() {
            return "EXISTING REWRITE RULES: None configured yet.".to_string();
        }

        match serde_json::to_string_pretty(&self.rewrite_rules) {
            Ok(json) => format!("EXISTING REWRITE RULES:\n{json}"),
            Err(_) => "EXISTING REWRITE RULES: (serialization error)".to_string(),
        }
    }

    /// Format open edit intents for prompt context (dedup hints for the model).
    fn format_open_intents(open_intents: Option<&[EditIntent]>) -> String {
        match open_intents {
            None | Some([]) => "PENDING EDITS: None".to_string(),
            Some(intents) => {
                let edits_list = intents
                    .iter()
                    .map(|intent| {
                        let mut changes = Vec::new();
                        if let Some(track_name) = &intent.proposed.track_name {
                            if *track_name != intent.subject.track {
                                changes.push(format!("track → \"{track_name}\""));
                            }
                        }
                        if intent.proposed.artist_name != intent.subject.artist {
                            changes.push(format!("artist → \"{}\"", intent.proposed.artist_name));
                        }
                        if let Some(album_name) = &intent.proposed.album_name {
                            if intent.subject.album.as_deref() != Some(album_name.as_str()) {
                                changes.push(format!("album → \"{album_name}\""));
                            }
                        }
                        let change_summary = if changes.is_empty() {
                            "changes pending approval".to_string()
                        } else {
                            changes.join(", ")
                        };
                        format!(
                            "- \"{}\" by \"{}\" → {} [provider: {}]",
                            intent.subject.track,
                            intent.subject.artist,
                            change_summary,
                            intent.provider
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("PENDING EDITS (already suggested, avoid duplicates):\n{edits_list}")
            }
        }
    }

    /// Format pending rewrite-rule proposals for prompt context.
    fn format_pending_rules(pending_rules: Option<&[PendingRule]>) -> String {
        match pending_rules {
            None | Some([]) => "PENDING REWRITE RULES: None".to_string(),
            Some(rules) => {
                let rules_list = rules
                    .iter()
                    .map(|pending| match &pending.example {
                        Some(example) => format!(
                            "- {} (triggered by: \"{}\" by \"{}\")",
                            pending.motivation, example.track, example.artist
                        ),
                        None => format!("- {}", pending.motivation),
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                format!(
                    "PENDING REWRITE RULES (already suggested, avoid duplicates):\n{rules_list}"
                )
            }
        }
    }

    fn format_tracks_info(tracks: &[Track]) -> String {
        tracks
            .iter()
            .enumerate()
            .map(|(idx, track)| {
                let album_info = if let Some(album) = &track.album {
                    format!(" from album \"{album}\"")
                } else {
                    " (no album info)".to_string()
                };
                let timestamp_info = if let Some(timestamp) = track.timestamp {
                    format!(" [scrobbled: {timestamp}]")
                } else {
                    String::new()
                };
                format!(
                    "Track {}: \"{}\" by \"{}\"{}{} (play count: {})",
                    idx, track.name, track.artist, album_info, timestamp_info, track.playcount
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn process_track_edit_suggestion(
        &self,
        arguments: &str,
        tracks: &[Track],
    ) -> Result<(usize, ScrubActionSuggestion), ActionProviderError> {
        let args: ScrobbleEditWithIndex = serde_json::from_str(arguments)
            .map_err(|e| ActionProviderError(format!("Failed to parse function arguments: {e}")))?;

        if args.track_index >= tracks.len() {
            return Err(ActionProviderError(format!(
                "Invalid track index {} for batch size {}",
                args.track_index,
                tracks.len()
            )));
        }

        let track = &tracks[args.track_index];
        let mut edit = crate::rewrite::create_no_op_edit(track);

        if let Some(track_name) = args.track_name {
            edit.track_name = Some(track_name);
        }
        if let Some(artist_name) = args.artist_name {
            edit.artist_name = artist_name;
        }
        if let Some(album_name) = args.album_name {
            edit.album_name = Some(album_name);
        }
        if let Some(album_artist_name) = args.album_artist_name {
            edit.album_artist_name = Some(album_artist_name);
        }

        Ok((
            args.track_index,
            ScrubActionSuggestion::Edit(Box::new(edit)),
        ))
    }

    fn process_rewrite_rule_suggestion(
        &self,
        arguments: &str,
        tracks: &[Track],
    ) -> Result<(usize, ScrubActionSuggestion), ActionProviderError> {
        let args: RewriteRuleSuggestionWithIndex =
            serde_json::from_str(arguments).map_err(|e| {
                ActionProviderError(format!("Failed to parse rewrite rule arguments: {e}"))
            })?;

        if args.track_index >= tracks.len() {
            return Err(ActionProviderError(format!(
                "Invalid track index {} for batch size {}",
                args.track_index,
                tracks.len()
            )));
        }

        let suggestion = RewriteRuleSuggestion {
            track_name: args.track_name,
            album_name: args.album_name,
            artist_name: args.artist_name,
            album_artist_name: args.album_artist_name,
            requires_confirmation: args.requires_confirmation.unwrap_or(false),
            motivation: args.motivation.clone(),
        };

        let (rule, motivation) = suggestion.into_rule_and_motivation();
        Ok((
            args.track_index,
            ScrubActionSuggestion::ProposeRule {
                rule: Box::new(rule),
                motivation,
            },
        ))
    }

    fn add_suggestion_to_results(
        results: &mut Vec<(usize, Vec<ScrubActionSuggestion>)>,
        track_index: usize,
        suggestion: ScrubActionSuggestion,
    ) {
        if let Some(existing) = results.iter_mut().find(|(idx, _)| *idx == track_index) {
            existing.1.push(suggestion);
        } else {
            results.push((track_index, vec![suggestion]));
        }
    }

    fn process_tool_calls(
        &self,
        response: &openai_api_rs::v1::chat_completion::ChatCompletionResponse,
        tracks: &[Track],
        results: &mut Vec<(usize, Vec<ScrubActionSuggestion>)>,
    ) -> Result<(), ActionProviderError> {
        let Some(choice) = response.choices.first() else {
            return Ok(());
        };

        let Some(tool_calls) = &choice.message.tool_calls else {
            return Ok(());
        };

        for tool_call in tool_calls {
            let Some(name) = &tool_call.function.name else {
                continue;
            };

            let Some(arguments) = &tool_call.function.arguments else {
                continue;
            };

            match name.as_str() {
                "suggest_track_edit" => {
                    match self.process_track_edit_suggestion(arguments, tracks) {
                        Ok((track_index, suggestion)) => {
                            Self::add_suggestion_to_results(results, track_index, suggestion);
                        }
                        Err(e) => {
                            log::warn!("Failed to process track edit suggestion: {e}");
                        }
                    }
                }
                "suggest_rewrite_rule" => {
                    match self.process_rewrite_rule_suggestion(arguments, tracks) {
                        Ok((track_index, suggestion)) => {
                            Self::add_suggestion_to_results(results, track_index, suggestion);
                        }
                        Err(e) => {
                            log::warn!("Failed to process rewrite rule suggestion: {e}");
                        }
                    }
                }
                _ => {
                    log::warn!("Unknown function call: {name}");
                }
            }
        }

        Ok(())
    }

    /// Common OpenAI request logic
    async fn make_openai_request(
        &self,
        user_message: &str,
        tracks: &[Track],
    ) -> Result<Vec<(usize, Vec<ScrubActionSuggestion>)>, ActionProviderError> {
        // Add track_index parameter to edit function
        let mut edit_properties = Self::create_edit_function_properties();
        edit_properties.insert(
            "track_index".to_string(),
            Box::new(JSONSchemaDefine {
                schema_type: Some(JSONSchemaType::Number),
                description: Some(
                    "Index of the track this suggestion applies to (0-based)".to_string(),
                ),
                enum_values: None,
                properties: None,
                required: None,
                items: None,
            }),
        );

        let suggest_edit_function = Function {
            name: "suggest_track_edit".to_string(),
            description: Some(
                "Suggest metadata corrections for a specific track from the batch".to_string(),
            ),
            parameters: FunctionParameters {
                schema_type: JSONSchemaType::Object,
                properties: Some(edit_properties),
                required: Some(vec!["track_index".to_string(), "reason".to_string()]),
            },
        };

        // Add track_index parameter to rule function
        let mut rule_properties = Self::create_rule_function_properties();
        rule_properties.insert(
            "track_index".to_string(),
            Box::new(JSONSchemaDefine {
                schema_type: Some(JSONSchemaType::Number),
                description: Some(
                    "Index of the track that triggered this rule suggestion (0-based)".to_string(),
                ),
                enum_values: None,
                properties: None,
                required: None,
                items: None,
            }),
        );

        let suggest_rule_function = Function {
            name: "suggest_rewrite_rule".to_string(),
            description: Some(
                "Propose a new rewrite rule based on patterns found in the tracks".to_string(),
            ),
            parameters: FunctionParameters {
                schema_type: JSONSchemaType::Object,
                properties: Some(rule_properties),
                required: Some(vec!["track_index".to_string(), "motivation".to_string()]),
            },
        };

        let req = ChatCompletionRequest::new(
            self.model.clone(),
            vec![
                chat_completion::ChatCompletionMessage {
                    role: chat_completion::MessageRole::system,
                    content: chat_completion::Content::Text(self.get_effective_system_prompt()),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
                chat_completion::ChatCompletionMessage {
                    role: chat_completion::MessageRole::user,
                    content: chat_completion::Content::Text(user_message.to_string()),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
            ],
        )
        .tools(vec![
            Tool {
                r#type: ToolType::Function,
                function: suggest_edit_function,
            },
            Tool {
                r#type: ToolType::Function,
                function: suggest_rule_function,
            },
        ])
        .tool_choice(ToolChoiceType::Auto);

        // Log the request being sent to OpenAI
        log::info!(
            "Making OpenAI request for {} tracks: {}",
            tracks.len(),
            tracks
                .iter()
                .map(|t| format!("\"{}\" by \"{}\"", t.name, t.artist))
                .collect::<Vec<_>>()
                .join(", ")
        );

        let response = self
            .client
            .lock()
            .await
            .chat_completion(req)
            .await
            .map_err(|e| ActionProviderError(format!("OpenAI API error: {e}")))?;

        // Log OpenAI response details
        let tool_calls_count = response
            .choices
            .first()
            .and_then(|choice| choice.message.tool_calls.as_ref())
            .map(|calls| calls.len())
            .unwrap_or(0);
        log::info!("OpenAI response received with {tool_calls_count} tool calls");

        // Log the full response for debugging
        if let Ok(response_json) = serde_json::to_string_pretty(&response) {
            log::debug!("OpenAI response: {response_json}");
        }

        // Log individual tool calls for easier debugging
        if let Some(choice) = response.choices.first() {
            if let Some(tool_calls) = &choice.message.tool_calls {
                for (i, tool_call) in tool_calls.iter().enumerate() {
                    log::info!(
                        "Tool call {}: {} with args: {}",
                        i + 1,
                        tool_call.function.name.as_deref().unwrap_or("unknown"),
                        tool_call.function.arguments.as_deref().unwrap_or("none")
                    );
                }
            }
        }

        let mut results: Vec<(usize, Vec<ScrubActionSuggestion>)> = Vec::new();

        // Process the response
        self.process_tool_calls(&response, tracks, &mut results)?;

        Ok(results)
    }
}

#[async_trait]
impl ScrubActionProvider for OpenAIScrubActionProvider {
    type Error = ActionProviderError;

    async fn analyze_tracks(
        &self,
        tracks: &[Track],
        open_intents: Option<&[EditIntent]>,
        pending_rules: Option<&[PendingRule]>,
    ) -> Result<Vec<(usize, Vec<SuggestionWithContext>)>, Self::Error> {
        if tracks.is_empty() {
            return Ok(Vec::new());
        }

        let existing_rules = self.format_existing_rules();
        let tracks_info = Self::format_tracks_info(tracks);
        let pending_edits_info = Self::format_open_intents(open_intents);
        let pending_rules_info = Self::format_pending_rules(pending_rules);

        let user_message = format!(
            "Analyze these Last.fm scrobbles and provide suggestions for each track that needs improvement.\n\nIMPORTANT: Check the pending items below to avoid suggesting duplicates.\n\n{tracks_info}\n\n{existing_rules}\n\n{pending_edits_info}\n\n{pending_rules_info}"
        );

        let results = self.make_openai_request(&user_message, tracks).await?;

        // Convert ScrubActionSuggestion to SuggestionWithContext
        let converted_results = results
            .into_iter()
            .map(|(index, suggestions)| {
                let wrapped_suggestions = suggestions
                    .into_iter()
                    .map(|suggestion| {
                        SuggestionWithContext::new(
                            suggestion,
                            false, // OpenAI suggestions generally don't require confirmation by default
                            self.provider_name().to_string(),
                        )
                    })
                    .collect();
                (index, wrapped_suggestions)
            })
            .collect();

        Ok(converted_results)
    }

    fn provider_name(&self) -> &'static str {
        "openai"
    }
}
