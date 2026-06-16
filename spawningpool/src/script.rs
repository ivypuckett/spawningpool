//! Tool scripts: a tool is one executable script whose header declares its
//! description and parameters, and which is run with those parameters passed as
//! environment variables.
//!
//! Nothing here is provider-aware. `spawningpool define tool` calls [`summarize`] to read
//! a script's `# desc:` and `# params:` header into a tool definition; the
//! runner calls [`run_script`] to execute it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::types::{ExitCode, Param, Type};

/// A tool script's declared description, parameters, and output type, parsed
/// from its header comments.
#[derive(Debug, PartialEq, Eq)]
pub struct ScriptSummary {
    pub desc: Option<String>,
    pub params: Vec<Param>,
    /// The tool's declared `# output:` type (workflow-dsl §3), or `None` when
    /// the header doesn't declare one.
    pub output: Option<Type>,
    /// The tool's declared `# exits:` codes (see `docs/tools.md`), empty when the
    /// header declares none.
    pub exits: Vec<ExitCode>,
}

/// The outcome of running a script: whether it exited successfully, its combined
/// stdout/stderr (ready to feed back to the model as a tool result), and any
/// structured output the script wrote to `$SP_OUTPUT_PATH`.
#[derive(Debug, PartialEq, Eq)]
pub struct ScriptRun {
    pub success: bool,
    /// The script's exit status code, or `None` when it was terminated by a
    /// signal (so it has no code). Lets a front-end report *how* a tool failed.
    pub code: Option<i32>,
    /// Combined stdout/stderr — ordinary logs, as today.
    pub output: String,
    /// The raw contents the script wrote to `$SP_OUTPUT_PATH` (workflow-dsl §3),
    /// or `None` if it wrote nothing there. The bytes are returned verbatim; a
    /// caller that knows the tool's declared output [`Type`] parses them as JSON.
    pub structured_output: Option<String>,
}

/// Read a tool script's header for its `# desc:` line, `# params:` list, and
/// `# output:` type. Parameters are separated by whitespace and/or commas and
/// may carry an optional `:type` suffix (no suffix means `string`); the
/// `# output:` line declares the tool's output type (see [`crate::types`]). The
/// first occurrence of each directive wins; non-comment lines are ignored. A
/// malformed type in the header is an error.
pub fn summarize(path: &Path) -> Result<ScriptSummary, Box<dyn std::error::Error>> {
    let contents = std::fs::read_to_string(path)?;
    Ok(parse_header(&contents)?)
}

fn parse_header(contents: &str) -> Result<ScriptSummary, String> {
    let mut desc = None;
    let mut params: Option<Vec<Param>> = None;
    let mut output: Option<Type> = None;
    let mut output_seen = false;
    let mut exits: Option<Vec<ExitCode>> = None;

    for line in contents.lines() {
        let Some(comment) = line.trim_start().strip_prefix('#') else {
            continue;
        };
        let comment = comment.trim();
        if let Some(rest) = comment.strip_prefix("desc:") {
            desc.get_or_insert_with(|| rest.trim().to_string());
        } else if let Some(rest) = comment.strip_prefix("params:") {
            if params.is_none() {
                params = Some(parse_params(rest)?);
            }
        } else if let Some(rest) = comment.strip_prefix("output:") {
            // First `# output:` wins even when it declares nothing usable, so a
            // later line can't override it (matching the other directives).
            if !output_seen {
                output_seen = true;
                let rest = rest.trim();
                if !rest.is_empty() {
                    output = Some(
                        Type::parse(rest).map_err(|e| format!("invalid `# output:` type: {e}"))?,
                    );
                }
            }
        } else if let Some(rest) = comment.strip_prefix("exits:") {
            if exits.is_none() {
                exits = Some(parse_exits(rest)?);
            }
        }
    }

    Ok(ScriptSummary {
        desc,
        params: params.unwrap_or_default(),
        output,
        exits: exits.unwrap_or_default(),
    })
}

/// Parse a `# exits:` value into [`ExitCode`]s. Entries are separated by
/// top-level commas (so a description may contain commas inside its quotes), and
/// each entry is `CODE NAME` or `CODE NAME "description"`:
///
/// - `CODE` is the integer exit status.
/// - `NAME` is a DSL identifier (see [`is_dsl_ident`]) so a later workflow stage
///   can branch on it; names must be unique within the tool.
/// - the optional `"description"` is the human-readable meaning.
fn parse_exits(rest: &str) -> Result<Vec<ExitCode>, String> {
    let mut exits: Vec<ExitCode> = Vec::new();
    for entry in split_exit_entries(rest) {
        let (code_tok, after_code) = split_first_word(&entry);
        let code: i32 = code_tok
            .parse()
            .map_err(|_| format!("invalid exit code `{code_tok}` in `# exits:`"))?;

        let (name, after_name) = split_first_word(after_code);
        if name.is_empty() {
            return Err(format!("exit code {code} in `# exits:` is missing a name"));
        }
        if !is_dsl_ident(&name) {
            return Err(format!(
                "exit code name `{name}` in `# exits:` isn't a valid identifier"
            ));
        }
        if exits.iter().any(|e| e.name == name) {
            return Err(format!("duplicate exit code name `{name}` in `# exits:`"));
        }

        let desc = parse_exit_desc(after_name, code)?;
        exits.push(ExitCode { code, name, desc });
    }
    Ok(exits)
}

