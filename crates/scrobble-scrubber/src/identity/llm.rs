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
pub(crate) mod scripted {
    use super::{ChoiceRequest, LlmBackend, RawChoice, RawResearch, ResearchRequest};
    use crate::identity::adjudicate::AdjudicateError;
    use std::sync::{Arc, Mutex};

    /// Replays canned answers in order; records every request.
    #[derive(Default)]
    pub(crate) struct ScriptedBackend {
        pub answers: Mutex<Vec<RawChoice>>,
        pub requests: Mutex<Vec<ChoiceRequest>>,
        pub research_answers: Mutex<Vec<RawResearch>>,
        pub research_requests: Mutex<Vec<ResearchRequest>>,
    }

    fn next<T>(script: &Mutex<Vec<T>>) -> Result<T, AdjudicateError> {
        let mut script = script.lock().unwrap();
        if script.is_empty() {
            return Err(AdjudicateError::Transport("script exhausted".into()));
        }
        Ok(script.remove(0))
    }

    #[async_trait::async_trait]
    impl LlmBackend for ScriptedBackend {
        async fn choose(&self, request: ChoiceRequest) -> Result<RawChoice, AdjudicateError> {
            self.requests.lock().unwrap().push(request);
            next(&self.answers)
        }

        async fn research(&self, request: ResearchRequest) -> Result<RawResearch, AdjudicateError> {
            self.research_requests.lock().unwrap().push(request);
            next(&self.research_answers)
        }

        fn label(&self) -> String {
            "scripted".into()
        }
    }

    /// Lets a test keep a handle on the recorded requests after boxing the backend.
    #[async_trait::async_trait]
    impl LlmBackend for Arc<ScriptedBackend> {
        async fn choose(&self, request: ChoiceRequest) -> Result<RawChoice, AdjudicateError> {
            self.as_ref().choose(request).await
        }

        async fn research(&self, request: ResearchRequest) -> Result<RawResearch, AdjudicateError> {
            self.as_ref().research(request).await
        }

        fn label(&self) -> String {
            self.as_ref().label()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Sent {
        url: String,
        auth: Option<String>,
        anthropic_headers: [Option<String>; 3],
        body: serde_json::Value,
        at: Instant,
    }

    #[derive(Debug)]
    struct FakeHttp {
        status: u16,
        body: String,
        sent: std::sync::Mutex<Vec<Sent>>,
    }

    #[async_trait::async_trait]
    impl http_client::HttpClient for FakeHttp {
        async fn send(
            &self,
            mut req: http_client::Request,
        ) -> Result<http_client::Response, http_client::Error> {
            let body = req.body_string().await?;
            self.sent.lock().unwrap().push(Sent {
                url: req.url().to_string(),
                auth: req.header("Authorization").map(|v| v.as_str().to_string()),
                anthropic_headers: ["x-api-key", "anthropic-version", "anthropic-beta"]
                    .map(|name| req.header(name).map(|v| v.as_str().to_string())),
                body: serde_json::from_str(&body).unwrap(),
                at: Instant::now(),
            });
            let mut response = http_client::Response::new(self.status);
            response.set_body(self.body.clone());
            Ok(response)
        }
    }

    fn backend(status: u16, body: &str, api_key: Option<&str>) -> (Arc<FakeHttp>, OpenAiCompat) {
        let http = Arc::new(FakeHttp {
            status,
            body: body.to_string(),
            sent: Default::default(),
        });
        let config = LlmConfig {
            provider: LlmProvider::OpenAiCompat,
            command: Vec::new(),
            base_url: "https://api.openai.com/v1/".into(),
            api_key: api_key.map(Into::into),
            model: "gpt-4o-mini".into(),
            daily_call_budget: 200,
            min_interval: Duration::from_secs(1),
            research_command: Vec::new(),
            research_daily_budget: 0,
        };
        (http.clone(), OpenAiCompat::new(http, config))
    }

    fn completion(content: &str) -> String {
        json!({
            "id": "chatcmpl-B9MHDbslfkBeAs8l4bebGdFOJ6PeG",
            "object": "chat.completion",
            "created": 1741570283,
            "model": "gpt-4o-mini-2024-07-18",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": content, "refusal": null},
                "logprobs": null,
                "finish_reason": "stop",
            }],
            "usage": {"prompt_tokens": 1117, "completion_tokens": 46, "total_tokens": 1163},
        })
        .to_string()
    }

