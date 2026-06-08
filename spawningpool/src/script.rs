//! Tool scripts: a tool is one executable script whose header declares its
//! description and parameters, and which is run with those parameters passed as
//! environment variables.
//!
//! Nothing here is provider-aware. `sp define tool` calls [`summarize`] to read
//! a script's `# desc:` and `# params:` header into a tool definition; the
//! runner calls [`run_script`] to execute it.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

/// A tool script's declared description and parameters, parsed from its header
/// comments.
#[derive(Debug, PartialEq, Eq)]
pub struct ScriptSummary {
    pub desc: Option<String>,
    pub params: Vec<String>,
}

/// The outcome of running a script: whether it exited successfully and its
/// combined stdout/stderr, ready to feed back to the model as a tool result.
#[derive(Debug, PartialEq, Eq)]
pub struct ScriptRun {
    pub success: bool,
    pub output: String,
}

/// Read a tool script's header for its `# desc:` line and `# params:` list.
/// Parameters are separated by whitespace and/or commas. The first occurrence
/// of each directive wins; non-comment lines are ignored.
pub fn summarize(path: &Path) -> Result<ScriptSummary, Box<dyn std::error::Error>> {
    let contents = std::fs::read_to_string(path)?;
    Ok(parse_header(&contents))
}

fn parse_header(contents: &str) -> ScriptSummary {
    let mut desc = None;
    let mut params: Option<Vec<String>> = None;

    for line in contents.lines() {
        let Some(comment) = line.trim_start().strip_prefix('#') else {
            continue;
        };
        let comment = comment.trim();
        if let Some(rest) = comment.strip_prefix("desc:") {
            desc.get_or_insert_with(|| rest.trim().to_string());
        } else if let Some(rest) = comment.strip_prefix("params:") {
            params.get_or_insert_with(|| {
                rest.split([',', ' ', '\t'])
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .map(String::from)
                    .collect()
            });
        }
    }

    ScriptSummary {
        desc,
        params: params.unwrap_or_default(),
    }
}

/// Run a tool script directly, passing each argument as an environment variable.
/// Captures combined stdout/stderr; a non-zero exit is reported via
/// [`ScriptRun::success`], not as an `Err` (only a failure to launch is an
/// `Err`). The script must be executable and carry a shebang.
pub fn run_script(
    path: &Path,
    args: &HashMap<String, String>,
) -> Result<ScriptRun, Box<dyn std::error::Error>> {
    let mut cmd = Command::new(path);
    for (key, value) in args {
        cmd.env(key, value);
    }
    let output = cmd.output()?;

    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.is_empty() {
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str(&stderr);
    }

    Ok(ScriptRun {
        success: output.status.success(),
        output: combined,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn parse_header_reads_desc_and_params() {
        let summary =
            parse_header("#!/bin/sh\n# desc: Deploy a service\n# params: ENV, REGION\necho hi\n");
        assert_eq!(summary.desc.as_deref(), Some("Deploy a service"));
        assert_eq!(
            summary.params,
            vec!["ENV".to_string(), "REGION".to_string()]
        );
    }

    #[test]
    fn parse_header_defaults_when_absent_and_first_directive_wins() {
        let none = parse_header("#!/bin/sh\necho hi\n");
        assert_eq!(none.desc, None);
        assert!(none.params.is_empty());

        let first = parse_header("# desc: one\n# desc: two\n# params: A\n# params: B\n");
        assert_eq!(first.desc.as_deref(), Some("one"));
        assert_eq!(first.params, vec!["A".to_string()]);
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
}
