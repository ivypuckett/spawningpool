//! Workflow DSL: parse, type-check, and evaluate orchestration scripts that
//! chain tools and specialists with typed data flow.
//!
//! See `docs/workflow-dsl.md` for the full language specification (v1).
//!
//! ## Usage
//!
//! ```rust,no_run
//! use spawningpool::workflow::{parse, check, eval};
//! use spawningpool::ai::{Client, CompleteOptions};
//! use spawningpool::store;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let source = r#"
//! city = "Portland"
//!
//! weather = call get_weather { CITY: city }
//!
//! result = { "city": city, "ok": weather.reachable }
//! "#;
//!
//! let workflow = parse(source)?;
//!
//! let registry = store::load()?;
//! let tools_dir = store::tools_dir();
//! let tools = spawningpool::tools::resolve_all(&tools_dir, &["get_weather".to_string()])?;
//!
//! // Optional: type-check before running.
//! check(&workflow, &registry, &tools)?;
//!
//! let client = Client::new();
//! let opts = CompleteOptions::default();
//! let result = eval(&workflow, &registry, &tools, &client, &opts).await?;
//! println!("{result}");
//! # Ok(())
//! # }
//! ```

pub mod ast;
pub mod check;
pub mod eval;
pub mod parse;

pub use ast::{AccessKey, BinOp, Expr, Statement, Workflow};
pub use check::{check, specialist_return_type, TypeEnv, TypeError};
pub use eval::{eval, WorkflowError};
pub use parse::{parse, ParseError};
