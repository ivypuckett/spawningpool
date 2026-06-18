//! The `converse` runner: a human-in-the-loop loop around a one-turn workflow.
//!
//! The DSL has no loop or "wait for input" construct, and deliberately so — a
//! workflow is a straight-line pass that runs to completion (workflow-dsl.md
//! §5). Turn-taking lives here instead: this runner owns the loop, the carried
//! conversation window, and the `continue` exit, and re-invokes a *one-turn*
//! workflow once per turn. The workflow stays a pure function of its inputs;
//! all continuity is the `window` string this runner threads back in.
//!
//! ## The contract
//!
//! The workflow must declare exactly these inputs, each a `string`:
//!
//! - `MODE`    — the mode the human picked this turn: `discuss` or `summarize`.
//! - `MESSAGE` — the human's message (used by `discuss`).
//! - `WINDOW`  — the conversation window carried in (`""` on the first turn).
//!
//! and return an object with two `string` fields:
//!
//! - `window` — the new window to carry into the next turn.
//! - `reply`  — the text to show the human.
//!
//! `continue` never reaches the workflow: it's the runner deciding to stop
//! looping, which is exactly "continue exits turn-taking". See
//! `docs/human-in-the-loop.md` and the example `converse` workflow.

use std::collections::HashMap;
use std::io::Write;

use spawningpool::ai::Client;

use super::run;

/// A persisted conversation run: its id, the workflow it drives, and the
/// conversation window carried between turns. This is the entire resumable
/// state — re-seeding `window` is all it takes to continue where a previous
/// session (or process) left off.
struct Run {
    id: String,
    workflow: String,
    window: String,
}

impl Run {
    /// Begin a fresh run with an empty window. The id is timestamp + pid, which
    /// is unique enough to key one user's runs without pulling in a uuid crate.
    fn start(workflow: &str) -> Self {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Run {
            id: format!("{secs}-{}", std::process::id()),
            workflow: workflow.to_string(),
            window: String::new(),
        }
    }

    /// Load a previously saved run by id from the runs folder.
    fn load(id: &str) -> Result<Self, String> {
        let path = spawningpool::store::runs_dir().join(format!("{id}.json"));
        let contents = std::fs::read_to_string(&path)
            .map_err(|e| format!("can't read run '{id}' at {}: {e}", path.display()))?;
        let value: serde_json::Value =
            serde_json::from_str(&contents).map_err(|e| format!("run '{id}' is corrupt: {e}"))?;
        let field = |k: &str| {
            value
                .get(k)
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .ok_or_else(|| format!("run '{id}' is missing string field `{k}`"))
        };
        Ok(Run {
            id: id.to_string(),
            workflow: field("workflow")?,
            window: field("window")?,
        })
    }

    /// Persist the run as `runs/<id>.json`, creating the folder if needed.
    fn save(&self) -> Result<(), String> {
        let dir = spawningpool::store::runs_dir();
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("can't create runs dir {}: {e}", dir.display()))?;
        let path = dir.join(format!("{}.json", self.id));
        let json = serde_json::json!({
            "id": self.id,
            "workflow": self.workflow,
            "window": self.window,
        });
        std::fs::write(&path, json.to_string())
            .map_err(|e| format!("can't write run {}: {e}", path.display()))
    }
}

