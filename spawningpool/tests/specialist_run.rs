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
use spawningpool::{
    run_specialist, ModelDef, ProviderDef, Registry, Session, Specialist, SpecialistLog, ToolDef,
};

/// Spawn an HTTP/1.1 server that answers each request with the next body in
/// `bodies`, falling back to `fallback` once the queue drains. Returns the base
/// URL, a counter of how many requests it served, and the raw text of each
/// request it received (so a test can assert what was sent). Each response sets
/// `Connection: close`, so the client opens a fresh connection — a fresh accept
/// — per turn, letting one server serve a whole multi-turn run.
async fn mock_seq(
    bodies: Vec<String>,
    fallback: String,
) -> (String, Arc<AtomicUsize>, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let queue = Arc::new(Mutex::new(VecDeque::from(bodies)));
    let hits = Arc::new(AtomicUsize::new(0));
    let hits_task = hits.clone();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let requests_task = requests.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            // Drain enough of the request to unblock the client, then reply.
            let mut buf = [0u8; 16384];
            let n = socket.read(&mut buf).await.unwrap_or(0);
            requests_task
                .lock()
                .unwrap()
                .push(String::from_utf8_lossy(&buf[..n]).into_owned());
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
    (format!("http://{addr}"), hits, requests)
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
        exits: vec![],
    };
    (def, path)
}

