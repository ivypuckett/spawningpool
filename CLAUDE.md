# spawningpool

Create hyper-specific, 0-waste agents.

## Project Structure

```
spawningpool/          # library crate — core logic and public API
cli/                   # binary crate — consumes the library, named `spawningpool`
```

## Build & Test

```sh
cargo build            # build all workspace members
cargo test             # test all workspace members
cargo clippy           # lint
cargo fmt              # format
```

The binary is built to `target/debug/spawningpool` (or `target/release/spawningpool` with `--release`).

## Git Hooks

Pre-commit hooks live in `.githooks/`. Install once per clone:

```sh
git config core.hooksPath .githooks
```

The pre-commit hook runs `cargo fmt --check`, `cargo clippy`, and `cargo test`.

---

## Rules for Agents

### 1. Think Before Coding
Don't assume. Don't hide confusion. Surface tradeoffs.
- If a request is ambiguous, state the ambiguity and ask before writing code.
- If there are multiple valid approaches, name them and recommend one with a reason.
- Never silently pick an interpretation and proceed.

### 2. Simplicity First
Minimum code that solves the problem. Nothing speculative.
- No abstractions, generics, or flexibility that wasn't asked for.
- No "while I'm here" refactors. No preparing for hypothetical future requirements.
- Three similar lines is better than a premature abstraction.

### 3. Surgical Changes
Touch only what you must. Clean up only your own mess.
- Every changed line must trace directly to the stated task.
- Don't reformat unrelated code, rename unrelated identifiers, or reorganize unrelated files.
- If you notice something broken outside your scope, report it — don't silently fix it.

### 4. Goal-Driven Execution
Define success criteria. Loop until verified.
- Before starting, convert the request into a concrete, checkable outcome.
- After making changes, verify: does `cargo test` pass? Does `cargo clippy` pass?
- Don't report done until you've confirmed the success criteria are met.
