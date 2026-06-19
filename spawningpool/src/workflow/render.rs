//! Render a workflow as a Mermaid `flowchart` showing its data flow.
//!
//! A workflow is a flat sequence of `name = expr` assignments over a shared
//! namespace of inputs and prior statement names (workflow-dsl.md §5). The
//! diagram makes the implicit data flow explicit: one node per input and per
//! statement, with an edge `A --v--> B` whenever statement `B` references the
//! variable `v` defined by `A`.
//!
//! Nodes carry just a name: a `run tool`/`run specialist`/`run workflow`
//! statement shows the name of the thing it calls; every other statement (and
//! each input) shows its own variable name. Node shape reflects the kind —
//! inputs are parallelograms, tool runs rectangles, specialist runs stadiums,
//! workflow runs subroutines, and plain data declarations rounded.

use std::collections::{BTreeSet, HashMap};

use crate::workflow::ast::{AccessKey, Expr, Workflow};

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
        nodes.push_str(&format!("    {id}{}\n", shape(Kind::Input, &input.name)));
        defs.insert(input.name.clone(), id);
    }

    for stmt in &workflow.statements {
        let id = format!("n{counter}");
        counter += 1;

        // Edges from the current definition of each referenced variable, each
        // labeled with that variable. Read `defs` before updating it, so a
        // self-reference (`x = x + 1`) points at the previous `x`, not this node.
        let mut vars = BTreeSet::new();
        free_vars(&stmt.expr, &mut vars);
        for v in &vars {
            if let Some(src) = defs.get(v) {
                edges.push_str(&format!("    {src} --{v}--> {id}\n"));
            }
        }

        let label = node_label(&stmt.name, &stmt.expr);
        nodes.push_str(&format!("    {id}{}\n", shape(kind(&stmt.expr), &label)));
        defs.insert(stmt.name.clone(), id);
    }

    let mut out = String::from("flowchart TD\n");
    out.push_str(&nodes);
    out.push_str(&edges);
    out
}

/// The variable names an expression reads, each as the top-level variable —
/// `weather.summary` contributes `weather`, not the access path. `for [item:
/// array] (body)` binds `item` within `body` only (workflow-dsl.md §6.5), so it
/// is removed from the body's free variables before merging — otherwise the
/// loop variable would be mistaken for an outer reference and draw a phantom
/// edge.
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
        // `do (...) while (...) max (...)` binds the loop's running value to `var`
        // (the assigned name) within `cond` only, so that self-reference is
        // removed from the condition's free variables before merging.
        Expr::Do {
            var,
            body,
            cond,
            max,
        } => {
            free_vars(body, out);
            free_vars(max, out);
            let mut inner = BTreeSet::new();
            free_vars(cond, &mut inner);
            inner.remove(var);
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

/// The name a node displays: the callee for a `run` statement, otherwise the
/// statement's own variable name.
fn node_label(name: &str, expr: &Expr) -> String {
    match expr {
        Expr::RunTool { tool, .. } => tool.clone(),
        Expr::RunSpecialist { specialist, .. } => specialist.clone(),
        Expr::RunWorkflow { workflow, .. } => workflow.clone(),
        _ => name.to_string(),
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

#[cfg(test)]
#[path = "render_tests.rs"]
mod tests;