    fn request() -> ChoiceRequest {
        ChoiceRequest {
            system: "system prompt".into(),
            user: "case text".into(),
            allowed: vec!["A".into(), "B".into(), "keep".into(), "unsure".into()],
        }
    }

    #[tokio::test]
    async fn sends_a_schema_constrained_request_and_parses_the_answer() {
        let content = r#"{"choice":"A","confidence":"high","reason":"Shares ISRC NLF057790024 with the Slowhand recording."}"#;
        let (http, backend) = backend(200, &completion(content), Some("sk-test"));

        let raw = backend.choose(request()).await.unwrap();

        assert_eq!(
            raw,
            RawChoice {
                choice: "A".into(),
                confidence: "high".into(),
                reason: "Shares ISRC NLF057790024 with the Slowhand recording.".into(),
                text: content.into(),
            }
        );
        let sent = http.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].url, "https://api.openai.com/v1/chat/completions");
        assert_eq!(sent[0].auth.as_deref(), Some("Bearer sk-test"));
        assert_eq!(
            sent[0].body,
            json!({
                "model": "gpt-4o-mini",
                "temperature": 0,
                "messages": [
                    {"role": "system", "content": "system prompt"},
                    {"role": "user", "content": "case text"},
                ],
                "response_format": {
                    "type": "json_schema",
                    "json_schema": {
                        "name": "choice",
                        "strict": true,
                        "schema": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["choice", "confidence", "reason"],
                            "properties": {
                                "choice": {"type": "string", "enum": ["A", "B", "keep", "unsure"]},
                                "confidence": {"type": "string", "enum": ["high", "medium", "low"]},
                                "reason": {"type": "string"},
                            },
                        },
                    },
                },
            })
        );
    }

    #[tokio::test]
    async fn maps_responses_without_a_valid_answer() {
        let prose = "I think it is option A, the studio version.";
        let cases: Vec<(u16, String, Result<RawChoice, &str>)> = vec![
            (
                200,
                completion(prose),
                Ok(RawChoice {
                    text: prose.into(),
                    ..RawChoice::default()
                }),
            ),
            (
                200,
                completion(r#"{"confidence":"low"}"#),
                Ok(RawChoice {
                    confidence: "low".into(),
                    text: r#"{"confidence":"low"}"#.into(),
                    ..RawChoice::default()
                }),
            ),
            (
                429,
                r#"{"error":{"code":"rate_limit_exceeded"}}"#.into(),
                Err("llm rate limited"),
            ),
            (
                500,
                "upstream exploded".into(),
                Err("llm transport: http 500: upstream exploded"),
            ),
            (
                200,
                "<html>gateway</html>".into(),
                Err("llm transport: unexpected response: expected value at line 1 column 1"),
            ),
        ];
        for (status, body, expected) in cases {
            let (http, backend) = backend(status, &body, None);
            let got = backend.choose(request()).await.map_err(|e| e.to_string());
            assert_eq!(
                got,
                expected.map_err(String::from),
                "status {status}, body {body}"
            );
            assert_eq!(http.sent.lock().unwrap()[0].auth, None);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn spaces_calls_by_min_interval() {
        let content = r#"{"choice":"keep","confidence":"medium","reason":"Live album."}"#;
        let (http, backend) = backend(200, &completion(content), None);

        backend.choose(request()).await.unwrap();
        backend.choose(request()).await.unwrap();

        let sent = http.sent.lock().unwrap();
        assert_eq!(sent[1].at - sent[0].at, Duration::from_secs(1));
    }

    fn anthropic(status: u16, body: &str) -> (Arc<FakeHttp>, AnthropicMessages) {
        let http = Arc::new(FakeHttp {
            status,
            body: body.to_string(),
            sent: Default::default(),
        });
        let config = LlmConfig {
            provider: LlmProvider::Anthropic,
            command: Vec::new(),
            base_url: "https://api.anthropic.com/v1".into(),
            api_key: Some("sk-ant-test".into()),
            model: "claude-opus-5-5".into(),
            daily_call_budget: 200,
            min_interval: Duration::from_secs(1),
            research_command: Vec::new(),
            research_daily_budget: 0,
        };
        (http.clone(), AnthropicMessages::new(http, config))
    }

    fn message(stop_reason: &str, text: &str) -> String {
        json!({
            "id": "msg_01",
            "type": "message",
            "role": "assistant",
            "model": "claude-opus-5-5",
            "content": [
                {"type": "thinking", "thinking": "", "signature": "sig"},
                {"type": "text", "text": text},
            ],
            "stop_reason": stop_reason,
            "usage": {"input_tokens": 1200, "output_tokens": 80},
        })
        .to_string()
    }

    #[tokio::test]
    async fn anthropic_sends_structured_output_request_and_parses_the_text_block() {
        let content = r#"{"choice":"keep","confidence":"medium","reason":"Live album; different performance."}"#;
        let (http, backend) = anthropic(200, &message("end_turn", content));

        let raw = backend.choose(request()).await.unwrap();

        assert_eq!(raw.choice, "keep");
        assert_eq!(raw.confidence, "medium");
        assert_eq!(backend.label(), "anthropic:claude-opus-5-5");
        let sent = http.sent.lock().unwrap();
        assert_eq!(sent[0].url, "https://api.anthropic.com/v1/messages");
        assert_eq!(
            sent[0].anthropic_headers,
            [
                Some("sk-ant-test".to_string()),
                Some("2023-06-01".to_string()),
                Some("server-side-fallback-2026-07-01".to_string()),
            ]
        );
        assert_eq!(
            sent[0].body,
            json!({
                "model": "claude-opus-5-5",
                "max_tokens": 4096,
                "system": "system prompt",
                "messages": [{"role": "user", "content": "case text"}],
                "output_config": {
                    "effort": "low",
                    "format": {
                        "type": "json_schema",
                        "schema": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["choice", "confidence", "reason"],
                            "properties": {
                                "choice": {"type": "string", "enum": ["A", "B", "keep", "unsure"]},
                                "confidence": {"type": "string", "enum": ["high", "medium", "low"]},
                                "reason": {"type": "string"},
                            },
                        },
                    },
                },
                "fallbacks": "default",
            })
        );
    }

    #[tokio::test]
    async fn anthropic_refusal_yields_no_choice() {
        let (_http, backend) = anthropic(200, &message("refusal", ""));

        let raw = backend.choose(request()).await.unwrap();

        assert_eq!(
            raw,
            RawChoice {
                text: "stop_reason refusal: ".into(),
                ..RawChoice::default()
            }
        );
    }

    fn command(argv: &[&str]) -> CommandBackend {
        CommandBackend::new(LlmConfig {
            command: argv.iter().map(|arg| arg.to_string()).collect(),
            min_interval: Duration::ZERO,
            ..LlmConfig::claude_cli()
        })
    }

    #[tokio::test]
    async fn command_reads_the_case_on_stdin_and_unwraps_claude_structured_output() {
        let script = r#"read -r line; printf '{"type":"result","is_error":false,"result":"","structured_output":{"choice":"A","confidence":"high","reason":"%s"}}' "$line""#;
        let backend = command(&["sh", "-c", script]);

        let raw = backend.choose(request()).await.unwrap();

        assert_eq!(raw.choice, "A");
        assert_eq!(raw.confidence, "high");
        assert_eq!(raw.reason, "case text");
        assert_eq!(backend.label(), "sh:opus");
    }

    #[tokio::test]
    async fn command_placeholders_become_whole_arguments() {
        let backend = command(&[
            "sh",
            "-c",
            r#"printf '%s|%s|%s' "$0" "$1" "$2""#,
            "{model}",
            "{system}",
            "{schema}",
        ]);

        let raw = backend.choose(request()).await.unwrap();

        let schema = answer_schema(&request().allowed).to_string();
        assert_eq!(raw.text, format!("opus|system prompt|{schema}"));
        assert_eq!(raw.choice, "");
    }

    #[tokio::test]
    async fn command_failures_are_transport_errors() {
        let cases: Vec<(&[&str], &str)> = vec![
            (
                &["sh", "-c", "echo 'not logged in' >&2; exit 1"],
                "not logged in",
            ),
            (
                &[
                    "sh",
                    "-c",
                    r#"printf '{"is_error":true,"result":"usage limit reached"}'"#,
                ],
                "usage limit reached",
            ),
            (&["/nonexistent/claude"], "starting /nonexistent/claude"),
        ];
        for (argv, expected) in cases {
            let error = command(argv).choose(request()).await.unwrap_err();
            assert!(
                matches!(&error, AdjudicateError::Transport(message) if message.contains(expected)),
                "{argv:?}: {error:?}"
            );
        }
    }

    fn research_request() -> ResearchRequest {
        ResearchRequest {
            system: "research prompt".into(),
            user: "case text".into(),
            schema: json!({"type": "object"}),
        }
    }

    fn researcher(argv: &[&str]) -> CommandBackend {
        CommandBackend::new(LlmConfig {
            research_command: argv.iter().map(|arg| arg.to_string()).collect(),
            min_interval: Duration::ZERO,
            ..LlmConfig::claude_cli()
        })
    }

    #[tokio::test]
    async fn command_research_unwraps_claude_structured_output_and_validates_ids() {
        let envelope = |recording: &str, release_group: &str| {
            format!(
                r#"printf '%s' '{{"type":"result","subtype":"success","is_error":false,"num_turns":4,"result":"","structured_output":{{"recording_mbid":"{recording}","release_group_mbid":"{release_group}","confidence":"medium","reason":"take 2 is the master take"}}}}'"#
            )
        };
        let cases = [
            (
                envelope(
                    "D31D4528-14CF-48D9-960D-6A07EAE19760",
                    "392e7fcf-5ae9-37f6-b8c9-57901cfbee1f",
                ),
                Some("d31d4528-14cf-48d9-960d-6a07eae19760"),
                Some("392e7fcf-5ae9-37f6-b8c9-57901cfbee1f"),
            ),
            (
                envelope("d31d4528-14cf-48d9-960d-6a07eae1976", "392e7fcf"),
                None,
                None,
            ),
            (
                envelope("", "https://musicbrainz.org/release-group/392e7fcf"),
                None,
                None,
            ),
        ];
        for (script, recording, release_group) in cases {
            let raw = researcher(&["sh", "-c", &script])
                .research(research_request())
                .await
                .unwrap();
            assert_eq!(
                (
                    raw.recording.map(|id| id.0),
                    raw.release_group.map(|id| id.0),
                    raw.confidence.as_str(),
                    raw.reason.as_str(),
                ),
                (
                    recording.map(String::from),
                    release_group.map(String::from),
                    "medium",
                    "take 2 is the master take",
                ),
                "{script}"
            );
        }
    }

    #[tokio::test]
    async fn command_research_substitutes_its_own_schema_and_prompt() {
        let backend = researcher(&[
            "sh",
            "-c",
            r#"printf '%s|%s' "$0" "$1""#,
            "{system}",
            "{schema}",
        ]);

        let raw = backend.research(research_request()).await.unwrap();

        assert_eq!(raw.text, r#"research prompt|{"type":"object"}"#);
        assert_eq!(raw.recording, None);
    }

    #[tokio::test]
    async fn command_research_times_out() {
        let backend = CommandBackend {
            research_timeout: Duration::from_millis(100),
            ..researcher(&["sh", "-c", "sleep 30"])
        };

        let started = Instant::now();
        let error = backend.research(research_request()).await.unwrap_err();

        assert_eq!(error.to_string(), "llm transport: sh timed out");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn backends_without_a_research_command_refuse_research() {
        let (_http, anthropic) = anthropic(200, "{}");
        let no_command = researcher(&[]);
        assert_eq!(
            anthropic
                .research(research_request())
                .await
                .unwrap_err()
                .to_string(),
            "anthropic:claude-opus-5-5 cannot do web research"
        );
        assert_eq!(
            no_command
                .research(research_request())
                .await
                .unwrap_err()
                .to_string(),
            "claude:opus cannot do web research"
        );
    }
}
