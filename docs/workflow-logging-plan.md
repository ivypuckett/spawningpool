# Workflow logging — implementation plan

Implements the NDJSON event format agreed in
[`workflow-logging.md`](./workflow-logging.md) (currently "not yet
implemented"). This plan turns that format into code.

## Decisions (locked)

- **Sink**: always on. Each run writes one file `logs/<datestamp>-<root>.ndjson`
  in the current directory, creating `logs/` if absent. `root` is the root
  workflow name, or the specialist name for a bare `run specialist`.
  `<datestamp>` is a filesystem-safe compact UTC stamp, e.g. `20260619T142301Z`.
  Logs never go to stdout (stdout is the data channel — it carries the workflow
  result / specialist envelope) nor stderr (already carries `[usage]` lines and
  `ask` prompts).
- **`ts` / `run`**: hand-rolled with std only, no new dependency. `ts` is
  formatted from `SystemTime` (Unix epoch → civil date); `run` is 8 hex chars
  derived from `SystemTime` nanos mixed with the pid.

## Where events originate

| Event | Emitted from | Context it needs |
| --- | --- | --- |
| `workflow.start` / `done` / `error` | `eval_workflow` (per frame, incl. sub-workflows) | `wf` = `workflow.name`, `inputs`, elapsed |
| `specialist.start` / `done` | `run::run_specialist` | `wf`, `stmt`, specialist, model, prompt, aggregate turns/tokens/stop_reason/elapsed |
| `tool.call` / `done` (workflow `call tool`) | `eval_expr` `Expr::RunTool` (`specialist: null`) | `wf`, `stmt`, tool, typed args, success/exit_code/elapsed |
| `tool.call` / `done` (specialist's own tools) | `run::run_tool_call` (`specialist: "<name>"`) | same, plus specialist name |
| `ask.prompt` / `answer` | `eval_expr` `Expr::Ask` | `wf`, `stmt`, prompt, answered |

All events from one CLI invocation share the same `run` (owned by the sink), so
sub-workflows reached via `run workflow` land in the same file with a different
`wf` — exactly as the format requires.

## Universal-field split

The library builds each event object with `event`, `wf`, and the
event-specific fields. The sink injects the two fields it owns — `ts` (stamped
at emit time) and `run` (fixed for the invocation) — then writes the line. Key
order in the object is not significant; every line stays self-contained and
`jq`-parseable, so no `serde_json` `preserve_order` feature is needed.

## New library module: `spawningpool/src/log.rs`

Mirrors the existing `AskHandler` injection pattern (`eval.rs:47`) so the
library stays front-end agnostic.

```rust
/// A structured-log sink. The library hands it a JSON object carrying `event`,
/// `wf`, and event-specific fields; the sink adds `ts` and `run` and writes one
/// NDJSON line. A no-op closure disables logging.
pub type LogSink<'a> = dyn Fn(serde_json::Value) + 'a;
```

Event builders (one source of truth for the JSON shape, unit-tested here):

```rust
pub fn workflow_start(wf: &str, inputs: &serde_json::Value) -> Value
pub fn workflow_done(wf: &str, elapsed_ms: u128) -> Value
pub fn workflow_error(wf: &str, elapsed_ms: u128, error: &str) -> Value
pub fn specialist_start(wf: Option<&str>, stmt: Option<&str>, specialist: &str, model: &str, prompt: &str) -> Value
pub fn specialist_done(wf: Option<&str>, stmt: Option<&str>, specialist: &str, stop_reason: StopReason, turns: u32, input_tokens: u32, output_tokens: u32, elapsed_ms: u128) -> Value
pub fn tool_call(wf: Option<&str>, stmt: Option<&str>, specialist: Option<&str>, tool: &str, args: &serde_json::Value) -> Value
pub fn tool_done(wf: Option<&str>, stmt: Option<&str>, specialist: Option<&str>, tool: &str, success: bool, exit_code: Option<i32>, elapsed_ms: u128) -> Value
pub fn ask_prompt(wf: Option<&str>, stmt: Option<&str>, prompt: &str) -> Value
pub fn ask_answer(wf: Option<&str>, stmt: Option<&str>, answered: bool) -> Value
```

Re-export from `lib.rs`: `pub mod log;` plus `pub use log::LogSink;`. The
library never reads a clock or generates the run id — both belong to the sink.

`stop_reason` is serialized straight from `StopReason` (snake_case: `stop`,
`tool_use`, …). See [doc reconciliation](#doc-reconciliation).

## Evaluator changes: `workflow/eval.rs`

1. **`EvalCtx`** gains `log: &'a LogSink<'a>`. Created once in `eval`, shared by
   every frame, so all events carry the sink's single `run`.

2. **`eval` signature** gains a `log: &LogSink<'_>` parameter (threaded into
   `EvalCtx`). Update the `pub use` in `workflow/mod.rs:62` and the doc comment.

3. **Frame threading.** Introduce a small `Copy` struct so the per-frame
   workflow name and per-statement variable name reach the emitting expressions
   without a long argument list:

   ```rust
   #[derive(Clone, Copy)]
   struct Frame<'a> { wf: &'a str, stmt: Option<&'a str> }
   ```

   `eval_expr` / `eval_access` take `frame: Frame<'ctx>` (replacing nothing; it
   threads alongside `env`/`ctx`/`visited`). `eval_workflow` sets
   `frame.stmt = Some(&stmt.name)` per statement (every `Statement` has a
   `name`, confirmed at `ast.rs:135`).

4. **`eval_workflow`** wraps the statement loop:
   - On entry: `Instant::now()` + emit `workflow.start` with `wf = workflow.name`
     and `inputs` serialized as an object (the `inputs` map it already receives).
   - On success: emit `workflow.done` with `elapsed_ms`.
   - On error: before propagating, emit `workflow.error` with `elapsed_ms` and
     the error string. Each frame emits exactly one terminal event; a failing
     sub-workflow therefore produces its own `workflow.error` and the outer
     frame's `workflow.error` too.

5. **`Expr::RunTool`** (`eval.rs:245`): build a typed `args` object from the
   evaluated values **before** they are lowered into the `vars: HashMap<String,
   String>` (so the log shows `{"url": "..."}` typed, matching the format
   example, not stringified). Emit `tool.call` (`specialist: null`), time the
   `run_script` call with `Instant`, then emit `tool.done` with
   `success = run.success` and `exit_code = run.code`. Recovery (`else`) arms run
   after `tool.done` as normal.

6. **`Expr::Ask`** (`eval.rs:367`): emit `ask.prompt` before invoking the
   handler; emit `ask.answer` with `answered = matches!(outcome, Answered(_))`
   after.

7. **`Expr::RunSpecialist`** (`eval.rs:305`): build a `SpecialistLog` (below)
   from `ctx.log` and the current `Frame`, and pass it to `run_specialist` so the
   specialist's `start`/`done` and its internal `tool.*` events are emitted with
   the right `wf`/`stmt`. The `Collector` envelope is unchanged.

## Run-loop changes: `run.rs`

`run_specialist` is the only place that sees a specialist's internal tool calls
(with arguments) and can time them, so it owns `specialist.*` and the
specialist-scoped `tool.*` events.

1. New context struct (in `run.rs` or `log.rs`):

   ```rust
   pub struct SpecialistLog<'a> {
       pub sink: &'a LogSink<'a>,
       pub wf: Option<&'a str>,
       pub stmt: Option<&'a str>,
   }
   ```

2. `run_specialist` gains `log: Option<&SpecialistLog<'_>>`. `None` disables the
   specialist layer (used by tests / embedders that don't want it). The CLI and
   the evaluator always pass `Some`.
   - Before the loop: emit `specialist.start` (model from
     `registry.resolve_model`/`specialist.model`, the `prompt`). Start an
     `Instant` and zero accumulators.
   - Accumulate `turns`, `input_tokens`, `output_tokens`, and the last
     `stop_reason` from the values `one_turn` already returns (same data the
     `Collector` aggregates).
   - After the loop (on the `Ok` return paths): emit `specialist.done` with the
     aggregates and `elapsed_ms`.

3. `run_tool_call` (`run.rs:189`) gains the same `log` context. For each call:
   emit `tool.call` with `specialist = Some(name)` and `args = arguments` (the
   model's JSON object, already in hand), time the `run_script`, then emit
   `tool.done`:
   - `Ok(run)` → `success = run.success`, `exit_code = run.code`.
   - `Err(_)` (script failed to launch) → `success = false`, `exit_code = None`.

   This matches the format's `exit_code: null` = "failed to launch" rule. (Minor
   overload to note: a signal-killed script also yields `code = None`; both read
   as `null`. Acceptable and called out in the doc.)

4. Update the existing call sites for the new parameter:
   - `cli/src/commands/run.rs:79` and `:128` (bare specialist) → pass the CLI's
     `SpecialistLog`.
   - `eval.rs:352` → pass the evaluator's `SpecialistLog`.
   - `spawningpool/tests/specialist_run.rs`, `run_tests.rs` → pass `None`.

## CLI changes: build the sink, run id, and file

New module `cli/src/log.rs`, all std-only:

- `fn run_id() -> String` — 8 hex chars from `SystemTime::now()` nanos mixed
  with `process::id()`.
- `fn rfc3339_millis(t: SystemTime) -> String` — `YYYY-MM-DDTHH:MM:SS.mmmZ` via
  the standard civil-from-days conversion (days → y/m/d, plus ms-of-day).
  Unit-tested against known epochs.
- `fn datestamp(t: SystemTime) -> String` — compact `YYYYMMDDThhmmssZ` for the
  filename, from the same civil date.
- `fn open_sink(root: &str) -> Result<Box<dyn Fn(Value)>, String>` — create
  `logs/`, open `logs/<datestamp>-<root>.ndjson`, capture
  `RefCell<BufWriter<File>>` (eval is single-threaded — `LocalBoxFuture`, `!Send`
  — so `RefCell` suffices, no `Mutex`), and return a closure that: inserts `ts`
  (stamped now) and `run` (fixed) into the object, writes the serialized line
  + `\n`, and flushes per line for crash-observability.

Wire-up in `cli/src/commands/run.rs`:
- `run_workflow` — build the sink with `root = name`, pass `&sink` into
  `spawningpool::workflow::eval(...)` (the new param).
- `run_specialist` (both `json` and `plaintext` arms) — build the sink with
  `root = name`, build `SpecialistLog { sink: &sink, wf: None, stmt: None }`,
  pass `Some(&log)` to `run_specialist`. The existing render closures are
  unchanged; logging is an additional, independent observer.

## Doc reconciliation

`workflow-logging.md`'s example shows `"stop_reason":"end_turn"`, but the
normalized `StopReason` (`ai/message.rs:98`) serializes snake_case — a natural
finish is `"stop"`. Fix the example's value to the real serialization and add a
one-line note that `stop_reason` is the normalized `StopReason`. Also add a
short "Destination" note recording the agreed `logs/<datestamp>-<root>.ndjson`
sink.

## Tests

- `log_tests.rs` (new) — each builder produces the documented shape and fields.
- `eval_tests.rs` — capture into a `RefCell<Vec<Value>>` sink; assert the event
  sequence and `wf`/`stmt` for a workflow that calls a tool, runs a specialist
  with an internal tool, and performs an `ask` (mirrors the doc example, minus
  the sink-injected `ts`/`run`). Cover `workflow.error` on a failing statement
  and the sub-workflow nesting case (shared run, distinct `wf`).
- `run_tests.rs` — with a `Some` `SpecialistLog`, assert
  `specialist.start`/`tool.call`/`tool.done`/`specialist.done` and their
  context; with `None`, assert nothing is emitted.
- `cli/src/log.rs` tests — `rfc3339_millis` against known epochs, `run_id` is 8
  hex chars, the sink injects `ts`+`run` and emits valid NDJSON, and the file is
  named/created under `logs/`.

## File touch list

- **new** `spawningpool/src/log.rs` (+ `#[path]` `log_tests.rs`)
- `spawningpool/src/lib.rs` — `pub mod log;` + re-export
- `spawningpool/src/workflow/eval.rs` — `EvalCtx.log`, `eval` param, `Frame`,
  `workflow.*`, `tool.*` (RunTool), `ask.*`, `SpecialistLog` for RunSpecialist
- `spawningpool/src/workflow/mod.rs` — update `eval` re-export/signature
- `spawningpool/src/run.rs` — `SpecialistLog`, `run_specialist`/`run_tool_call`
  log param, `specialist.*` + specialist-scoped `tool.*`, aggregation + timing
- **new** `cli/src/log.rs` — run id, RFC3339, datestamp, sink/file
- `cli/src/main.rs` — `mod log;`
- `cli/src/commands/run.rs` — build + thread the sink in both run paths
- `spawningpool/tests/specialist_run.rs`, `spawningpool/src/run_tests.rs` —
  new `None` arg
- `docs/workflow-logging.md` — `stop_reason` example fix + destination note

## Success criteria

- `cargo test`, `cargo clippy`, `cargo fmt --check` all pass (the pre-commit
  hook gate).
- Running a workflow produces `logs/<datestamp>-<name>.ndjson` whose lines match
  the format: every line has `ts`/`run`/`event`/`wf`, sub-workflows share `run`,
  specialist invocations are bracketed atomically, and all tool calls (workflow
  and specialist-internal) appear with args, `exit_code`, and `elapsed_ms`.
- No NDJSON on stdout/stderr; the workflow result / specialist envelope on
  stdout is byte-for-byte unchanged.
