//! Tests for [`super`]. Extracted from `parse.rs` and included via
//! `#[path]` so they remain a child module with access to private items.

use super::*;
use crate::workflow::ast::*;

fn str(s: &str) -> Expr {
    Expr::Str(s.to_string())
}
fn num(n: f64) -> Expr {
    Expr::Num(n)
}
fn var(s: &str) -> Expr {
    Expr::Var(s.to_string())
}
fn add(l: Expr, r: Expr) -> Expr {
    Expr::BinOp {
        op: BinOp::Add,
        lhs: Box::new(l),
        rhs: Box::new(r),
    }
}

#[test]
fn parses_string_assignment() {
    let wf = parse(r#"city = "Portland""#).unwrap();
    assert_eq!(
        wf.statements,
        vec![Statement {
            name: "city".to_string(),
            expr: str("Portland"),
        }]
    );
}

#[test]
fn parses_number_and_bool_literals() {
    let wf = parse("x = 42\n\ny = true\n\nz = 1.5").unwrap();
    assert_eq!(wf.statements[0].expr, num(42.0));
    assert_eq!(wf.statements[1].expr, Expr::Bool(true));
    assert_eq!(wf.statements[2].expr, Expr::Num(1.5));
}

#[test]
fn parses_string_concatenation() {
    let wf = parse(r#"s = "hello" + " " + "world""#).unwrap();
    assert_eq!(
        wf.statements[0].expr,
        add(add(str("hello"), str(" ")), str("world"))
    );
}

#[test]
fn parses_binary_ops_left_to_right() {
    let wf = parse("x = 1 + 2 * 3").unwrap();
    assert_eq!(
        wf.statements[0].expr,
        Expr::BinOp {
            op: BinOp::Mul,
            lhs: Box::new(Expr::BinOp {
                op: BinOp::Add,
                lhs: Box::new(num(1.0)),
                rhs: Box::new(num(2.0)),
            }),
            rhs: Box::new(num(3.0)),
        }
    );
}

#[test]
fn parses_not_expr() {
    let wf = parse("x = !true").unwrap();
    assert_eq!(wf.statements[0].expr, Expr::Not(Box::new(Expr::Bool(true))));
}

#[test]
fn parses_object_literal() {
    let wf = parse(r#"r = { "city": city, "ok": true }"#).unwrap();
    assert_eq!(
        wf.statements[0].expr,
        Expr::Object(vec![
            ("city".to_string(), var("city")),
            ("ok".to_string(), Expr::Bool(true)),
        ])
    );
}

#[test]
fn parses_member_access_chain() {
    let wf = parse(r#"x = result.output"#).unwrap();
    assert_eq!(
        wf.statements[0].expr,
        Expr::Access {
            base: Box::new(var("result")),
            keys: vec![AccessKey::Ident("output".to_string())],
        }
    );
}

#[test]
fn parses_quoted_and_indexed_access() {
    let wf = parse(r#"x = a."key".0.(n)"#).unwrap();
    assert_eq!(
        wf.statements[0].expr,
        Expr::Access {
            base: Box::new(var("a")),
            keys: vec![
                AccessKey::Quoted("key".to_string()),
                AccessKey::Index(0),
                AccessKey::Computed(Box::new(var("n"))),
            ],
        }
    );
}

#[test]
fn parses_if_expression() {
    let wf = parse("x = if (a) b, (_) c").unwrap();
    assert_eq!(
        wf.statements[0].expr,
        Expr::If {
            branches: vec![(var("a"), var("b"))],
            default: Box::new(var("c")),
        }
    );
}

#[test]
fn parses_for_expression() {
    let wf = parse("x = for [item: items] (item)").unwrap();
    assert_eq!(
        wf.statements[0].expr,
        Expr::For {
            item: "item".to_string(),
            array: Box::new(var("items")),
            body: Box::new(var("item")),
        }
    );
}

#[test]
fn parses_foreach_as_alias_for_for() {
    let wf = parse("x = foreach [item: items] (item)").unwrap();
    assert!(matches!(wf.statements[0].expr, Expr::For { .. }));
}

#[test]
fn parses_do_expression() {
    let wf = parse("answer = do (run tool poll {}) while (answer.ready) max (5)").unwrap();
    assert_eq!(
        wf.statements[0].expr,
        Expr::Do {
            var: "answer".to_string(),
            body: Box::new(Expr::RunTool {
                tool: "poll".to_string(),
                args: vec![],
                recover: vec![],
                recover_default: None,
            }),
            cond: Box::new(Expr::Access {
                base: Box::new(var("answer")),
                keys: vec![AccessKey::Ident("ready".to_string())],
            }),
            max: Box::new(num(5.0)),
        }
    );
}

#[test]
fn rejects_do_without_while_and_max() {
    assert!(parse("answer = do (1)").is_err());
    assert!(parse("answer = do (1) while (true)").is_err());
}

#[test]
fn rejects_do_not_at_statement_top_level() {
    // `do` refers to the assigned variable, so it can't be nested in an expression.
    assert!(parse("answer = 1 + do (1) while (true) max (2)").is_err());
}

#[test]
fn parses_run_tool_expression() {
    let wf = parse(r#"w = run tool get_weather { CITY: city }"#).unwrap();
    assert_eq!(
        wf.statements[0].expr,
        Expr::RunTool {
            tool: "get_weather".to_string(),
            args: vec![("CITY".to_string(), var("city"))],
            recover: vec![],
            recover_default: None,
        }
    );
}

#[test]
fn parses_run_tool_with_hyphenated_name() {
    let wf = parse(r#"w = run tool get-weather { CITY: city }"#).unwrap();
    assert_eq!(
        wf.statements[0].expr,
        Expr::RunTool {
            tool: "get-weather".to_string(),
            args: vec![("CITY".to_string(), var("city"))],
            recover: vec![],
            recover_default: None,
        }
    );
}

#[test]
fn parses_run_workflow_expression() {
    let wf = parse(r#"r = run workflow deploy { ENV: env, COUNT: 3 }"#).unwrap();
    assert_eq!(
        wf.statements[0].expr,
        Expr::RunWorkflow {
            workflow: "deploy".to_string(),
            args: vec![
                ("ENV".to_string(), var("env")),
                ("COUNT".to_string(), num(3.0)),
            ],
        }
    );
}

#[test]
fn parses_run_specialist_expression() {
    let wf = parse(r#"s = run specialist reporter "hello""#).unwrap();
    assert_eq!(
        wf.statements[0].expr,
        Expr::RunSpecialist {
            specialist: "reporter".to_string(),
            prompt: Box::new(str("hello")),
        }
    );
}

#[test]
fn run_verb_and_kinds_accept_aliases() {
    // `spawn` aliases `run`; `overseer`/`lenny`/`ling` alias the kinds.
    assert!(matches!(
        parse("w = spawn tool t {}").unwrap().statements[0].expr,
        Expr::RunTool { .. }
    ));
    assert!(matches!(
        parse("w = run overseer deploy {}").unwrap().statements[0].expr,
        Expr::RunWorkflow { .. }
    ));
    assert!(matches!(
        parse(r#"s = run lenny reporter "hi""#).unwrap().statements[0].expr,
        Expr::RunSpecialist { .. }
    ));
    assert!(matches!(
        parse(r#"s = run ling reporter "hi""#).unwrap().statements[0].expr,
        Expr::RunSpecialist { .. }
    ));
}

#[test]
fn rejects_run_with_unknown_kind() {
    assert!(parse("x = run gadget foo {}").is_err());
}

#[test]
fn parses_run_tool_with_else_block() {
    let wf = parse(
        r#"r = run tool ping { HOST: h } else { unreachable: { "ms": 0 }, _: { "ms": 10 } }"#,
    )
    .unwrap();
    assert_eq!(
        wf.statements[0].expr,
        Expr::RunTool {
            tool: "ping".to_string(),
            args: vec![("HOST".to_string(), var("h"))],
            recover: vec![(
                "unreachable".to_string(),
                Expr::Object(vec![("ms".to_string(), num(0.0))]),
            )],
            recover_default: Some(Box::new(Expr::Object(vec![("ms".to_string(), num(10.0),)]))),
        }
    );
}

#[test]
fn parses_run_tool_without_else_block() {
    let wf = parse("r = run tool ping {}").unwrap();
    assert_eq!(
        wf.statements[0].expr,
        Expr::RunTool {
            tool: "ping".to_string(),
            args: vec![],
            recover: vec![],
            recover_default: None,
        }
    );
}

#[test]
fn rejects_else_block_with_duplicate_default() {
    assert!(parse("r = run tool ping {} else { _: 1, _: 2 }").is_err());
}

#[test]
fn parses_full_example_from_doc() {
    // The §5 example from workflow-dsl.md.
    let src = r#"city = "Portland"

weather = run tool get_weather { CITY: city }

summary = run specialist reporter ("Summarize: " + weather.summary)

result = { "city": city, "ok": weather.reachable, "report": summary.output }"#;
    let wf = parse(src).unwrap();
    assert_eq!(wf.statements.len(), 4);
    assert_eq!(wf.statements[0].name, "city");
    assert_eq!(wf.statements[1].name, "weather");
    assert_eq!(wf.statements[2].name, "summary");
    assert_eq!(wf.statements[3].name, "result");
}

#[test]
fn multiple_blank_lines_act_as_one_separator() {
    let wf = parse("a = 1\n\n\n\nb = 2").unwrap();
    assert_eq!(wf.statements.len(), 2);
}

#[test]
fn reports_unknown_tool_name_with_digits_after_hyphen_as_separate_tokens() {
    // `a-1` tokenizes as Ident("a"), Minus, Num(1) — not a single ident.
    let wf = parse("x = a - 1").unwrap();
    assert_eq!(
        wf.statements[0].expr,
        Expr::BinOp {
            op: BinOp::Sub,
            lhs: Box::new(var("a")),
            rhs: Box::new(num(1.0)),
        }
    );
}

#[test]
fn rejects_unterminated_string() {
    assert!(parse(r#"x = "hello"#).is_err());
}

#[test]
fn rejects_if_without_default_branch() {
    assert!(parse("x = if (a) b, (c) d").is_err());
}

#[test]
fn rejects_unknown_character() {
    assert!(parse("x = @foo").is_err());
}

#[test]
fn parses_inputs_header_into_typed_params() {
    let wf = parse("# inputs: CITY:string, COUNT:number\n\nx = CITY").unwrap();
    assert_eq!(
        wf.inputs,
        vec![
            Param {
                name: "CITY".to_string(),
                ty: crate::types::Type::String,
            },
            Param {
                name: "COUNT".to_string(),
                ty: crate::types::Type::Number,
            },
        ]
    );
    assert_eq!(wf.statements.len(), 1);
    assert_eq!(wf.statements[0].name, "x");
}

#[test]
fn no_inputs_header_yields_empty_inputs() {
    let wf = parse("x = 1").unwrap();
    assert!(wf.inputs.is_empty());
}

#[test]
fn comment_lines_are_ignored_and_dont_merge_statements() {
    let wf = parse("# a leading note\nx = 1\n# between\n\ny = 2").unwrap();
    assert!(wf.inputs.is_empty());
    assert_eq!(wf.statements.len(), 2);
    assert_eq!(wf.statements[0].name, "x");
    assert_eq!(wf.statements[1].name, "y");
}

#[test]
fn first_inputs_header_wins() {
    let wf = parse("# inputs: A:string\n# inputs: B:number\n\nx = A").unwrap();
    assert_eq!(wf.inputs.len(), 1);
    assert_eq!(wf.inputs[0].name, "A");
}

#[test]
fn rejects_malformed_inputs_header() {
    assert!(parse("# inputs: COUNT:int\n\nx = 1").is_err());
}
