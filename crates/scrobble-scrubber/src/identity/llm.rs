//! Provider-agnostic LLM backends: multiple choice over native Anthropic Messages,
//! OpenAI-compatible chat completions, or a harness CLI; web research only through a
//! harness CLI.

use super::adjudicate::AdjudicateError;
use super::mb::{RecordingId, ReleaseGroupId};
use super::{LlmConfig, LlmProvider};
use http_client::http_types::{Body, Method, Request, StatusCode, Url};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::Instant;

/// A constrained multiple-choice question. Backends enforce `allowed` through a JSON
/// schema where supported; the adjudicator re-validates regardless.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ChoiceRequest {
    pub system: String,
    pub user: String,
    pub allowed: Vec<String>,
}

/// Unvalidated model output.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RawChoice {
    pub choice: String,
    pub confidence: String,
    pub reason: String,
    /// Full response text, for audit.
    pub text: String,
}

/// A web-research question answered with MusicBrainz identifiers in `schema`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ResearchRequest {
    pub system: String,
    pub user: String,
    pub schema: serde_json::Value,
}

/// Model output with its identifiers parsed: an id that isn't a UUID is `None`.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RawResearch {
    pub recording: Option<RecordingId>,
    pub release_group: Option<ReleaseGroupId>,
    pub confidence: String,
    pub reason: String,
    /// Full response text, for audit.
    pub text: String,
}

#[async_trait::async_trait]
#[allow(
    clippy::double_must_use,
    reason = "async-trait adds must_use to generated futures"
)]
pub(crate) trait LlmBackend: Send + Sync {
    async fn choose(&self, request: ChoiceRequest) -> Result<RawChoice, AdjudicateError>;
    async fn research(&self, _request: ResearchRequest) -> Result<RawResearch, AdjudicateError> {
        Err(AdjudicateError::Unsupported(self.label()))
    }
    /// "<provider>:<model>", recorded with every verdict.
    fn label(&self) -> String;
}

pub(crate) fn backend(
    http: Arc<dyn http_client::HttpClient>,
    config: LlmConfig,
) -> Box<dyn LlmBackend> {
    match config.provider {
        LlmProvider::Command => Box::new(CommandBackend::new(config)),
        LlmProvider::Anthropic => Box::new(AnthropicMessages::new(http, config)),
        LlmProvider::OpenAiCompat => Box::new(OpenAiCompat::new(http, config)),
    }
}

fn answer_schema(allowed: &[String]) -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["choice", "confidence", "reason"],
        "properties": {
            "choice": {"type": "string", "enum": allowed},
            "confidence": {"type": "string", "enum": ["high", "medium", "low"]},
            "reason": {"type": "string"},
        },
    })
}

/// Serializes calls and keeps `min_interval` between their starts.
struct CallSpacing {
    min_interval: Duration,
    last_call: Mutex<Option<Instant>>,
}

impl CallSpacing {
    fn new(min_interval: Duration) -> Self {
        Self {
            min_interval,
            last_call: Mutex::new(None),
        }
    }

    /// POSTs JSON with `headers` once the interval allows; returns the 2xx body.
    async fn post_json(
        &self,
        http: &dyn http_client::HttpClient,
        url: &str,
        headers: &[(&str, String)],
        body: &serde_json::Value,
    ) -> Result<String, AdjudicateError> {
        let transport = |e: &dyn std::fmt::Display| AdjudicateError::Transport(e.to_string());
        let url = Url::parse(url).map_err(|e| transport(&e))?;
        let mut request = Request::new(Method::Post, url);
        for (name, value) in headers {
            request.insert_header(*name, value.as_str()).map_err(|_| {
                AdjudicateError::Transport(format!("{name} is not a valid header value"))
            })?;
        }
        request.set_body(Body::from_json(body).map_err(|e| transport(&e))?);

        let mut last_call = self.last_call.lock().await;
        if let Some(last) = *last_call {
            tokio::time::sleep_until(last + self.min_interval).await;
        }
        *last_call = Some(Instant::now());
        let mut response = http.send(request).await.map_err(|e| transport(&e))?;
        drop(last_call);

        let status = response.status();
        if status == StatusCode::TooManyRequests {
            return Err(AdjudicateError::RateLimited);
        }
        let text = response.body_string().await.map_err(|e| transport(&e))?;
        if !status.is_success() {
            let code = u16::from(status);
            let excerpt: String = text.chars().take(200).collect();
            return Err(AdjudicateError::Transport(format!(
                "http {code}: {excerpt}"
            )));
        }
        Ok(text)
    }
}

