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

use crate::types::{Param, Type};

/// A tool script's declared description, parameters, and output type, parsed
/// from its header comments.
#[derive(Debug, PartialEq, Eq)]
pub struct ScriptSummary {
    pub desc: Option<String>,
    pub params: Vec<Param>,
    /// The tool's declared `# output:` type (workflow-dsl §3), or `None` when
    /// the header doesn't declare one.
    pub output: Option<Type>,
}

/// The outcome of running a script: whether it exited successfully, its combined
/// stdout/stderr (ready to feed back to the model as a tool result), and any
/// structured output the script wrote to `$SP_OUTPUT_PATH`.
#[derive(Debug, PartialEq, Eq)]
pub struct ScriptRun {
    pub success: bool,
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
        }
    }

    Ok(ScriptSummary {
        desc,
        params: params.unwrap_or_default(),
        output,
    })
}

/// Parse a `# params:` value into typed parameters. Each token is `NAME` (type
/// `string`) or `NAME:type`; the type notation may itself contain commas and
/// spaces (inside `[]`/`{}`/`""`), so tokens are split only on top-level commas
/// and whitespace.
fn parse_params(rest: &str) -> Result<Vec<Param>, String> {
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
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn param(name: &str, ty: Type) -> Param {
        Param {
            name: name.to_string(),
            ty,
        }
    }

    #[test]
    fn parse_header_reads_desc_and_params() {
        let summary =
            parse_header("#!/bin/sh\n# desc: Deploy a service\n# params: ENV, REGION\necho hi\n")
                .unwrap();
        assert_eq!(summary.desc.as_deref(), Some("Deploy a service"));
        // Bare params default to `string`.
        assert_eq!(
            summary.params,
            vec![param("ENV", Type::String), param("REGION", Type::String)]
        );
        assert_eq!(summary.output, None);
    }

    #[test]
    fn parse_header_reads_typed_params_and_output() {
        let summary = parse_header(concat!(
            "#!/bin/sh\n",
            "# desc: Look up a host's latency\n",
            "# params: HOST:string, COUNT:number\n",
            "# output: { \"host\": string, \"reachable\": bool, \"ms\": number }\n",
            "echo hi\n",
        ))
        .unwrap();
        assert_eq!(
            summary.params,
            vec![param("HOST", Type::String), param("COUNT", Type::Number)]
        );
        assert_eq!(
            summary.output,
            Some(Type::Object(vec![
                ("host".to_string(), Type::String),
                ("reachable".to_string(), Type::Bool),
                ("ms".to_string(), Type::Number),
            ]))
        );
    }

    #[test]
    fn parse_header_splits_params_with_compound_types() {
        // A comma/space inside a `{}`/`[]` type doesn't end the param.
        let summary =
            parse_header("# params: HOSTS:[string], INFO:{ \"a\": number, \"b\": bool }, NAME\n")
                .unwrap();
        assert_eq!(
            summary.params,
            vec![
                param("HOSTS", Type::Array(Box::new(Type::String))),
                param(
                    "INFO",
                    Type::Object(vec![
                        ("a".to_string(), Type::Number),
                        ("b".to_string(), Type::Bool),
                    ])
                ),
                param("NAME", Type::String),
            ]
        );
    }

    #[test]
    fn parse_header_reports_a_malformed_type() {
        assert!(parse_header("# params: COUNT:int\n").is_err());
        assert!(parse_header("# output: [number\n").is_err());
    }

    #[test]
    fn parse_header_defaults_when_absent_and_first_directive_wins() {
        let none = parse_header("#!/bin/sh\necho hi\n").unwrap();
        assert_eq!(none.desc, None);
        assert!(none.params.is_empty());
        assert_eq!(none.output, None);

        let first = parse_header("# desc: one\n# desc: two\n# params: A\n# params: B\n").unwrap();
        assert_eq!(first.desc.as_deref(), Some("one"));
        assert_eq!(first.params, vec![param("A", Type::String)]);
    }

    fn write_script(body: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "sp_script_{}_{}.sh",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn prepare_script_returns_absolute_path_for_executable() {
        let path = write_script("#!/bin/sh\necho hi\n");
        let resolved = prepare_script(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert!(resolved.is_absolute());
    }

    #[test]
    fn prepare_script_rejects_non_executable() {
        let path = std::env::temp_dir().join(format!(
            "sp_noexec_{}_{}.sh",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, "#!/bin/sh\necho hi\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = prepare_script(&path).unwrap_err();
        std::fs::remove_file(&path).ok();
        assert!(matches!(err, ScriptError::NotExecutable { .. }));
    }

    #[test]
    fn prepare_script_reports_a_missing_script_as_unreadable() {
        let err = prepare_script(Path::new("/nonexistent/sp_missing_xyz.sh")).unwrap_err();
        assert!(matches!(err, ScriptError::Unreadable { .. }));
    }

    #[test]
    fn run_script_passes_params_as_env_and_captures_output() {
        let path = write_script("#!/bin/sh\necho \"hi $NAME\"\n");
        let mut args = HashMap::new();
        args.insert("NAME".to_string(), "world".to_string());

        let run = run_script(&path, &args).unwrap();
        std::fs::remove_file(&path).ok();

        assert!(run.success);
        assert_eq!(run.output.trim(), "hi world");
    }

    #[test]
    fn run_script_reports_failure_and_includes_stderr() {
        let path = write_script("#!/bin/sh\necho oops >&2\nexit 3\n");
        let run = run_script(&path, &HashMap::new()).unwrap();
        std::fs::remove_file(&path).ok();

        assert!(!run.success);
        assert!(run.output.contains("oops"));
    }

    #[test]
    fn run_script_reads_structured_output_from_sp_output_path() {
        // The script logs to stdout but writes its structured result to the file.
        let path =
            write_script("#!/bin/sh\necho logging\nprintf '{\"ms\":12}' > \"$SP_OUTPUT_PATH\"\n");
        let run = run_script(&path, &HashMap::new()).unwrap();
        std::fs::remove_file(&path).ok();

        assert!(run.success);
        assert_eq!(run.output.trim(), "logging");
        assert_eq!(run.structured_output.as_deref(), Some("{\"ms\":12}"));
    }

    #[test]
    fn run_script_has_no_structured_output_when_file_is_untouched() {
        let path = write_script("#!/bin/sh\necho hi\n");
        let run = run_script(&path, &HashMap::new()).unwrap();
        std::fs::remove_file(&path).ok();

        assert_eq!(run.structured_output, None);
    }
}
