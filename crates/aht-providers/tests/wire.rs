//! Wire-level tests against a local wiremock server with recorded-style SSE payloads.

use aht_core::model::{Provider, ProviderKind, ProviderPreset};
use aht_core::ProviderId;
use aht_providers::*;
use futures::StreamExt;
use serde_json::json;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{body_partial_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn sse(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body.to_string(), "text/event-stream")
}

fn req(model: &str) -> ChatRequest {
    ChatRequest {
        model: model.into(),
        system: Some("be brief".into()),
        messages: vec![Message::user_text("hello")],
        tools: vec![ToolDef {
            name: "run_command".into(),
            description: "run".into(),
            input_schema: json!({"type":"object"}),
        }],
        max_tokens: 256,
        temperature: None,
    }
}

async fn collect(p: &dyn ChatProvider, r: ChatRequest) -> Vec<ProviderEvent> {
    let s = p.stream(r, CancellationToken::new()).await.unwrap();
    s.map(|e| e.unwrap()).collect().await
}

async fn stream_err(p: &dyn ChatProvider, r: ChatRequest) -> Error {
    match p.stream(r, CancellationToken::new()).await {
        Err(e) => e,
        Ok(s) => {
            let items: Vec<_> = s.collect().await;
            items
                .into_iter()
                .find_map(|e| e.err())
                .expect("expected an error")
        }
    }
}

const BACKOFF: Duration = Duration::from_millis(5);

fn anthropic(server: &MockServer) -> AnthropicProvider {
    AnthropicProvider::new(format!("{}/v1", server.uri()), Some("sk-ant".into()))
        .with_retry_backoff(BACKOFF)
}

fn openai(server: &MockServer) -> OpenAiChatProvider {
    OpenAiChatProvider::new(format!("{}/v1", server.uri()), Some("sk-oa".into()))
        .with_retry_backoff(BACKOFF)
}

// ---------------------------------------------------------------- Anthropic

const ANTHROPIC_TOOL_STREAM: &str = r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","content":[],"model":"claude-sonnet-5-5","stop_reason":null,"usage":{"input_tokens":25,"output_tokens":1}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}

event: ping
data: {"type":"ping"}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Need to list files."}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"abc"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: content_block_start
data: {"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Let me "}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"check."}}

event: content_block_stop
data: {"type":"content_block_stop","index":1}

event: content_block_start
data: {"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"toolu_01","name":"run_command","input":{}}}

event: content_block_delta
data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"session_id\": \"abc"}}

event: content_block_delta
data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"\", \"command\": \"ls -"}}

event: content_block_delta
data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"la\"}"}}

event: content_block_stop
data: {"type":"content_block_stop","index":2}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":42}}

event: message_stop
data: {"type":"message_stop"}

"#;

#[tokio::test]
async fn anthropic_streams_text_thinking_and_split_tool_call() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "sk-ant"))
        .and(header("anthropic-version", "2023-06-01"))
        .and(body_partial_json(json!({
            "model": "claude-sonnet-5-5", "stream": true, "max_tokens": 256, "system": "be brief",
            "tools": [{"name": "run_command"}]
        })))
        .respond_with(sse(ANTHROPIC_TOOL_STREAM))
        .expect(1)
        .mount(&server)
        .await;
    let evs = collect(&anthropic(&server), req("claude-sonnet-5-5")).await;
    assert_eq!(
        evs,
        vec![
            ProviderEvent::ThinkingDelta("Need to list files.".into()),
            ProviderEvent::TextDelta("Let me ".into()),
            ProviderEvent::TextDelta("check.".into()),
            ProviderEvent::ToolUse {
                id: "toolu_01".into(),
                name: "run_command".into(),
                input: json!({"session_id":"abc","command":"ls -la"}),
            },
            ProviderEvent::Usage {
                input: 25,
                output: 42
            },
            ProviderEvent::Stop {
                reason: StopReason::ToolUse
            },
        ]
    );
}

#[tokio::test]
async fn anthropic_tool_with_no_arguments_gets_empty_object() {
    let server = MockServer::start().await;
    let body = r#"event: message_start
data: {"type":"message_start","message":{"usage":{"input_tokens":3,"output_tokens":1}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t","name":"list_sessions","input":{}}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":5}}

event: message_stop
data: {"type":"message_stop"}

"#;
    Mock::given(method("POST"))
        .respond_with(sse(body))
        .mount(&server)
        .await;
    let evs = collect(&anthropic(&server), req("m")).await;
    assert!(evs.contains(&ProviderEvent::ToolUse {
        id: "t".into(),
        name: "list_sessions".into(),
        input: json!({})
    }));
}

