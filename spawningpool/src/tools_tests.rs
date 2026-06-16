//! Tests for [`super`]. Extracted from `tools.rs` and included via
//! `#[path]` so they remain a child module with access to private items.

use super::*;
use std::os::unix::fs::PermissionsExt;

/// A unique temp dir for one test's tool folder.
fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "sp_tools_{tag}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_tool(dir: &Path, file: &str, body: &str) {
    let path = dir.join(file);
    std::fs::write(&path, body).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn resolve_reads_header_and_strips_extension() {
    use crate::types::{Param, Type};

    let dir = temp_dir("resolve");
    write_tool(
        &dir,
        "ping.sh",
        "#!/bin/sh\n# desc: Ping a host\n# params: HOST\n# output: { \"ms\": number }\necho hi\n",
    );

    let tool = resolve(&dir, "ping").unwrap();
    assert_eq!(tool.name, "ping");
    assert_eq!(tool.description, "Ping a host");
    assert_eq!(
        tool.params,
        vec![Param {
            name: "HOST".to_string(),
            ty: Type::String,
        }]
    );
    assert_eq!(
        tool.output,
        Some(Type::Object(vec![("ms".to_string(), Type::Number)]))
    );
    assert!(tool.script.is_absolute());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn resolve_reports_unknown_and_ambiguous_tools() {
    let dir = temp_dir("ambig");
    // Nothing yet -> unknown.
    assert!(resolve(&dir, "ping").unwrap_err().contains("unknown tool"));

    // Two files share the stem -> ambiguous.
    write_tool(&dir, "ping", "#!/bin/sh\necho a\n");
    write_tool(&dir, "ping.sh", "#!/bin/sh\necho b\n");
    let err = resolve(&dir, "ping").unwrap_err();
    assert!(err.contains("ambiguous"), "got: {err}");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn list_returns_sorted_unique_names_and_skips_junk() {
    let dir = temp_dir("list");
    // Missing folder lists empty.
    let missing = dir.join("nope");
    assert!(list(&missing).unwrap().is_empty());

    write_tool(&dir, "beta.sh", "#!/bin/sh\n");
    write_tool(&dir, "alpha", "#!/bin/sh\n");
    write_tool(&dir, ".hidden", "#!/bin/sh\n");
    std::fs::create_dir_all(dir.join("subdir")).unwrap();

    assert_eq!(
        list(&dir).unwrap(),
        vec!["alpha".to_string(), "beta".to_string()]
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn exists_and_remove_clear_the_whole_name() {
    let dir = temp_dir("remove");
    write_tool(&dir, "ping", "#!/bin/sh\n");
    write_tool(&dir, "ping.sh", "#!/bin/sh\n");
    assert!(exists(&dir, "ping"));

    // Remove clears every file sharing the stem and reports it did so.
    assert!(remove(&dir, "ping").unwrap());
    assert!(!exists(&dir, "ping"));
    // Removing an absent tool reports nothing removed.
    assert!(!remove(&dir, "ping").unwrap());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn valid_tool_name_accepts_safe_names_only() {
    assert!(is_valid_tool_name("ping_2-x"));
    assert!(!is_valid_tool_name(""));
    assert!(!is_valid_tool_name("with.dot"));
    assert!(!is_valid_tool_name("space bar"));
}
