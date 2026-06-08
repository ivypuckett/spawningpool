pub mod ai;
pub mod domain;

mod script;
pub use script::{prepare_script, run_script, summarize, ScriptError, ScriptRun, ScriptSummary};

pub use domain::{
    EntityKind, MissingRef, ModelDef, ProviderDef, Referrer, Registry, Specialist, ToolDef,
};