#[tokio::test]
async fn anthropic_error_mapping() {
    for (status, check) in [
        (401u16, "auth"),
        (403, "auth"),
        (404, "notfound"),
        (400, "bad"),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(status).set_body_json(json!({
                "type":"error","error":{"type":"x","message":"boom message"}})))
            .expect(1) // not retried
            .mount(&server)
            .await;
        let e = stream_err(&anthropic(&server), req("m")).await;
        assert!(!e.retryable(), "{e:?}");
        match (check, &e) {
            ("auth", Error::Auth(m)) | ("notfound", Error::NotFound(m)) => {
                assert_eq!(m, "boom message")
            }
            ("bad", Error::BadRequest { status: 400, .. }) => {}
            _ => panic!("status {status} -> {e:?}"),
        }
    }
}

#[tokio::test]
async fn anthropic_retries_once_on_429_then_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "0"))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(sse(ANTHROPIC_TOOL_STREAM))
        .expect(1)
        .mount(&server)
        .await;
    let evs = collect(&anthropic(&server), req("m")).await;
    assert!(matches!(evs.last(), Some(ProviderEvent::Stop { .. })));
}

#[tokio::test]
async fn anthropic_persistent_429_surfaces_retry_after_when_too_long() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "120"))
        .expect(1) // 120s > cap: no automatic retry
        .mount(&server)
        .await;
    match stream_err(&anthropic(&server), req("m")).await {
        Error::RateLimited { retry_after, .. } => {
            assert_eq!(retry_after, Some(Duration::from_secs(120)))
        }
        e => panic!("{e:?}"),
    }
}

#[tokio::test]
async fn anthropic_500_retried_once_then_fails() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500).set_body_string("upstream down"))
        .expect(2)
        .mount(&server)
        .await;
    let e = stream_err(&anthropic(&server), req("m")).await;
    assert!(e.retryable());
    assert!(matches!(e, Error::Server { status: 500, .. }), "{e:?}");
}

#[tokio::test]
async fn anthropic_overloaded_error_event_and_truncation() {
    let server = MockServer::start().await;
    let body = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\nevent: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n";
    Mock::given(method("POST"))
        .respond_with(sse(body))
        .mount(&server)
        .await;
    let e = stream_err(&anthropic(&server), req("m")).await;
    assert!(e.retryable(), "{e:?}");

    let server = MockServer::start().await;
    let truncated = "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"par\"}}\n\n";
    Mock::given(method("POST"))
        .respond_with(sse(truncated))
        .mount(&server)
        .await;
    let e = stream_err(&anthropic(&server), req("m")).await;
    assert!(matches!(e, Error::Protocol(_)), "{e:?}");
}

#[tokio::test]
async fn anthropic_list_models() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(query_param("limit", "1000"))
        .and(header("x-api-key", "sk-ant"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data":[{"type":"model","id":"claude-sonnet-5-5","display_name":"Claude Sonnet 5.5","created_at":"2026-01-01T00:00:00Z"}],
            "has_more": false
        })))
        .mount(&server)
        .await;
    let m = anthropic(&server).list_models().await.unwrap();
    assert_eq!(
        m,
        vec![ModelInfo {
            id: "claude-sonnet-5-5".into(),
            display_name: Some("Claude Sonnet 5.5".into())
        }]
    );
}

// ------------------------------------------------------------------- OpenAI

const OPENAI_TEXT_STREAM: &str = r#"data: {"id":"c1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}],"usage":null}

data: {"id":"c1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"reasoning_content":"Thinking..."},"finish_reason":null}],"usage":null}

data: {"id":"c1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":"Hel"},"finish_reason":null}],"usage":null}

data: {"id":"c1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":"lo!"},"finish_reason":null}],"usage":null}

data: {"id":"c1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":null}

data: {"id":"c1","object":"chat.completion.chunk","choices":[],"usage":{"prompt_tokens":11,"completion_tokens":4,"total_tokens":15}}

data: [DONE]

"#;

const OPENAI_TOOL_STREAM: &str = r#"data: {"id":"c2","choices":[{"index":0,"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call_a","type":"function","function":{"name":"run_command","arguments":""}}]},"finish_reason":null}]}

data: {"id":"c2","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"session"}}]},"finish_reason":null}]}

data: {"id":"c2","choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"id":"call_b","type":"function","function":{"name":"list_sessions","arguments":""}}]},"finish_reason":null}]}

data: {"id":"c2","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"_id\":\"s1\",\"comm"}}]},"finish_reason":null}]}

data: {"id":"c2","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"and\":\"uptime\"}"}}]},"finish_reason":null}]}

