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
    /// The regex pattern or literal string to match
    pattern: String,
    /// The replacement string (may include capture groups like $1, $2)
    replacement: String,
    /// The field to target: track_name, artist_name, album_name, or album_artist_name
    target_field: String,
    /// Whether this is a regex pattern (true) or literal string (false)
    is_regex: bool,
    /// Optional regex flags (i, w, s)
    regex_flags: Option<String>,
    /// Explanation of why this rule would be helpful
    motivation: String,
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
            "pattern".to_string(),
            Box::new(JSONSchemaDefine {
                schema_type: Some(JSONSchemaType::String),
                description: Some("The regex pattern or literal string to match".to_string()),
                enum_values: None,
                properties: None,
                required: None,
                items: None,
            }),
        );

        properties.insert(
            "replacement".to_string(),
            Box::new(JSONSchemaDefine {
                schema_type: Some(JSONSchemaType::String),
                description: Some("The replacement string (may include capture groups like $1, $2)".to_string()),
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
            "is_regex".to_string(),
            Box::new(JSONSchemaDefine {
                schema_type: Some(JSONSchemaType::Boolean),
                description: Some("Whether this is a regex pattern (true) or literal string (false)".to_string()),
                enum_values: None,
                properties: None,
                required: None,
                items: None,
            }),
        );

        properties.insert(
            "regex_flags".to_string(),
            Box::new(JSONSchemaDefine {
                schema_type: Some(JSONSchemaType::String),
                description: Some("Optional regex flags (i, w, s)".to_string()),
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

    async fn analyze_track(&self, track: &Track) -> Result<ScrubActionSuggestion, Self::Error> {
        let existing_rules = self.format_existing_rules();

        let user_message = format!(
            "Analyze this Last.fm scrobble:\n\
            Track: \"{}\"\n\
            Artist: \"{}\"\n\
            Play count: {}\n\
            \n\
            {}",
            track.name, track.artist, track.playcount, existing_rules
        );

        let suggest_edit_function = Function {
            name: "suggest_track_edit".to_string(),
            description: Some(
                "Suggest metadata corrections for a Last.fm track scrobble".to_string(),
            ),
            parameters: FunctionParameters {
                schema_type: JSONSchemaType::Object,
                properties: Some(Self::create_edit_function_properties()),
                required: Some(vec!["reason".to_string()]),
            },
        };

        let suggest_rule_function = Function {
            name: "suggest_rewrite_rule".to_string(),
            description: Some(
                "Propose a new rewrite rule to automate similar metadata fixes".to_string(),
            ),
            parameters: FunctionParameters {
                schema_type: JSONSchemaType::Object,
                properties: Some(Self::create_rule_function_properties()),
                required: Some(vec!["pattern".to_string(), "replacement".to_string(), "target_field".to_string(), "is_regex".to_string(), "motivation".to_string()]),
            },
        };

        let no_action_function = Function {
            name: "no_action_needed".to_string(),
            description: Some("Indicate that no changes are needed for this track".to_string()),
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

        // Process the response
        if let Some(choice) = response.choices.first() {
            if let Some(tool_calls) = &choice.message.tool_calls {
                for tool_call in tool_calls {
                    if let Some(ref name) = tool_call.function.name {
                        match name.as_str() {
                            "suggest_track_edit" => {
                                if let Some(ref arguments) = tool_call.function.arguments {
                                    let args: TrackEditSuggestion = serde_json::from_str(arguments)
                                        .map_err(|e| {
                                            ActionProviderError(format!(
                                                "Failed to parse function arguments: {}",
                                                e
                                            ))
                                        })?;

                                    // Create a ScrobbleEdit with the suggested changes
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

                                    return Ok(ScrubActionSuggestion::Edit(edit));
                                }
                            }
                            "suggest_rewrite_rule" => {
                                if let Some(ref arguments) = tool_call.function.arguments {
                                    let args: RewriteRuleSuggestion = serde_json::from_str(arguments)
                                        .map_err(|e| {
                                            ActionProviderError(format!(
                                                "Failed to parse rewrite rule arguments: {}",
                                                e
                                            ))
                                        })?;

                                    // Create a RewriteRule from the suggestion
                                    let mut rule = crate::rewrite::RewriteRule::new();
                                    let sd_rule = if args.is_regex {
                                        crate::rewrite::SdRule::new_regex(&args.pattern, &args.replacement)
                                    } else {
                                        crate::rewrite::SdRule::new_literal(&args.pattern, &args.replacement)
                                    };

                                    match args.target_field.as_str() {
                                        "track_name" => rule = rule.with_track_name(sd_rule),
                                        "artist_name" => rule = rule.with_artist_name(sd_rule),
                                        "album_name" => rule = rule.with_album_name(sd_rule),
                                        "album_artist_name" => rule = rule.with_album_artist_name(sd_rule),
                                        _ => {
                                            return Err(ActionProviderError(format!(
                                                "Invalid target field: {}",
                                                args.target_field
                                            )));
                                        }
                                    }

                                    return Ok(ScrubActionSuggestion::ProposeRule {
                                        rule,
                                        motivation: args.motivation,
                                    });
                                }
                            }
                            "no_action_needed" => {
                                return Ok(ScrubActionSuggestion::NoAction);
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

        // If no function calls found, default to no action
        Ok(ScrubActionSuggestion::NoAction)
    }

    fn provider_name(&self) -> &str {
        "OpenAI"
    }
}