/// `POST {base_url}/chat/completions` with `response_format: json_schema` constraining
/// `choice` to `allowed`. Works against OpenAI, OpenRouter, Ollama, and other compatible
/// servers.
pub(crate) struct OpenAiCompat {
    http: Arc<dyn http_client::HttpClient>,
    spacing: CallSpacing,
    config: LlmConfig,
}

impl OpenAiCompat {
    pub fn new(http: Arc<dyn http_client::HttpClient>, config: LlmConfig) -> Self {
        Self {
            http,
            spacing: CallSpacing::new(config.min_interval),
            config,
        }
    }

    fn request_body(&self, request: &ChoiceRequest) -> serde_json::Value {
        json!({
            "model": self.config.model,
            "temperature": 0,
            "messages": [
                {"role": "system", "content": request.system},
                {"role": "user", "content": request.user},
            ],
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "name": "choice",
                    "strict": true,
                    "schema": answer_schema(&request.allowed),
                },
            },
        })
    }
}

#[derive(serde::Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(serde::Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(serde::Deserialize)]
struct ChatMessage {
    content: Option<String>,
}

#[derive(serde::Deserialize)]
struct Answer {
    #[serde(default)]
    choice: String,
    #[serde(default)]
    confidence: String,
    #[serde(default)]
    reason: String,
}

/// Model text that isn't the schema's JSON still comes back, with an empty choice, so
/// validation records it as invalid instead of failing the call.
fn parse_content(content: String) -> RawChoice {
    match serde_json::from_str::<Answer>(&content) {
        Ok(answer) => RawChoice {
            choice: answer.choice,
            confidence: answer.confidence,
            reason: answer.reason,
            text: content,
        },
        Err(_) => RawChoice {
            text: content,
            ..RawChoice::default()
        },
    }
}

#[async_trait::async_trait]
impl LlmBackend for OpenAiCompat {
    async fn choose(&self, request: ChoiceRequest) -> Result<RawChoice, AdjudicateError> {
        let base = self.config.base_url.trim_end_matches('/');
        let headers: Vec<(&str, String)> = self
            .config
            .api_key
            .iter()
            .map(|key| ("Authorization", format!("Bearer {key}")))
            .collect();
        let body = self
            .spacing
            .post_json(
                self.http.as_ref(),
                &format!("{base}/chat/completions"),
                &headers,
                &self.request_body(&request),
            )
            .await?;
        let parsed: ChatResponse = serde_json::from_str(&body)
            .map_err(|e| AdjudicateError::Transport(format!("unexpected response: {e}")))?;
        let content = parsed
            .choices
            .into_iter()
            .next()
            .and_then(|choice| choice.message.content)
            .ok_or_else(|| AdjudicateError::Transport("response has no message content".into()))?;
        Ok(parse_content(content))
    }

    fn label(&self) -> String {
        format!("openai-compat:{}", self.config.model)
    }
}

/// Runs a harness CLI per question: the case on stdin, the answer on stdout. Placeholders
/// in the argv are replaced whole-argument by the schema JSON, system prompt and model.
pub(crate) struct CommandBackend {
    config: LlmConfig,
    last_call: Mutex<Option<Instant>>,
    /// A harness that hangs (login prompt, network stall) must not wedge the lane.
    choose_timeout: Duration,
    /// Research browses the web for several turns.
    research_timeout: Duration,
}

impl CommandBackend {
    pub fn new(config: LlmConfig) -> Self {
        Self {
            config,
            last_call: Mutex::new(None),
            choose_timeout: Duration::from_secs(180),
            research_timeout: Duration::from_secs(600),
        }
    }

    fn argv(&self, template: &[String], schema: &str, system: &str) -> Vec<String> {
        template
            .iter()
            .map(|arg| match arg.as_str() {
                "{schema}" => schema.to_string(),
                "{system}" => system.to_string(),
                "{model}" => self.config.model.clone(),
                _ => arg.clone(),
            })
            .collect()
    }

