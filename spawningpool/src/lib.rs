pub mod ai;
pub mod domain;

mod script;
pub use script::{run_script, summarize, ScriptRun, ScriptSummary};

pub use domain::{ModelDef, ProviderDef, Registry, Specialist, ToolDef};