data: {"id":"c2","choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"function":{"arguments":"{}"}}]},"finish_reason":null}]}

data: {"id":"c2","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}

data: {"id":"c2","choices":[],"usage":{"prompt_tokens":100,"completion_tokens":30}}

data: [DONE]

"#;

#[tokio::test]
async fn openai_streams_text_reasoning_and_usage() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer sk-oa"))
        .and(body_partial_json(json!({
            "model": "gpt-x", "stream": true, "max_tokens": 256,
            "stream_options": {"include_usage": true},
            "messages": [{"role":"system","content":"be brief"},{"role":"user","content":"hello"}],
            "tools": [{"type":"function","function":{"name":"run_command"}}]
        })))
        .respond_with(sse(OPENAI_TEXT_STREAM))
        .expect(1)
        .mount(&server)
        .await;
    let evs = collect(&openai(&server), req("gpt-x")).await;
    assert_eq!(
        evs,
        vec![
            ProviderEvent::ThinkingDelta("Thinking...".into()),
            ProviderEvent::TextDelta("Hel".into()),
            ProviderEvent::TextDelta("lo!".into()),
            ProviderEvent::Usage {
                input: 11,
                output: 4
            },
            ProviderEvent::Stop {
                reason: StopReason::EndTurn
            },
        ]
    );
}

#[tokio::test]
async fn openai_accumulates_interleaved_tool_calls_by_index() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse(OPENAI_TOOL_STREAM))
        .mount(&server)
        .await;
    let evs = collect(&openai(&server), req("m")).await;
    assert_eq!(
        evs,
        vec![
            ProviderEvent::ToolUse {
                id: "call_a".into(),
                name: "run_command".into(),
                input: json!({"session_id":"s1","command":"uptime"}),
            },
            ProviderEvent::ToolUse {
                id: "call_b".into(),
                name: "list_sessions".into(),
                input: json!({}),
            },
            ProviderEvent::Usage {
                input: 100,
                output: 30
            },
            ProviderEvent::Stop {
                reason: StopReason::ToolUse
            },
        ]
    );
}

#[tokio::test]
async fn openai_tool_calls_with_stop_finish_reason_and_no_done_marker() {
    // Ollama-style: whole tool call in one chunk, no ids, finish_reason "stop", no [DONE].
    let server = MockServer::start().await;
    let body = "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"function\":{\"name\":\"list_sessions\",\"arguments\":\"{}\"}}]},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n";
    Mock::given(method("POST"))
        .respond_with(sse(body))
        .mount(&server)
        .await;
    let evs = collect(&openai(&server), req("m")).await;
    assert_eq!(
        evs,
        vec![
            ProviderEvent::ToolUse {
                id: "call_0".into(),
                name: "list_sessions".into(),
                input: json!({})
            },
            ProviderEvent::Stop {
                reason: StopReason::ToolUse
            },
        ]
    );
}

#[tokio::test]
async fn openai_length_and_truncated_stream() {
    let server = MockServer::start().await;
    let body = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"x\"},\"finish_reason\":\"length\"}]}\n\ndata: [DONE]\n\n";
    Mock::given(method("POST"))
        .respond_with(sse(body))
        .mount(&server)
        .await;
    let evs = collect(&openai(&server), req("m")).await;
    assert_eq!(
        evs.last(),
        Some(&ProviderEvent::Stop {
            reason: StopReason::MaxTokens
        })
    );

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"x\"}}]}\n\n",
        ))
        .mount(&server)
        .await;
    let e = stream_err(&openai(&server), req("m")).await;
    assert!(matches!(e, Error::Protocol(_)), "{e:?}");
}

#[tokio::test]
async fn openai_error_mapping_and_retry() {
    for (status, expect_calls) in [
        (401u16, 1u64),
        (403, 1),
        (404, 1),
        (429, 2),
        (500, 2),
        (503, 2),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(status)
                    .insert_header("retry-after", "0")
                    .set_body_json(json!({"error":{"message":"nope","type":"t"}})),
            )
            .expect(expect_calls)
            .mount(&server)
            .await;
        let e = stream_err(&openai(&server), req("m")).await;
        match (status, &e) {
            (401 | 403, Error::Auth(m)) | (404, Error::NotFound(m)) => assert_eq!(m, "nope"),
            (429, Error::RateLimited { message, .. }) => assert_eq!(message, "nope"),
            (500 | 503, Error::Server { status: s, .. }) => assert_eq!(*s, status),
            _ => panic!("{status} -> {e:?}"),
        }
        let core: aht_core::Error = e.into();
        if status == 401 {
            assert!(matches!(core, aht_core::Error::Auth(_)));
        }
    }
}

