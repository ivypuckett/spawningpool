//! AST types for the workflow DSL (workflow-dsl.md §5–6).

use crate::types::Param;

/// Binary operator. All operators have the same precedence and associate
/// left-to-right (workflow-dsl.md §6.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
    Or,
    And,
    /// Equality `==` / `!=`. Operands must share a type (any type); result is
    /// `bool`.
    Eq,
    Ne,
    /// Ordering `<` `<=` `>` `>=`. Operands must both be `number` or both be
    /// `string`; result is `bool`.
    Lt,
    Le,
    Gt,
    Ge,
}

/// Access key used in a member-access chain (workflow-dsl.md §6.7).
#[derive(Debug, Clone, PartialEq)]
pub enum AccessKey {
    /// `.ident` — literal key (bare identifier, not a variable).
    Ident(String),
    /// `."key"` — quoted literal key.
    Quoted(String),
    /// `.0` — numeric array index.
    Index(usize),
    /// `.(expr)` — computed access (v1: arrays only).
    Computed(Box<Expr>),
}

/// An expression in the workflow DSL (workflow-dsl.md §6).
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// String literal `"..."`.
    Str(String),
    /// Number literal (integer or float).
    Num(f64),
    /// Boolean literal `true` or `false`.
    Bool(bool),
    /// Object literal `{ "key": expr, ... }`. Keys are quoted strings.
    Object(Vec<(String, Expr)>),
    /// Variable reference — a camelCase name.
    Var(String),
    /// Unary logical negation `!expr`. Operand must be bool.
    Not(Box<Expr>),
    /// Binary operation `lhs op rhs`. No precedence; left-to-right.
    BinOp {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    /// Member access chain `base.key.key2...`.
    Access {
        base: Box<Expr>,
        keys: Vec<AccessKey>,
    },
    /// Selection `if (cond) then, ..., (_) default` (workflow-dsl.md §6.4).
    If {
        branches: Vec<(Expr, Expr)>,
        default: Box<Expr>,
    },
    /// Map iteration `for [item: array] (body)` (workflow-dsl.md §6.5).
    For {
        item: String,
        array: Box<Expr>,
        body: Box<Expr>,
    },
    /// Conditional repetition `do (body) while (cond) max (n)` (workflow-dsl.md
    /// §6.5). Evaluates `body`, binds its value to `var` (the assigned variable),
    /// then re-runs while `cond` is `true` — so `cond` can inspect the running
    /// value through `var`. The body always runs at least once and at most `max`
    /// times (a required cap, evaluated once in the outer scope). The loop's value
    /// is the body's final value, of the body's type. `var` is the enclosing
    /// statement's name; `do` is therefore only valid as a statement's whole RHS.
    Do {
        var: String,
        body: Box<Expr>,
        cond: Box<Expr>,
        max: Box<Expr>,
    },
    /// Tool run `run tool <name> { KEY: expr, ... }` (workflow-dsl.md §6.6),
    /// with an optional `else` recovery block (§7).
    RunTool {
        tool: String,
        args: Vec<(String, Expr)>,
        /// `else` recovery arms keyed by the tool's `# exits:` name: on a
        /// matching non-zero exit, the arm's value substitutes for the tool's
        /// output. Empty when there's no `else` block.
        recover: Vec<(String, Expr)>,
        /// The `else { ..., _: expr }` default arm, catching any non-zero exit
        /// not named in `recover` (including undeclared codes and signals).
        /// `None` when absent.
        recover_default: Option<Box<Expr>>,
    },
    /// Workflow run `run workflow <name> { KEY: expr, ... }` (workflow-dsl.md
    /// §6.6). Supplies the callee's declared `# inputs:` by name and yields the
    /// callee's result value. The `run` verb's `<kind>` (vs a bare name) selects
    /// the namespace, so a tool and a workflow may share a name without
    /// ambiguity.
    RunWorkflow {
        workflow: String,
        args: Vec<(String, Expr)>,
    },
    /// Specialist run `run specialist <name> <prompt-expr>` (workflow-dsl.md
    /// §6.6).
    RunSpecialist {
        specialist: String,
        prompt: Box<Expr>,
    },
    /// Ask the user `ask <prompt-expr> [else <string-expr>]` (workflow-dsl.md
    /// §6.8, docs/ask.md). Pauses the run and resolves to the user's reply as a
    /// `string`. The optional `fallback` supplies a single string when the
    /// question can't be answered (headless run, or the user cancels); with no
    /// fallback an un-answerable `ask` aborts the workflow. Unlike `run <kind>`
    /// it resolves no named on-disk entity, so it's a built-in keyword.
    Ask {
        prompt: Box<Expr>,
        fallback: Option<Box<Expr>>,
    },
}

/// A single assignment statement `name = expr`.
#[derive(Debug, Clone, PartialEq)]
pub struct Statement {
    pub name: String,
    pub expr: Expr,
}

/// A workflow: declared external inputs plus a flat sequence of assignment
/// statements separated by blank lines (workflow-dsl.md §5).
#[derive(Debug, Clone, PartialEq)]
pub struct Workflow {
    /// External inputs declared in the `# inputs:` header, supplied at run time
    /// (workflow-dsl.md §5.1). Each is in scope as a variable of its declared
    /// type from the first statement on. Empty when no header is present.
    pub inputs: Vec<Param>,
    pub statements: Vec<Statement>,
}
