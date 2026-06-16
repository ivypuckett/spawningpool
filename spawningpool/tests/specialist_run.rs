//! End-to-end tests for the specialist run loop ([`run_specialist`]): a local
//! mock HTTP server stands in for an OpenAI-compatible endpoint, and real tool
//! scripts run between turns, so the full agentic path — request, tool call,
//! script execution, result feedback, next request — is exercised without a
//! real model or network. The unit tests in `run.rs` cover the pieces in
//! isolation; these cover them wired together.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use spawningpool::ai::{Api, Client, CompleteOptions, Reasoning};
use spawningpool::types::{Param, Type};
use spawningpool::{run_specialist, ModelDef, ProviderDef, Registry, Specialist, ToolDef};

/// Spawn an HTTP/1.1 server that answers each request with the next body in
/// `bodies`, falling back to `fallback` once the queue drains. Returns the base
/// URL and a counter of how many requests it served. Each response sets
/// `Connection: close`, so the client opens a fresh connection — a fresh accept
/// — per turn, letting one server serve a whole multi-turn run.
async fn mock_seq(bodies: Vec<String>, fallback: String) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let queue = Arc::new(Mutex::new(VecDeque::from(bodies)));
    let hits = Arc::new(AtomicUsize::new(0));
    let hits_task = hits.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            // Drain enough of the request to unblock the client, then reply.
            let mut buf = [0u8; 16384];
            let _ = socket.read(&mut buf).await;
            let body = queue
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| fallback.clone());
            hits_task.fetch_add(1, Ordering::SeqCst);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len(),
            );
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.flush().await;
        }
    });
    (format!("http://{addr}"), hits)
}

/// An OpenAI completion that calls `tool` with `{"NAME": "world"}`.
fn tool_call_body(tool: &str) -> String {
    format!(
        r#"{{
            "choices": [{{"message": {{"content": null, "tool_calls": [
                {{"id": "call_1", "type": "function", "function": {{"name": "{tool}", "arguments": "{{\"NAME\":\"world\"}}"}}}}
            ]}}, "finish_reason": "tool_calls"}}],
            "usage": {{"prompt_tokens": 5, "completion_tokens": 3}}
        }}"#
    )
}

/// An OpenAI completion that settles on a final text answer.
fn final_answer_body(text: &str) -> String {
    format!(
        r#"{{
            "choices": [{{"message": {{"content": "{text}"}}, "finish_reason": "stop"}}],
            "usage": {{"prompt_tokens": 8, "completion_tokens": 2}}
        }}"#
    )
}

/// A registry with one keyless OpenAI-compatible provider and model pointed at
/// `base_url`. The run loop resolves its model from here.
fn registry_at(base_url: String) -> Registry {
    let mut registry = Registry::default();
    registry.providers.insert(
        "lmstudio".into(),
        ProviderDef {
            name: "lmstudio".into(),
            api: Api::OpenAiCompletions,
            base_url,
            api_key_env: None,
            constrained_decoding: false,
        },
    );
    registry.models.insert(
        "local-model".into(),
        ModelDef {
            id: "local-model".into(),
            name: "local-model".into(),
            provider: "lmstudio".into(),
            max_tokens: 256,
            context_window: 8192,
        },
    );
    registry
}

fn specialist(name: &str, tools: Vec<String>, constraint: Option<String>) -> Specialist {
    Specialist {
        name: name.into(),
        provider: "lmstudio".into(),
        model: "local-model".into(),
        system_prompt: "do the thing".into(),
        tools,
        constraint,
        reasoning: Reasoning::Off,
        stream: false,
    }
}

/// A tool backed by a temp script that echoes `<word> $NAME`, plus the script
/// path so the caller can clean it up.
fn echo_tool(name: &str, word: &str) -> (ToolDef, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::temp_dir().join(format!(
        "sp_specrun_{}_{}.sh",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::write(&path, format!("#!/bin/sh\necho \"{word} $NAME\"\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    let def = ToolDef {
        name: name.into(),
        script: path.clone(),
        description: "echoes a greeting".into(),
        params: vec![Param {
            name: "NAME".into(),
            ty: Type::String,
        }],
        output: None,
    };
    (def, path)
}

#[tokio::test]
async fn agentic_loop_runs_a_tool_then_settles_on_an_answer() {
    let (greet, script) = echo_tool("greet", "greeted");
    let answer = final_answer_body("All greeted.");
    // Turn 1 calls the tool; turn 2 returns the final answer.
    let (base_url, hits) = mock_seq(vec![tool_call_body("greet"), answer.clone()], answer).await;
    let registry = registry_at(base_url);
    let spec = specialist("greeter", vec!["greet".into()], None);

    let mut events: Vec<String> = Vec::new();
    let result = run_specialist(
        &Client::new(),
        &registry,
        &spec,
        "say hi",
        &[greet],
        &CompleteOptions::default(),
        &mut |e| events.push(format!("{e:?}")),
    )
    .await;
    std::fs::remove_file(&script).ok();

    result.unwrap();
    // Exactly two model turns: the tool-call turn and the final-answer turn.
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    // The tool ran with the model's argument and its output was reported back.
    assert!(events
        .iter()
        .any(|e| e.contains("ToolRan") && e.contains("greeted world")));
    // The run ended on the model's final text answer.
    assert!(events.iter().any(|e| e.contains(r#"Text("All greeted.")"#)));
}

#[tokio::test]
async fn agentic_loop_stops_after_max_turns() {
    let (greet, script) = echo_tool("greet", "looping");
    // The fallback (empty queue) means every turn gets another tool call, so the
    // specialist never settles and must be cut off at the turn cap.
    let (base_url, hits) = mock_seq(vec![], tool_call_body("greet")).await;
    let registry = registry_at(base_url);
    let spec = specialist("looper", vec!["greet".into()], None);

    let err = run_specialist(
        &Client::new(),
        &registry,
        &spec,
        "go",
        &[greet],
        &CompleteOptions::default(),
        &mut |_| {},
    )
    .await
    .unwrap_err();
    std::fs::remove_file(&script).ok();

    assert!(err.contains("did not finish within 16 turns"), "{err}");
    // The cap is 16 turns, so it made exactly 16 requests before giving up.
    assert_eq!(hits.load(Ordering::SeqCst), 16);
}

#[tokio::test]
async fn constrained_specialist_makes_exactly_one_forced_call() {
    let (classify, script) = echo_tool("classify", "classified");
    // Only the forced call is offered; a second turn would be a bug, so the
    // fallback is harmless and `hits` proves the loop stopped after one.
    let call = tool_call_body("classify");
    let (base_url, hits) = mock_seq(vec![call.clone()], call).await;
    let registry = registry_at(base_url);
    let spec = specialist("classifier", vec![], Some("classify".into()));

    let mut events: Vec<String> = Vec::new();
    run_specialist(
        &Client::new(),
        &registry,
        &spec,
        "classify this",
        &[classify],
        &spec.complete_options(),
        &mut |e| events.push(format!("{e:?}")),
    )
    .await
    .unwrap();
    std::fs::remove_file(&script).ok();

    // A constraint forces a single call, then the run ends — one turn only.
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert!(events
        .iter()
        .any(|e| e.contains("ToolRan") && e.contains("classified")));
}