#[tokio::test]
async fn agentic_loop_runs_a_tool_then_settles_on_an_answer() {
    let (greet, script) = echo_tool("greet", "greeted");
    let answer = final_answer_body("All greeted.");
    // Turn 1 calls the tool; turn 2 returns the final answer.
    let (base_url, hits, _reqs) =
        mock_seq(vec![tool_call_body("greet"), answer.clone()], answer).await;
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
        None,
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
async fn logs_specialist_lifecycle_with_its_tool_calls() {
    let (greet, script) = echo_tool("greet", "greeted");
    let answer = final_answer_body("All greeted.");
    let (base_url, _hits, _reqs) =
        mock_seq(vec![tool_call_body("greet"), answer.clone()], answer).await;
    let registry = registry_at(base_url);
    let spec = specialist("greeter", vec!["greet".into()], None);

    // Capture structured log events as compact JSON lines (the test crate has no
    // serde_json dependency, so we inspect the rendered text).
    let log: std::cell::RefCell<Vec<String>> = std::cell::RefCell::new(Vec::new());
    let sink = |e| log.borrow_mut().push(format!("{e}"));
    let spec_log = SpecialistLog {
        sink: &sink,
        wf: Some("flow"),
        stmt: Some("res"),
    };

    run_specialist(
        &Client::new(),
        &registry,
        &spec,
        "say hi",
        &[greet],
        &CompleteOptions::default(),
        &mut |_| {},
        Some(&spec_log),
    )
    .await
    .unwrap();
    std::fs::remove_file(&script).ok();

    let events = log.into_inner();
    // start, the specialist's own tool call/done, then done — in that order.
    let kinds: Vec<&str> = events
        .iter()
        .filter_map(|e| {
            [
                "specialist.start",
                "tool.call",
                "tool.done",
                "specialist.done",
            ]
            .into_iter()
            .find(|k| e.contains(&format!("\"event\":\"{k}\"")))
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "specialist.start",
            "tool.call",
            "tool.done",
            "specialist.done"
        ]
    );

    // The events carry the invocation's workflow/statement context and identity.
    let start = &events[0];
    assert!(start.contains("\"wf\":\"flow\""));
    assert!(start.contains("\"stmt\":\"res\""));
    assert!(start.contains("\"specialist\":\"greeter\""));

    // The tool call is scoped to the specialist and names the tool it invoked.
    let call = events.iter().find(|e| e.contains("tool.call")).unwrap();
    assert!(call.contains("\"specialist\":\"greeter\""));
    assert!(call.contains("\"tool\":\"greet\""));

    let done = events.iter().find(|e| e.contains("tool.done")).unwrap();
    assert!(done.contains("\"success\":true"));
    assert!(done.contains("\"exit_code\":0"));

    // specialist.done aggregates the run: two turns, settled on a stop reason.
    let spec_done = events
        .iter()
        .find(|e| e.contains("specialist.done"))
        .unwrap();
    assert!(spec_done.contains("\"turns\":2"));
    assert!(spec_done.contains("\"stop_reason\":"));
}

#[tokio::test]
async fn agentic_loop_stops_after_max_turns() {
    let (greet, script) = echo_tool("greet", "looping");
    // The fallback (empty queue) means every turn gets another tool call, so the
    // specialist never settles and must be cut off at the turn cap.
    let (base_url, hits, _reqs) = mock_seq(vec![], tool_call_body("greet")).await;
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
        None,
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
    let (base_url, hits, _reqs) = mock_seq(vec![call.clone()], call).await;
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
        None,
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

/// Collect the assistant `Text` blocks an observer sees into `out`.
fn capture_text(out: &mut String) -> impl FnMut(spawningpool::RunEvent<'_>) + '_ {
    move |e| {
        if let spawningpool::RunEvent::Text(t) = e {
            out.push_str(t);
        }
    }
}

#[tokio::test]
async fn session_carries_context_across_human_turns() {
    // A plain chat specialist (no tools): each turn settles on a final answer.
    let answer_a = final_answer_body("first answer");
    let answer_b = final_answer_body("second answer");
    let (base_url, hits, reqs) =
        mock_seq(vec![answer_a.clone(), answer_b.clone()], answer_b.clone()).await;
    let registry = registry_at(base_url);
    let spec = specialist("chatter", vec![], None);
    let opts = CompleteOptions::default();
    let mut session = Session::new(&registry, &spec, &[], &opts).unwrap();

    let mut out = String::new();
    session
        .turn(
            &Client::new(),
            "first question",
            &mut capture_text(&mut out),
            None,
        )
        .await
        .unwrap();
    assert_eq!(out, "first answer");

    out.clear();
    session
        .turn(
            &Client::new(),
            "second question",
            &mut capture_text(&mut out),
            None,
        )
        .await
        .unwrap();
    assert_eq!(out, "second answer");

    // Two human turns, one model call each.
    assert_eq!(hits.load(Ordering::SeqCst), 2);

    // The second request resends the whole conversation — both questions and the
    // first answer — proving the context carried across turns.
    let reqs = reqs.lock().unwrap();
    assert_eq!(reqs.len(), 2);
    assert!(reqs[0].contains("first question"));
    assert!(!reqs[0].contains("second question"));
    assert!(reqs[1].contains("first question"));
    assert!(reqs[1].contains("first answer"));
    assert!(reqs[1].contains("second question"));
}

#[tokio::test]
async fn session_rejects_a_constrained_specialist() {
    let registry = registry_at("http://127.0.0.1:1".into());
    let spec = specialist("classifier", vec![], Some("classify".into()));
    let opts = spec.complete_options();

    // A constrained specialist forces one call and has nothing to converse about,
    // so the session refuses to start — with a message naming the forced tool and
    // pointing at the single-shot command instead.
    let err = match Session::new(&registry, &spec, &[], &opts) {
        Ok(_) => panic!("expected the constrained specialist to be rejected"),
        Err(e) => e,
    };
    assert!(err.contains("constrained"), "{err}");
    assert!(err.contains("classify"), "{err}");
    assert!(err.contains("run specialist classifier"), "{err}");
}

#[tokio::test]
async fn session_errors_when_context_window_is_exhausted() {
    // Shrink the window so one turn's usage (8 in + 2 out) plus the model's
    // max_tokens overflows it: the second turn's pre-check must trip.
    let answer = final_answer_body("ok");
    let (base_url, _hits, _reqs) = mock_seq(vec![answer.clone()], answer.clone()).await;
    let mut registry = registry_at(base_url);
    registry
        .models
        .get_mut("local-model")
        .unwrap()
        .context_window = 260;
    let spec = specialist("chatter", vec![], None);
    let opts = CompleteOptions::default();
    let mut session = Session::new(&registry, &spec, &[], &opts).unwrap();

    // First turn succeeds: no prior usage yet, so the pre-check is skipped.
    session
        .turn(&Client::new(), "hi", &mut |_| {}, None)
        .await
        .unwrap();
    // Second turn trips it: 10 used + 256 max_tokens > 260 window.
    let err = session
        .turn(&Client::new(), "again", &mut |_| {}, None)
        .await
        .unwrap_err();
    assert!(err.contains("context window"), "{err}");
    assert!(err.contains("Start a new session"), "{err}");
}
