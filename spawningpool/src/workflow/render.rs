//! Render a workflow as a Mermaid `flowchart` showing its data flow.
//!
//! A workflow is a flat sequence of `name = expr` assignments over a shared
//! namespace of inputs and prior statement names (workflow-dsl.md §5). The
//! diagram makes the implicit data flow explicit: one node per input and per
//! statement, with an edge `A --> B` whenever statement `B` references variable
//! `A`. Statement nodes are shaped by what they do — `run tool`, `run
//! specialist`, `run workflow`, or a plain expression.

use std::collections::{BTreeSet, HashMap};

use crate::types::Type;
use crate::workflow::ast::{AccessKey, BinOp, Expr, Workflow};

/// Render `workflow` as Mermaid `flowchart` source.
pub fn mermaid(workflow: &Workflow) -> String {
    let mut nodes = String::new();
    let mut edges = String::new();
    // The node that currently defines each variable name. Reassignment and
    // `for` shadowing mean a name can map to different nodes over the script, so
    // this is updated as we walk statements in order.
    let mut defs: HashMap<String, String> = HashMap::new();
    let mut counter = 0usize;

    for input in &workflow.inputs {
        let id = format!("n{counter}");
        counter += 1;
        let label = format!("{}: {}", input.name, type_label(&input.ty));
        nodes.push_str(&format!("    {id}{}\n", shape(Kind::Input, &label)));
        defs.insert(input.name.clone(), id);
    }

    for stmt in &workflow.statements {
        let id = format!("n{counter}");
        counter += 1;

        // Edges from the current definition of each referenced variable. Read
        // `defs` before updating it, so a self-reference (`x = x + 1`) points at
        // the previous `x`, not this node.
        let mut vars = BTreeSet::new();
        free_vars(&stmt.expr, &mut vars);
        for v in &vars {
            if let Some(src) = defs.get(v) {
                edges.push_str(&format!("    {src} --> {id}\n"));
            }
        }

        let label = format!("{} = {}", stmt.name, describe(&stmt.expr));
        nodes.push_str(&format!("    {id}{}\n", shape(kind(&stmt.expr), &label)));
        defs.insert(stmt.name.clone(), id);
    }

    let mut out = String::from("flowchart TD\n");
    out.push_str(&nodes);
    out.push_str(&edges);
    out
}

/// The variable names an expression reads. `for [item: array] (body)` binds
/// `item` within `body` only (workflow-dsl.md §6.5), so it is removed from the
/// body's free variables before merging — otherwise the loop variable would be
/// mistaken for an outer reference and draw a phantom edge.
fn free_vars(expr: &Expr, out: &mut BTreeSet<String>) {
    match expr {
        Expr::Str(_) | Expr::Num(_) | Expr::Bool(_) => {}
        Expr::Var(name) => {
            out.insert(name.clone());
        }
        Expr::Object(fields) => {
            for (_, e) in fields {
                free_vars(e, out);
            }
        }
        Expr::Not(inner) => free_vars(inner, out),
        Expr::BinOp { lhs, rhs, .. } => {
            free_vars(lhs, out);
            free_vars(rhs, out);
        }
        Expr::Access { base, keys } => {
            free_vars(base, out);
            for key in keys {
                if let AccessKey::Computed(e) = key {
                    free_vars(e, out);
                }
            }
        }
        Expr::If { branches, default } => {
            for (cond, result) in branches {
                free_vars(cond, out);
                free_vars(result, out);
            }
            free_vars(default, out);
        }
        Expr::For { item, array, body } => {
            free_vars(array, out);
            let mut inner = BTreeSet::new();
            free_vars(body, &mut inner);
            inner.remove(item);
            out.extend(inner);
        }
        Expr::RunTool {
            args,
            recover,
            recover_default,
            ..
        } => {
            for (_, e) in args {
                free_vars(e, out);
            }
            for (_, e) in recover {
                free_vars(e, out);
            }
            if let Some(e) = recover_default {
                free_vars(e, out);
            }
        }
        Expr::RunWorkflow { args, .. } => {
            for (_, e) in args {
                free_vars(e, out);
            }
        }
        Expr::RunSpecialist { prompt, .. } => free_vars(prompt, out),
    }
}

/// The shape category of a statement node, chosen by its top-level expression.
enum Kind {
    Input,
    Tool,
    Specialist,
    Workflow,
    Pure,
}

fn kind(expr: &Expr) -> Kind {
    match expr {
        Expr::RunTool { .. } => Kind::Tool,
        Expr::RunSpecialist { .. } => Kind::Specialist,
        Expr::RunWorkflow { .. } => Kind::Workflow,
        _ => Kind::Pure,
    }
}

/// Wrap a label in the Mermaid node shape for `kind`. The label is quoted so
/// spaces and punctuation are safe; embedded quotes become the `&quot;` entity.
fn shape(kind: Kind, label: &str) -> String {
    let l = escape(label);
    match kind {
        Kind::Input => format!("[/\"{l}\"/]"),
        Kind::Tool => format!("[\"{l}\"]"),
        Kind::Specialist => format!("([\"{l}\"])"),
        Kind::Workflow => format!("[[\"{l}\"]]"),
        Kind::Pure => format!("(\"{l}\")"),
    }
}

fn escape(s: &str) -> String {
    s.replace('"', "&quot;").replace('\n', " ")
}

/// A concise, human-readable summary of an expression for a node label. The
/// `run` forms name their target; other forms collapse to their shape.
fn describe(expr: &Expr) -> String {
    match expr {
        Expr::Str(s) => format!("\"{s}\""),
        Expr::Num(n) => format!("{n}"),
        Expr::Bool(b) => format!("{b}"),
        Expr::Object(_) => "{ ... }".to_string(),
        Expr::Var(name) => name.clone(),
        Expr::Not(_) => "!...".to_string(),
        Expr::BinOp { op, .. } => format!("... {} ...", bin_symbol(op)),
        Expr::Access { .. } => "...".to_string(),
        Expr::If { .. } => "if ...".to_string(),
        Expr::For { item, .. } => format!("for {item} ..."),
        Expr::RunTool { tool, .. } => format!("run tool {tool}"),
        Expr::RunWorkflow { workflow, .. } => format!("run workflow {workflow}"),
        Expr::RunSpecialist { specialist, .. } => format!("run specialist {specialist}"),
    }
}

fn bin_symbol(op: &BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Rem => "%",
        BinOp::Pow => "^",
        BinOp::Or => "or",
        BinOp::And => "and",
    }
}

fn type_label(ty: &Type) -> String {
    match ty {
        Type::String => "string".to_string(),
        Type::Number => "number".to_string(),
        Type::Bool => "bool".to_string(),
        Type::Array(inner) => format!("[{}]", type_label(inner)),
        Type::Object(_) => "object".to_string(),
    }
}

#[cfg(test)]
#[path = "render_tests.rs"]
mod tests;