/// Drive a human-in-the-loop conversation over the one-turn workflow `name`.
/// With `resume`, continue an existing run's window instead of starting fresh.
pub(crate) async fn converse(name: &str, resume: Option<String>) -> Result<(), String> {
    let registry = spawningpool::store::load()?;

    // Load the workflow and its `run` closure, resolve its tools, and type-check
    // it once up front — the same workflow is re-invoked every turn, so this
    // setup is paid once rather than per turn.
    let closure = run::load_workflow_closure(name, &registry)?;
    let workflows = &closure.workflows;
    let root = workflows
        .get(name)
        .expect("the closure always contains the root workflow");
    check_contract(root)?;

    let tools_dir = spawningpool::store::tools_dir();
    let tool_names: Vec<String> = closure.tools.iter().cloned().collect();
    let tools = spawningpool::tools::resolve_all(&tools_dir, &tool_names)?;
    spawningpool::workflow::check(root, &registry, &tools, workflows)
        .map_err(|e| format!("workflow '{name}' failed type-checking: {e}"))?;

    let keys = run::provider_keys(&registry);
    run::warn_unset_keys(&closure.specialists, &registry, &keys);
    let client = Client::new();

    // Start a new run or resume an existing window.
    let mut run = match resume {
        Some(id) => {
            let run = Run::load(&id)?;
            if run.workflow != name {
                return Err(format!(
                    "run '{id}' belongs to workflow '{}', not '{name}'",
                    run.workflow
                ));
            }
            println!("Resuming run '{id}'.");
            run
        }
        None => {
            let run = Run::start(name);
            println!("Started run '{}'. Pick a mode each turn:", run.id);
            run
        }
    };
    println!("  discuss (d) — take another turn   summarize (s) — condense the conversation   continue (c) — finish\n");

    loop {
        let mode = match read_line("mode [d/s/c] \u{25b8} ") {
            // EOF (e.g. piped input ends) is treated like `continue`.
            None => break,
            Some(line) => match line.trim().to_lowercase().as_str() {
                "discuss" | "d" => "discuss",
                "summarize" | "s" => "summarize",
                "continue" | "c" | "quit" | "q" => break,
                "" => continue,
                other => {
                    eprintln!("unknown mode '{other}'; pick discuss, summarize, or continue");
                    continue;
                }
            },
        };

        // Only `discuss` consumes a message; `summarize` works off the window.
        let message = if mode == "discuss" {
            match read_line("you \u{25b8} ") {
                None => break,
                Some(m) => m,
            }
        } else {
            String::new()
        };

        let inputs = HashMap::from([
            ("MODE".to_string(), serde_json::json!(mode)),
            ("MESSAGE".to_string(), serde_json::json!(message)),
            ("WINDOW".to_string(), serde_json::json!(run.window)),
        ]);

        let result = spawningpool::workflow::eval(
            root, &registry, &tools, &client, &keys, &inputs, workflows,
        )
        .await
        .map_err(|e| format!("turn failed: {e}"))?;

        let window = result_field(&result, "window")?;
        let reply = result_field(&result, "reply")?;
        println!("\nai \u{25b8} {reply}\n");

        run.window = window;
        run.save()?;
    }

    run.save()?;
    println!("\nConversation saved as run '{}'. Resume with:", run.id);
    println!("  spawningpool converse {name} --resume {}", run.id);
    Ok(())
}

/// Verify the workflow honors the runner's input contract: exactly `MODE`,
/// `MESSAGE`, and `WINDOW`, each a `string`. Catching a mismatch here gives a
/// pointed error instead of an `undefined variable` or unsupplied-input failure
/// mid-conversation.
fn check_contract(workflow: &spawningpool::workflow::Workflow) -> Result<(), String> {
    use spawningpool::types::Type;
    let required = ["MODE", "MESSAGE", "WINDOW"];
    for name in required {
        match workflow.inputs.iter().find(|p| p.name == name) {
            Some(p) if p.ty == Type::String => {}
            Some(p) => {
                return Err(format!(
                    "converse workflow input `{name}` must be `string`, found `{}`",
                    p.ty
                ))
            }
            None => {
                return Err(format!(
                    "converse workflow must declare a `string` input `{name}` \
                     (the contract is `# inputs: MODE:string, MESSAGE:string, WINDOW:string`)"
                ))
            }
        }
    }
    for param in &workflow.inputs {
        if !required.contains(&param.name.as_str()) {
            return Err(format!(
                "converse workflow declares unsupported input `{}`; \
                 the runner only supplies MODE, MESSAGE, and WINDOW",
                param.name
            ));
        }
    }
    Ok(())
}

/// Pull a required `string` field out of the workflow's result object, with an
/// error that names the contract field that's missing or mistyped.
fn result_field(result: &serde_json::Value, key: &str) -> Result<String, String> {
    result
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| {
            format!(
                "workflow result is missing string field `{key}` (expected {{ window, reply }})"
            )
        })
}

/// Print `prompt` and read one line from stdin, returning `None` at EOF. The
/// trailing newline is stripped.
fn read_line(prompt: &str) -> Option<String> {
    print!("{prompt}");
    std::io::stdout().flush().ok();
    let mut line = String::new();
    match std::io::stdin().read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(line.trim_end_matches(['\n', '\r']).to_string()),
    }
}
