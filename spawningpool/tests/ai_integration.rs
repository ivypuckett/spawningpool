//! End-to-end tests for the `ai` module: a local mock HTTP server stands in
//! for an OpenAI-compatible endpoint (e.g. LM Studio) so the full path —
//! runtime provider selection in `Client`, the adapter, and HTTP — is
//! exercised without a real model or network.

use futures::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use spawningpool::ai::{Api, Client, CompleteOptions, Context, Message, Model, StopReason};

/// Spawn a one-shot HTTP/1.1 server that returns `body` with the given
/// `content_type`, and return the `http://127.0.0.1:PORT` base URL.
async fn mock_server(content_type: &'static str, body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        // Drain the request headers (and body) enough to respond.
        let mut buf = [0u8; 4096];
        let _ = socket.read(&mut buf).await.unwrap();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len(),
        );
        socket.write_all(response.as_bytes()).await.unwrap();
        socket.flush().await.unwrap();
    });
    format!("http://{addr}")
}

fn local_model(base_url: String) -> Model {
    Model {
        id: "local-model".into(),
        name: "local-model".into(),
        api: Api::OpenAiCompletions,
        provider: "lmstudio".into(),
        base_url,
        max_tokens: 256,
        context_window: 8192,
        cost: spawningpool::ai::CostRates::FREE,
    }
}

#[tokio::test]
async fn complete_dispatches_by_api_and_parses_response() {
    let body = r#"{
        "choices": [{"message": {"content": "Hello there"}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 5, "completion_tokens": 2}
    }"#;
    let base_url = mock_server("application/json", body).await;
    let model = local_model(base_url);

    let client = Client::new();
    let ctx = Context::new(None, vec![Message::user("hi")]);
    let completion = client
        .complete(&model, &ctx, &CompleteOptions::default())
        .await
        .unwrap();

    assert_eq!(completion.stop_reason, StopReason::Stop);
    assert_eq!(
        completion.message.content,
        vec![spawningpool::ai::ContentBlock::text("Hello there")]
    );
    assert_eq!(completion.usage.input, 5);
    assert_eq!(completion.usage.output, 2);
}

#[tokio::test]
async fn stream_yields_text_deltas_then_done() {
    let body = "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\
                data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\
                data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2}}\n\
                data: [DONE]\n";
    let base_url = mock_server("text/event-stream", body).await;
    let model = local_model(base_url);

    let client = Client::new();
    let ctx = Context::new(None, vec![Message::user("hi")]);
    let mut stream = client
        .stream(&model, &ctx, &CompleteOptions::default())
        .await
        .unwrap();

    let mut deltas = String::new();
    let mut done = None;
    while let Some(event) = stream.next().await {
        match event.unwrap() {
            spawningpool::ai::StreamEvent::TextDelta { delta, .. } => deltas.push_str(&delta),
            spawningpool::ai::StreamEvent::Done {
                message,
                stop_reason,
                usage,
            } => {
                done = Some((message, stop_reason, usage));
            }
            _ => {}
        }
    }

    assert_eq!(deltas, "Hello");
    let (message, stop_reason, usage) = done.expect("stream emitted a Done event");
    assert_eq!(stop_reason, StopReason::Stop);
    assert_eq!(usage.output, 2);
    assert_eq!(
        message.content,
        vec![spawningpool::ai::ContentBlock::text("Hello")]
    );
}

#[tokio::test]
async fn unregistered_api_is_a_config_error() {
    // A client whose registry has no adapter for the model's api.
    let client = Client::with_registry(spawningpool::ai::ProviderRegistry::new());
    let model = local_model("http://127.0.0.1:1".into());
    let err = client
        .complete(&model, &Context::default(), &CompleteOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(err, spawningpool::ai::Error::Config(_)));
}