#[tokio::test]
async fn openai_500_then_ok_recovers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(502))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(sse(OPENAI_TEXT_STREAM))
        .mount(&server)
        .await;
    let evs = collect(&openai(&server), req("m")).await;
    assert!(matches!(evs.last(), Some(ProviderEvent::Stop { .. })));
}

#[tokio::test]
async fn openai_list_models_and_no_auth_header_without_key() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object":"list","data":[{"id":"gpt-x","object":"model"},{"id":"gpt-y"}]
        })))
        .mount(&server)
        .await;
    let p = OpenAiChatProvider::new(format!("{}/v1", server.uri()), None);
    let m = p.list_models().await.unwrap();
    assert_eq!(
        m.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["gpt-x", "gpt-y"]
    );
    let reqs = server.received_requests().await.unwrap();
    assert!(reqs[0].headers.get("authorization").is_none());
}

#[tokio::test]
async fn cancel_ends_stream_without_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse(OPENAI_TEXT_STREAM))
        .mount(&server)
        .await;
    let p = openai(&server);
    let cancel = CancellationToken::new();
    cancel.cancel();
    let s = p.stream(req("m"), cancel).await.unwrap();
    let items: Vec<_> = s.collect().await;
    assert!(items.iter().all(|i| i.is_ok()));
}

// ------------------------------------------------------------- from_config

fn provider(preset: ProviderPreset, kind: ProviderKind, base: &str) -> Provider {
    Provider {
        id: ProviderId::new(),
        name: "t".into(),
        kind,
        preset,
        base_url: base.into(),
        api_key: None,
        default_model: "m".into(),
    }
}

#[tokio::test]
async fn opencode_routes_claude_to_messages_and_others_to_chat_completions() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/zen/v1/messages"))
        .and(header("x-api-key", "k"))
        .and(header("authorization", "Bearer k"))
        .and(body_partial_json(json!({"model": "claude-sonnet-5-5"})))
        .respond_with(sse(ANTHROPIC_TOOL_STREAM))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/zen/v1/chat/completions"))
        .and(body_partial_json(json!({"model": "kimi-k3"})))
        .respond_with(sse(OPENAI_TEXT_STREAM))
        .expect(1)
        .mount(&server)
        .await;
    let p = from_config(
        &provider(
            ProviderPreset::OpenCodeZen,
            ProviderKind::OpenAiCompat,
            &format!("{}/zen/v1", server.uri()),
        ),
        Some("k".into()),
    );
    let a = collect(p.as_ref(), req("opencode/claude-sonnet-5-5")).await;
    assert!(a.contains(&ProviderEvent::TextDelta("check.".into())));
    let b = collect(p.as_ref(), req("kimi-k3")).await;
    assert!(b.contains(&ProviderEvent::TextDelta("Hel".into())));
}

#[tokio::test]
async fn from_config_deepseek_defaults_and_echo_reasoning() {
    // Base URL override points at the mock; DeepSeek echoes reasoning_content on tool turns.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(body_partial_json(json!({
            "messages": [
                {"role":"user"},
                {"role":"assistant","reasoning_content":"why","tool_calls":[{"id":"c"}]},
                {"role":"tool","tool_call_id":"c"}
            ]
        })))
        .respond_with(sse(OPENAI_TEXT_STREAM))
        .expect(1)
        .mount(&server)
        .await;
    let p = from_config(
        &provider(
            ProviderPreset::DeepSeek,
            ProviderKind::OpenAiCompat,
            &server.uri(),
        ),
        Some("k".into()),
    );
    let mut r = req("deepseek-flash");
    r.system = None;
    r.messages = vec![
        Message::user_text("go"),
        Message {
            role: Role::Assistant,
            content: vec![
                Part::Reasoning { text: "why".into() },
                Part::ToolUse {
                    id: "c".into(),
                    name: "list_sessions".into(),
                    input: json!({}),
                },
            ],
        },
        Message {
            role: Role::User,
            content: vec![Part::ToolResult {
                tool_use_id: "c".into(),
                content: "[]".into(),
                is_error: false,
            }],
        },
    ];
    collect(p.as_ref(), r).await;
}

#[test]
fn preset_default_base_urls() {
    assert_eq!(
        default_base_url(ProviderPreset::DeepSeek),
        Some("https://api.deepseek.com")
    );
    assert_eq!(
        default_base_url(ProviderPreset::Ollama),
        Some("http://localhost:11434/v1")
    );
    assert_eq!(
        default_base_url(ProviderPreset::LmStudio),
        Some("http://localhost:1234/v1")
    );
    assert_eq!(
        default_base_url(ProviderPreset::OpenCodeGo),
        Some("https://opencode.ai/zen/go/v1")
    );
    assert_eq!(default_base_url(ProviderPreset::Custom), None);
}
