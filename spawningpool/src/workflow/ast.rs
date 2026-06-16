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
}

/// Access key used in a member-access chain (workflow-dsl.md §6.9).
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
    /// Tool call `call tool_name { KEY: expr, ... }` (workflow-dsl.md §6.6).
    Call {
        tool: String,
        args: Vec<(String, Expr)>,
    },
    /// Workflow call `run workflow_name { KEY: expr, ... }` (workflow-dsl.md
    /// §6.8). Supplies the callee's declared `# inputs:` by name and yields the
    /// callee's result value; the `run` verb (vs `call`) selects the `workflows/`
    /// namespace, so a tool and a workflow may share a name without ambiguity.
    Run {
        workflow: String,
        args: Vec<(String, Expr)>,
    },
    /// Specialist call `ask specialist prompt_expr` (workflow-dsl.md §6.7).
    Ask {
        specialist: String,
        prompt: Box<Expr>,
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