/// Read an entry's trailing `"description"` (or `None` when absent). Anything
/// after the closing quote, or an unterminated quote, is an error so a malformed
/// entry is rejected rather than silently truncated.
fn parse_exit_desc(rest: &str, code: i32) -> Result<Option<String>, String> {
    let rest = rest.trim();
    if rest.is_empty() {
        return Ok(None);
    }
    let inner = rest.strip_prefix('"').ok_or_else(|| {
        format!("unexpected text after exit code {code} name in `# exits:`; quote the description")
    })?;
    let (desc, after) = inner
        .split_once('"')
        .ok_or_else(|| format!("unterminated description for exit code {code} in `# exits:`"))?;
    if !after.trim().is_empty() {
        return Err(format!(
            "unexpected text after exit code {code} description in `# exits:`"
        ));
    }
    Ok(Some(desc.to_string()))
}

/// Split a `# exits:` value into trimmed, non-empty entries on top-level commas,
/// leaving commas inside a `"..."` description untouched.
fn split_exit_entries(rest: &str) -> Vec<String> {
    let mut entries = Vec::new();
    let mut current = String::new();
    let mut in_string = false;
    for c in rest.chars() {
        match c {
            '"' => {
                in_string = !in_string;
                current.push(c);
            }
            ',' if !in_string => {
                if !current.trim().is_empty() {
                    entries.push(current.trim().to_string());
                }
                current.clear();
            }
            _ => current.push(c),
        }
    }
    if !current.trim().is_empty() {
        entries.push(current.trim().to_string());
    }
    entries
}

/// Split `s` into its first whitespace-delimited word and the (trimmed)
/// remainder. A word ends at whitespace or a `"`, so `2"oops"` separates the
/// code/name from a quote with no space.
fn split_first_word(s: &str) -> (String, &str) {
    let s = s.trim_start();
    let end = s
        .find(|c: char| c.is_whitespace() || c == '"')
        .unwrap_or(s.len());
    (s[..end].to_string(), s[end..].trim_start())
}

/// Whether `name` is a valid DSL identifier (workflow-dsl lexer rule): an ASCII
/// letter or `_` start, then alphanumerics, `_`, or a `-` that is itself
/// followed by a letter or `_`. Mirrored here so a `# exits:` name is guaranteed
/// usable as an identifier at the workflow stage.
fn is_dsl_ident(name: &str) -> bool {
    let chars: Vec<char> = name.chars().collect();
    match chars.first() {
        Some(c) if c.is_ascii_alphabetic() || *c == '_' => {}
        _ => return false,
    }
    for (i, c) in chars.iter().enumerate().skip(1) {
        if c.is_ascii_alphanumeric() || *c == '_' {
            continue;
        }
        if *c == '-'
            && chars
                .get(i + 1)
                .is_some_and(|n| n.is_ascii_alphabetic() || *n == '_')
        {
            continue;
        }
        return false;
    }
    true
}

/// Parse a `# params:` value into typed parameters. Each token is `NAME` (type
/// `string`) or `NAME:type`; the type notation may itself contain commas and
/// spaces (inside `[]`/`{}`/`""`), so tokens are split only on top-level commas
/// and whitespace. Shared with the workflow parser, whose `# inputs:` header
/// uses the same notation (workflow-dsl.md §5.1).
pub(crate) fn parse_params(rest: &str) -> Result<Vec<Param>, String> {
    let mut params = Vec::new();
    for token in split_params(rest) {
        let param = match token.split_once(':') {
            Some((name, ty)) => {
                let name = name.trim().to_string();
                let ty = Type::parse(ty.trim())
                    .map_err(|e| format!("invalid type for param `{name}`: {e}"))?;
                Param { name, ty }
            }
            None => Param {
                name: token,
                ty: Type::String,
            },
        };
        params.push(param);
    }
    Ok(params)
}

/// Split a `# params:` value into trimmed, non-empty tokens, treating commas and
/// whitespace as separators only at the top level — never inside a `[]`/`{}`
/// type or a `"..."` object key.
fn split_params(rest: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    let mut in_string = false;

    let flush = |current: &mut String, tokens: &mut Vec<String>| {
        let token = current.trim();
        if !token.is_empty() {
            tokens.push(token.to_string());
        }
        current.clear();
    };

    for c in rest.chars() {
        match c {
            '"' => {
                in_string = !in_string;
                current.push(c);
            }
            '[' | '{' if !in_string => {
                depth += 1;
                current.push(c);
            }
            ']' | '}' if !in_string => {
                depth -= 1;
                current.push(c);
            }
            ',' if !in_string && depth == 0 => flush(&mut current, &mut tokens),
            c if c.is_whitespace() && !in_string && depth == 0 => flush(&mut current, &mut tokens),
            _ => current.push(c),
        }
    }
    flush(&mut current, &mut tokens);
    tokens
}