    /// Spawns `argv` with `stdin`, once `min_interval` allows; returns stdout of a
    /// successful exit.
    async fn run(
        &self,
        argv: Vec<String>,
        stdin: &str,
        timeout: Duration,
    ) -> Result<String, AdjudicateError> {
        use tokio::io::AsyncWriteExt;
        let transport = |e: &dyn std::fmt::Display| AdjudicateError::Transport(e.to_string());
        let (program, args) = argv
            .split_first()
            .ok_or_else(|| AdjudicateError::Transport("empty LLM command".into()))?;

        let mut last_call = self.last_call.lock().await;
        if let Some(last) = *last_call {
            tokio::time::sleep_until(last + self.config.min_interval).await;
        }
        *last_call = Some(Instant::now());
        let mut child = tokio::process::Command::new(program)
            .args(args)
            // Away from any project directory, so no project instructions get picked up.
            .current_dir(std::env::temp_dir())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| AdjudicateError::Transport(format!("starting {program}: {e}")))?;
        let mut child_stdin = child.stdin.take().expect("stdin is piped");
        match child_stdin.write_all(stdin.as_bytes()).await {
            // A child that exits without reading its input says why on stderr, below.
            Err(e) if e.kind() != std::io::ErrorKind::BrokenPipe => return Err(transport(&e)),
            _ => {}
        }
        drop(child_stdin);
        let output = tokio::time::timeout(timeout, child.wait_with_output())
            .await
            .map_err(|_| AdjudicateError::Transport(format!("{program} timed out")))?
            .map_err(|e| transport(&e))?;
        drop(last_call);

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let excerpt: String = stderr.chars().take(200).collect();
            return Err(AdjudicateError::Transport(format!(
                "{program} exited with {}: {excerpt}",
                output.status
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

/// `claude -p --output-format json` wraps the answer; other commands may print it bare.
/// Returns the answer text and the text kept for audit.
fn unwrap_envelope(stdout: String) -> Result<(String, String), AdjudicateError> {
    #[derive(serde::Deserialize)]
    struct Envelope {
        #[serde(default)]
        is_error: bool,
        structured_output: Option<serde_json::Value>,
        result: Option<String>,
    }
    let Ok(envelope) = serde_json::from_str::<Envelope>(&stdout) else {
        return Ok((stdout.clone(), stdout));
    };
    if envelope.is_error {
        let detail: String = envelope
            .result
            .unwrap_or_default()
            .chars()
            .take(200)
            .collect();
        return Err(AdjudicateError::Transport(format!(
            "command reported an error: {detail}"
        )));
    }
    match (envelope.structured_output, envelope.result) {
        (Some(answer), _) => Ok((answer.to_string(), stdout)),
        (None, Some(result)) => Ok((result.clone(), result)),
        (None, None) => Ok((stdout.clone(), stdout)),
    }
}

fn parse_research(answer: &str, text: String) -> RawResearch {
    #[derive(serde::Deserialize)]
    struct Answer {
        #[serde(default)]
        recording_mbid: String,
        #[serde(default)]
        release_group_mbid: String,
        #[serde(default)]
        confidence: String,
        #[serde(default)]
        reason: String,
    }
    let mbid = |raw: &str| {
        uuid::Uuid::parse_str(raw.trim())
            .ok()
            .map(|id| id.hyphenated().to_string())
    };
    match serde_json::from_str::<Answer>(answer) {
        Ok(answer) => RawResearch {
            recording: mbid(&answer.recording_mbid).map(RecordingId),
            release_group: mbid(&answer.release_group_mbid).map(ReleaseGroupId),
            confidence: answer.confidence,
            reason: answer.reason,
            text,
        },
        Err(_) => RawResearch {
            text,
            ..RawResearch::default()
        },
    }
}

#[async_trait::async_trait]
impl LlmBackend for CommandBackend {
    async fn choose(&self, request: ChoiceRequest) -> Result<RawChoice, AdjudicateError> {
        let schema = answer_schema(&request.allowed).to_string();
        let argv = self.argv(&self.config.command, &schema, &request.system);
        let stdout = self.run(argv, &request.user, self.choose_timeout).await?;
        let (answer, text) = unwrap_envelope(stdout)?;
        Ok(RawChoice {
            text,
            ..parse_content(answer)
        })
    }

    async fn research(&self, request: ResearchRequest) -> Result<RawResearch, AdjudicateError> {
        if self.config.research_command.is_empty() {
            return Err(AdjudicateError::Unsupported(self.label()));
        }
        let schema = request.schema.to_string();
        let argv = self.argv(&self.config.research_command, &schema, &request.system);
        let stdout = self.run(argv, &request.user, self.research_timeout).await?;
        let (answer, text) = unwrap_envelope(stdout)?;
        Ok(parse_research(&answer, text))
    }

    fn label(&self) -> String {
        let program = self
            .config
            .command
            .first()
            .map_or("command", String::as_str);
        format!("{program}:{}", self.config.model)
    }
}

/// Models that accept server-side refusal fallbacks (`fallbacks: "default"`).
const FALLBACK_MODELS: &[&str] = &[
    "claude-fable-5-1",
    "claude-opus-5-5",
    "claude-opus-5",
    "claude-sonnet-5-5",
];

/// `POST {base_url}/messages` with `output_config.format` constraining the answer to the
/// choice schema. A refusal or truncated answer comes back as unparseable text, which
/// validation turns into `Unsure`.
pub(crate) struct AnthropicMessages {
    http: Arc<dyn http_client::HttpClient>,
    spacing: CallSpacing,
    config: LlmConfig,
}

impl AnthropicMessages {
    pub fn new(http: Arc<dyn http_client::HttpClient>, config: LlmConfig) -> Self {
        Self {
            http,
            spacing: CallSpacing::new(config.min_interval),
            config,
        }
    }

    fn uses_fallbacks(&self) -> bool {
        FALLBACK_MODELS.contains(&self.config.model.as_str())
    }

    fn request_body(&self, request: &ChoiceRequest) -> serde_json::Value {
        let mut body = json!({
            "model": self.config.model,
            "max_tokens": 4096,
            "system": request.system,
            "messages": [{"role": "user", "content": request.user}],
            "output_config": {
                "effort": "low",
                "format": {"type": "json_schema", "schema": answer_schema(&request.allowed)},
            },
        });
        if self.uses_fallbacks() {
            body["fallbacks"] = json!("default");
        }
        body
    }
}

#[derive(serde::Deserialize)]
struct MessagesResponse {
    #[serde(default)]
    content: Vec<ContentBlock>,
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(serde::Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<String>,
}

#[async_trait::async_trait]
impl LlmBackend for AnthropicMessages {
    async fn choose(&self, request: ChoiceRequest) -> Result<RawChoice, AdjudicateError> {
        let base = self.config.base_url.trim_end_matches('/');
        let mut headers = vec![("anthropic-version", "2023-06-01".to_string())];
        if let Some(key) = &self.config.api_key {
            headers.push(("x-api-key", key.clone()));
        }
        if self.uses_fallbacks() {
            headers.push((
                "anthropic-beta",
                "server-side-fallback-2026-07-01".to_string(),
            ));
        }
        let body = self
            .spacing
            .post_json(
                self.http.as_ref(),
                &format!("{base}/messages"),
                &headers,
                &self.request_body(&request),
            )
            .await?;
        let parsed: MessagesResponse = serde_json::from_str(&body)
            .map_err(|e| AdjudicateError::Transport(format!("unexpected response: {e}")))?;
        let text: String = parsed
            .content
            .iter()
            .filter(|block| block.kind == "text")
            .filter_map(|block| block.text.as_deref())
            .collect();
        if parsed.stop_reason.as_deref() != Some("end_turn") {
            let reason = parsed.stop_reason.as_deref().unwrap_or("none");
            return Ok(RawChoice {
                text: format!("stop_reason {reason}: {text}"),
                ..RawChoice::default()
            });
        }
        Ok(parse_content(text))
    }

    fn label(&self) -> String {
        format!("anthropic:{}", self.config.model)
    }
}

#[cfg(test)]
#[path = "llm_scripted_tests.rs"]
pub(crate) mod scripted;

#[cfg(test)]
#[path = "llm_tests.rs"]
mod tests;
