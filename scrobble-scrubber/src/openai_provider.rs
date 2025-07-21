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

use crate::rewrite::RewriteRule;

use crate::config::DEFAULT_CLAUDE_SYSTEM_PROMPT;
use crate::scrub_action_provider::{
    ActionProviderError, ScrubActionProvider, ScrubActionSuggestion,
};

/// OpenAI-based action provider using function calling
pub struct OpenAIScrubActionProvider {
    client: Arc<Mutex<OpenAIClient>>,
    model: String,
    system_prompt: String,
    rewrite_rules: Vec<RewriteRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TrackEditSuggestion {
    /// The corrected track name (only if it needs changing)
    new_track_name: Option<String>,
    /// The corrected artist name (only if it needs changing)
    new_artist_name: Option<String>,
    /// The corrected album name (only if it needs changing)
    new_album_name: Option<String>,
    /// The corrected album artist name (only if it needs changing)
    new_album_artist_name: Option<String>,
    /// Brief explanation of why this change is suggested
    reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RewriteRuleSuggestion {
    /// The pattern to search for (regex by default, or literal if is_literal is true)
    find: String,
    /// The replacement string (supports $1, $2, ${named}, etc.)
    replace: String,
    /// The field to target: track_name, artist_name, album_name, or album_artist_name
    target_field: String,
    /// Whether to use literal string matching instead of regex
    is_literal: bool,
    /// Regex flags (e.g., "i" for case insensitive)
    flags: Option<String>,
    /// Maximum number of replacements (0 = unlimited)
    max_replacements: usize,
    /// Explanation of why this rule would be helpful
    motivation: String,
}

impl RewriteRuleSuggestion {
    /// Convert this suggestion into a RewriteRule and motivation pair
    fn into_rule_and_motivation(self) -> Result<(RewriteRule, String), ActionProviderError> {
        let mut rule = RewriteRule::new();
        let mut sd_rule = if self.is_literal {
            crate::rewrite::SdRule::new_literal(&self.find, &self.replace)
        } else {
            crate::rewrite::SdRule::new_regex(&self.find, &self.replace)
        };

        if let Some(flags) = &self.flags {
            sd_rule = sd_rule.with_flags(flags);
        }

        if self.max_replacements > 0 {
            sd_rule = sd_rule.with_max_replacements(self.max_replacements);
        }

        match self.target_field.as_str() {
            "track_name" => rule = rule.with_track_name(sd_rule),
            "artist_name" => rule = rule.with_artist_name(sd_rule),
            "album_name" => rule = rule.with_album_name(sd_rule),
            "album_artist_name" => rule = rule.with_album_artist_name(sd_rule),
            _ => {
                return Err(ActionProviderError(format!(
                    "Invalid target field: {}",
                    self.target_field
                )));
            }
        }

        Ok((rule, self.motivation))
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
            .map_err(|e| ActionProviderError(format!("Failed to create OpenAI client: {}", e)))?;

        let model = match model.as_deref() {
            Some("gpt-4") => "gpt-4".to_string(),
            Some("gpt-4-turbo") => "gpt-4-turbo".to_string(),
            Some("gpt-4o") => GPT4_O.to_string(),
            Some("gpt-4o-mini") => "gpt-4o-mini".to_string(),
            Some("gpt-3.5-turbo") => "gpt-3.5-turbo".to_string(),
            _ => GPT4_O.to_string(), // default to GPT-4o
        };

        let system_prompt =
            system_prompt.unwrap_or_else(|| DEFAULT_CLAUDE_SYSTEM_PROMPT.to_string());

        Ok(Self {
            client: Arc::new(Mutex::new(client)),
            model,
            system_prompt,
            rewrite_rules,
        })
    }

    fn create_edit_function_properties() -> HashMap<String, Box<JSONSchemaDefine>> {
        let mut properties = HashMap::new();

        properties.insert(
            "new_track_name".to_string(),
            Box::new(JSONSchemaDefine {
                schema_type: Some(JSONSchemaType::String),
                description: Some(
                    "The corrected track name (only if it needs changing)".to_string(),
                ),
                enum_values: None,
                properties: None,
                required: None,
                items: None,
            }),
        );

        properties.insert(
            "new_artist_name".to_string(),
            Box::new(JSONSchemaDefine {
                schema_type: Some(JSONSchemaType::String),
                description: Some(
                    "The corrected artist name (only if it needs changing)".to_string(),
                ),
                enum_values: None,
                properties: None,
                required: None,
                items: None,
            }),
        );

        properties.insert(
            "new_album_name".to_string(),
            Box::new(JSONSchemaDefine {
                schema_type: Some(JSONSchemaType::String),
                description: Some(
                    "The corrected album name (only if it needs changing)".to_string(),
                ),
                enum_values: None,
                properties: None,
                required: None,
                items: None,
            }),
        );

        properties.insert(
            "new_album_artist_name".to_string(),
            Box::new(JSONSchemaDefine {
                schema_type: Some(JSONSchemaType::String),
                description: Some(
                    "The corrected album artist name (only if it needs changing)".to_string(),
                ),
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

    fn create_rule_function_properties() -> HashMap<String, Box<JSONSchemaDefine>> {
        let mut properties = HashMap::new();

        properties.insert(
            "find".to_string(),
            Box::new(JSONSchemaDefine {
                schema_type: Some(JSONSchemaType::String),
                description: Some("The pattern to search for (regex by default, or literal if is_literal is true)".to_string()),
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
            "target_field".to_string(),
            Box::new(JSONSchemaDefine {
                schema_type: Some(JSONSchemaType::String),
                description: Some("The field to target: track_name, artist_name, album_name, or album_artist_name".to_string()),
                enum_values: Some(vec![
                    "track_name".to_string(),
                    "artist_name".to_string(),
                    "album_name".to_string(),
                    "album_artist_name".to_string(),
                ]),
                properties: None,
                required: None,
                items: None,
            }),
        );

        properties.insert(
            "is_literal".to_string(),
            Box::new(JSONSchemaDefine {
                schema_type: Some(JSONSchemaType::Boolean),
                description: Some(
                    "Whether to use literal string matching instead of regex".to_string(),
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
            Ok(json) => format!("EXISTING REWRITE RULES:\n{}", json),
            Err(_) => "EXISTING REWRITE RULES: (serialization error)".to_string(),
        }
    }
}

#[async_trait]
impl ScrubActionProvider for OpenAIScrubActionProvider {
    type Error = ActionProviderError;

    async fn analyze_tracks(
        &self,
        tracks: &[Track],
    ) -> Result<Vec<(usize, Vec<ScrubActionSuggestion>)>, Self::Error> {
        if tracks.is_empty() {
            return Ok(Vec::new());
        }

        let existing_rules = self.format_existing_rules();

        // Create a message that includes all tracks for batch analysis
        let tracks_info = tracks
            .iter()
            .enumerate()
            .map(|(idx, track)| {
                format!(
                    "Track {}: \"{}\" by \"{}\" (play count: {})",
                    idx, track.name, track.artist, track.playcount
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let user_message = format!(
            "Analyze these Last.fm scrobbles and provide suggestions for each track that needs improvement:\n\n{}\n\n{}",
            tracks_info, existing_rules
        );

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
                required: Some(vec![
                    "track_index".to_string(),
                    "find".to_string(),
                    "replace".to_string(),
                    "target_field".to_string(),
                    "is_literal".to_string(),
                    "motivation".to_string(),
                ]),
            },
        };

        let no_action_function = Function {
            name: "no_action_needed".to_string(),
            description: Some(
                "Indicate that no changes are needed for any of the tracks".to_string(),
            ),
            parameters: FunctionParameters {
                schema_type: JSONSchemaType::Object,
                properties: None,
                required: None,
            },
        };

        let req = ChatCompletionRequest::new(
            self.model.clone(),
            vec![
                chat_completion::ChatCompletionMessage {
                    role: chat_completion::MessageRole::system,
                    content: chat_completion::Content::Text(self.system_prompt.clone()),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
                chat_completion::ChatCompletionMessage {
                    role: chat_completion::MessageRole::user,
                    content: chat_completion::Content::Text(user_message),
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
            Tool {
                r#type: ToolType::Function,
                function: no_action_function,
            },
        ])
        .tool_choice(ToolChoiceType::Auto);

        let response = self
            .client
            .lock()
            .await
            .chat_completion(req)
            .await
            .map_err(|e| ActionProviderError(format!("OpenAI API error: {}", e)))?;

        let mut results: Vec<(usize, Vec<ScrubActionSuggestion>)> = Vec::new();

        // Process the response
        if let Some(choice) = response.choices.first() {
            if let Some(tool_calls) = &choice.message.tool_calls {
                for tool_call in tool_calls {
                    if let Some(ref name) = tool_call.function.name {
                        match name.as_str() {
                            "suggest_track_edit" => {
                                if let Some(ref arguments) = tool_call.function.arguments {
                                    #[derive(Deserialize)]
                                    struct TrackEditSuggestionWithIndex {
                                        track_index: usize,
                                        new_track_name: Option<String>,
                                        new_artist_name: Option<String>,
                                        new_album_name: Option<String>,
                                        new_album_artist_name: Option<String>,
                                        #[allow(dead_code)]
                                        reason: String,
                                    }

                                    let args: TrackEditSuggestionWithIndex =
                                        serde_json::from_str(arguments).map_err(|e| {
                                            ActionProviderError(format!(
                                                "Failed to parse function arguments: {}",
                                                e
                                            ))
                                        })?;

                                    if args.track_index >= tracks.len() {
                                        log::warn!(
                                            "Invalid track index {} for batch size {}",
                                            args.track_index,
                                            tracks.len()
                                        );
                                        continue;
                                    }

                                    // Create a ScrobbleEdit with the suggested changes
                                    let track = &tracks[args.track_index];
                                    let mut edit = crate::rewrite::create_no_op_edit(track);

                                    if let Some(new_name) = args.new_track_name {
                                        edit.track_name = new_name;
                                    }

                                    if let Some(new_artist) = args.new_artist_name {
                                        edit.artist_name = new_artist.clone();
                                        edit.album_artist_name = new_artist; // also update album artist
                                    }

                                    if let Some(new_album) = args.new_album_name {
                                        edit.album_name = new_album;
                                    }

                                    if let Some(new_album_artist) = args.new_album_artist_name {
                                        edit.album_artist_name = new_album_artist;
                                    }

                                    // Add to results
                                    if let Some(existing) =
                                        results.iter_mut().find(|(idx, _)| *idx == args.track_index)
                                    {
                                        existing.1.push(ScrubActionSuggestion::Edit(edit));
                                    } else {
                                        results.push((
                                            args.track_index,
                                            vec![ScrubActionSuggestion::Edit(edit)],
                                        ));
                                    }
                                }
                            }
                            "suggest_rewrite_rule" => {
                                if let Some(ref arguments) = tool_call.function.arguments {
                                    #[derive(Deserialize)]
                                    struct RewriteRuleSuggestionWithIndex {
                                        track_index: usize,
                                        find: String,
                                        replace: String,
                                        target_field: String,
                                        is_literal: bool,
                                        flags: Option<String>,
                                        max_replacements: Option<usize>,
                                        motivation: String,
                                    }

                                    let args: RewriteRuleSuggestionWithIndex =
                                        serde_json::from_str(arguments).map_err(|e| {
                                            ActionProviderError(format!(
                                                "Failed to parse rewrite rule arguments: {}",
                                                e
                                            ))
                                        })?;

                                    if args.track_index >= tracks.len() {
                                        log::warn!(
                                            "Invalid track index {} for batch size {}",
                                            args.track_index,
                                            tracks.len()
                                        );
                                        continue;
                                    }

                                    // Create a RewriteRule from the suggestion
                                    let mut rule = crate::rewrite::RewriteRule::new();
                                    let mut sd_rule = if args.is_literal {
                                        crate::rewrite::SdRule::new_literal(
                                            &args.find,
                                            &args.replace,
                                        )
                                    } else {
                                        crate::rewrite::SdRule::new_regex(
                                            &args.find,
                                            &args.replace,
                                        )
                                    };

                                    if let Some(flags) = &args.flags {
                                        sd_rule = sd_rule.with_flags(flags);
                                    }

                                    if let Some(max_replacements) = args.max_replacements {
                                        if max_replacements > 0 {
                                            sd_rule = sd_rule.with_max_replacements(max_replacements);
                                        }
                                    }

                                    match args.target_field.as_str() {
                                        "track_name" => rule = rule.with_track_name(sd_rule),
                                        "artist_name" => rule = rule.with_artist_name(sd_rule),
                                        "album_name" => rule = rule.with_album_name(sd_rule),
                                        "album_artist_name" => {
                                            rule = rule.with_album_artist_name(sd_rule)
                                        }
                                        _ => {
                                            log::warn!(
                                                "Invalid target field: {}",
                                                args.target_field
                                            );
                                            continue;
                                        }
                                    }

                                    // Add to results
                                    if let Some(existing) =
                                        results.iter_mut().find(|(idx, _)| *idx == args.track_index)
                                    {
                                        existing.1.push(ScrubActionSuggestion::ProposeRule {
                                            rule,
                                            motivation: args.motivation,
                                        });
                                    } else {
                                        results.push((
                                            args.track_index,
                                            vec![ScrubActionSuggestion::ProposeRule {
                                                rule,
                                                motivation: args.motivation,
                                            }],
                                        ));
                                    }
                                }
                            }
                            "no_action_needed" => {
                                // Do nothing - no suggestions to add
                            }
                            _ => {
                                log::warn!("Unknown function call: {}", name);
                                continue;
                            }
                        }
                    }
                }
            }
        }

        Ok(results)
    }

    fn provider_name(&self) -> &str {
        "OpenAI"
    }
}
