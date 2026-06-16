//! Tests for [`super`]. Extracted from `types.rs` and included via
//! `#[path]` so they remain a child module with access to private items.

use super::*;

#[test]
fn parses_scalars() {
    assert_eq!(Type::parse("string"), Ok(Type::String));
    assert_eq!(Type::parse("number"), Ok(Type::Number));
    assert_eq!(Type::parse(" bool "), Ok(Type::Bool));
}

#[test]
fn parses_arrays_and_nested_arrays() {
    assert_eq!(
        Type::parse("[number]"),
        Ok(Type::Array(Box::new(Type::Number)))
    );
    assert_eq!(
        Type::parse("[ [string] ]"),
        Ok(Type::Array(Box::new(Type::Array(Box::new(Type::String)))))
    );
}

#[test]
fn parses_objects_including_nesting_and_empty() {
    assert_eq!(Type::parse("{}"), Ok(Type::Object(vec![])));
    assert_eq!(
        Type::parse(r#"{ "host": string, "reachable": bool, "ms": number }"#),
        Ok(Type::Object(vec![
            ("host".to_string(), Type::String),
            ("reachable".to_string(), Type::Bool),
            ("ms".to_string(), Type::Number),
        ]))
    );
    assert_eq!(
        Type::parse(r#"[{ "k": [number] }]"#),
        Ok(Type::Array(Box::new(Type::Object(vec![(
            "k".to_string(),
            Type::Array(Box::new(Type::Number)),
        )]))))
    );
}

#[test]
fn rejects_unknown_types_and_malformed_notation() {
    assert!(Type::parse("int").is_err());
    assert!(Type::parse("[number").is_err());
    assert!(Type::parse(r#"{ host: string }"#).is_err());
    assert!(Type::parse("string number").is_err());
    assert!(Type::parse("").is_err());
}

#[test]
fn lowers_to_json_schema() {
    assert_eq!(
        Type::String.to_schema(),
        serde_json::json!({"type": "string"})
    );
    assert_eq!(
        Type::Number.to_schema(),
        serde_json::json!({"type": "number"})
    );
    assert_eq!(
        Type::Bool.to_schema(),
        serde_json::json!({"type": "boolean"})
    );
    assert_eq!(
        Type::Array(Box::new(Type::String)).to_schema(),
        serde_json::json!({"type": "array", "items": {"type": "string"}})
    );
    assert_eq!(
        Type::Object(vec![
            ("host".to_string(), Type::String),
            ("ms".to_string(), Type::Number),
        ])
        .to_schema(),
        serde_json::json!({
            "type": "object",
            "properties": {
                "host": {"type": "string"},
                "ms": {"type": "number"},
            },
            "required": ["host", "ms"],
        })
    );
}

#[test]
fn display_round_trips_through_parse() {
    for notation in ["string", "number", "bool", "[string]", "[[number]]", "{}"] {
        let ty = Type::parse(notation).unwrap();
        assert_eq!(Type::parse(&ty.to_string()), Ok(ty));
    }
    let object = Type::parse(r#"{ "host": string, "ms": number }"#).unwrap();
    assert_eq!(object.to_string(), r#"{ "host": string, "ms": number }"#);
}

#[test]
fn serializes_as_its_notation_string() {
    let param = Param {
        name: "HOST".to_string(),
        ty: Type::String,
    };
    assert_eq!(
        serde_json::to_value(&param).unwrap(),
        serde_json::json!({ "name": "HOST", "type": "string" })
    );
}
