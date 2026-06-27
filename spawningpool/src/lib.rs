#![doc = include_str!("../README.md")]

pub mod ai;
pub mod domain;
pub mod log;
pub mod run;
pub mod store;
pub mod tools;
pub mod types;
pub mod workflow;

mod script;
pub use script::{prepare_script, run_script, summarize, ScriptError, ScriptRun, ScriptSummary};

pub use log::{LogSink, SpecialistLog};

pub use run::{run_specialist, RunEvent, Session};

pub use domain::{
    EntityKind, MissingRef, ModelDef, ProviderDef, Referrer, Registry, Specialist, ToolDef,
};

pub use types::{ExitCode, Param, Type};
