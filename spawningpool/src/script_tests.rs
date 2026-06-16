//! Tests for [`super`]. Extracted from `script.rs` and included via
//! `#[path]` so they remain a child module with access to private items.

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

fn exit(code: i32, name: &str, desc: Option<&str>) -> ExitCode {
    ExitCode {
        code,
        name: name.to_string(),
        desc: desc.map(str::to_string),
    }
}

#[test]
fn parse_header_reads_exits() {
    let summary = parse_header(concat!(
        "#!/bin/sh\n",
        "# desc: Ping a host\n",
        "# exits: 0 ok \"host reachable\", 1 unreachable \"host did not respond\", 2 badArgs\n",
        "echo hi\n",
    ))
    .unwrap();
    assert_eq!(
        summary.exits,
        vec![
            exit(0, "ok", Some("host reachable")),
            exit(1, "unreachable", Some("host did not respond")),
            exit(2, "badArgs", None),
        ]
    );
}

#[test]
fn parse_header_keeps_commas_inside_an_exit_description() {
    let summary = parse_header("# exits: 1 failed \"down, unreachable, or slow\"\n").unwrap();
    assert_eq!(
        summary.exits,
        vec![exit(1, "failed", Some("down, unreachable, or slow"))]
    );
}

#[test]
fn parse_header_first_exits_directive_wins() {
    let summary = parse_header("# exits: 0 ok\n# exits: 1 nope\n").unwrap();
    assert_eq!(summary.exits, vec![exit(0, "ok", None)]);
}

#[test]
fn parse_header_rejects_malformed_exits() {
    // Non-integer code.
    assert!(parse_header("# exits: x ok\n").is_err());
    // Missing name.
    assert!(parse_header("# exits: 0\n").is_err());
    // Name that isn't a valid identifier (leading digit).
    assert!(parse_header("# exits: 0 1ok\n").is_err());
    // Duplicate name.
    assert!(parse_header("# exits: 0 ok, 1 ok\n").is_err());
    // Unquoted description.
    assert!(parse_header("# exits: 0 ok all good\n").is_err());
    // Unterminated description.
    assert!(parse_header("# exits: 0 ok \"all good\n").is_err());
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
    assert_eq!(run.code, Some(3));
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
