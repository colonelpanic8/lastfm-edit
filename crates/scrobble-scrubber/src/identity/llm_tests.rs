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
    let content =
        r#"{"choice":"keep","confidence":"medium","reason":"Live album; different performance."}"#;
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