/// Why a tool script can't be accepted as a tool's backing executable.
#[derive(Debug)]
pub enum ScriptError {
    /// The script path couldn't be canonicalized or read — most often it
    /// doesn't exist. Carries the path as given and the underlying I/O error.
    Unreadable {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The script exists but isn't executable (no `+x` bit). Carries the
    /// canonical path, so a caller can show the exact `chmod` target.
    NotExecutable { path: PathBuf },
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScriptError::Unreadable { path, source } => {
                write!(f, "tool script {} can't be read: {source}", path.display())
            }
            ScriptError::NotExecutable { path } => {
                write!(f, "tool script {} isn't executable", path.display())
            }
        }
    }
}

impl std::error::Error for ScriptError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ScriptError::Unreadable { source, .. } => Some(source),
            ScriptError::NotExecutable { .. } => None,
        }
    }
}

/// Resolve a tool script to an absolute path and confirm it can actually run: it
/// must exist and have the executable bit set. Storing the script absolute means
/// the tool resolves no matter where it is later invoked from. This is the check
/// `spawningpool define tool` (or a UI) runs before accepting a script, so an un-runnable
/// one fails at define time with a fixable error rather than mid-run.
pub fn prepare_script(script: &Path) -> Result<PathBuf, ScriptError> {
    use std::os::unix::fs::PermissionsExt;

    let path = std::fs::canonicalize(script).map_err(|source| ScriptError::Unreadable {
        path: script.to_path_buf(),
        source,
    })?;
    let mode = std::fs::metadata(&path)
        .map_err(|source| ScriptError::Unreadable {
            path: path.clone(),
            source,
        })?
        .permissions()
        .mode();
    if mode & 0o111 == 0 {
        return Err(ScriptError::NotExecutable { path });
    }
    Ok(path)
}

/// Run a tool script directly, passing each argument as an environment variable.
/// Captures combined stdout/stderr; a non-zero exit is reported via
/// [`ScriptRun::success`], not as an `Err` (only a failure to launch is an
/// `Err`). The script must be executable and carry a shebang.
///
/// Before running, `SP_OUTPUT_PATH` is set to a fresh temp file (workflow-dsl
/// §3); a tool writes its structured JSON result there. After the script exits
/// the file is read into [`ScriptRun::structured_output`] and removed. stdout
/// and stderr stay ordinary logs and are not parsed.
pub fn run_script(
    path: &Path,
    args: &HashMap<String, String>,
) -> Result<ScriptRun, Box<dyn std::error::Error>> {
    let output_path = fresh_output_path();

    let mut cmd = Command::new(path);
    for (key, value) in args {
        cmd.env(key, value);
    }
    cmd.env("SP_OUTPUT_PATH", &output_path);
    let output = run_with_etxtbsy_retry(&mut cmd)?;

    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.is_empty() {
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str(&stderr);
    }

    // A tool that wrote nothing (or doesn't speak the protocol) leaves no file,
    // which reads back as `None`. Either way the temp file is cleaned up.
    let structured_output = std::fs::read_to_string(&output_path)
        .ok()
        .filter(|s| !s.trim().is_empty());
    std::fs::remove_file(&output_path).ok();

    Ok(ScriptRun {
        success: output.status.success(),
        code: output.status.code(),
        output: combined,
        structured_output,
    })
}

/// A unique path for one run's `$SP_OUTPUT_PATH`. The process id plus a
/// monotonic counter keeps it collision-free across concurrent tool runs.
fn fresh_output_path() -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("sp_output_{}_{seq}.json", std::process::id()))
}

/// Run a command, retrying briefly on `ETXTBSY` ("text file busy").
///
/// Exec-ing a freshly written script can transiently fail this way: if another
/// thread `fork`s (e.g. spawns its own process) in the window between the file
/// being created and its write handle closing, the child inherits that write
/// fd, and the kernel refuses to exec a file open for writing until the child
/// goes away. It's a race, not a real error — common when a tool script was
/// just scaffolded — so a few short retries clear it.
fn run_with_etxtbsy_retry(cmd: &mut Command) -> std::io::Result<std::process::Output> {
    use std::io::ErrorKind;

    const MAX_RETRIES: u32 = 5;
    let mut attempt = 0;
    loop {
        match cmd.output() {
            Err(e) if e.kind() == ErrorKind::ExecutableFileBusy && attempt < MAX_RETRIES => {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(10 * u64::from(attempt)));
            }
            other => return other,
        }
    }
}

#[cfg(test)]
#[path = "script_tests.rs"]
mod tests;
