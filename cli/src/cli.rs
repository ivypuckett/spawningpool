use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "spawningpool", bin_name = "spawningpool", version, about)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

#[derive(clap::ValueEnum, Clone)]
pub(crate) enum OutputFormat {
    Json,
    Plaintext,
}

/// How `show workflow` renders a workflow.
#[derive(clap::ValueEnum, Clone)]
pub(crate) enum WorkflowFormat {
    /// The workflow's DSL source, verbatim.
    Source,
    /// A Mermaid `flowchart` of the workflow's data flow.
    Mermaid,
}

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Run a specialist, workflow, or tool.
    #[command(alias = "spawn")]
    Run {
        #[command(subcommand)]
        target: RunTarget,
    },
    /// List defined entities.
    List {
        #[command(subcommand)]
        kind: ListKind,
    },
    /// Show a defined entity's full definition.
    Show {
        #[command(subcommand)]
        entity: ShowEntity,
    },
    /// Define an entity.
    Define {
        #[command(subcommand)]
        entity: DefineEntity,
    },
    /// Delete an entity.
    Delete {
        #[command(subcommand)]
        entity: DeleteEntity,
        /// Skip the confirmation prompt.
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Hold a human-in-the-loop conversation over a one-turn workflow, picking
    /// `discuss`/`summarize`/`continue` each turn. The runner owns the loop and
    /// the carried conversation window; see `docs/human-in-the-loop.md`.
    Chat {
        /// The one-turn workflow to drive, from the `workflows/` folder.
        name: String,
        /// Resume an existing run by id, continuing its conversation window.
        #[arg(long, value_name = "RUN_ID")]
        resume: Option<String>,
    },
    /// Browse and manage everything in an interactive terminal UI.
    Tui,
}

#[derive(Subcommand)]
pub(crate) enum RunTarget {
    /// Run a specialist against a prompt.
    #[command(aliases = ["lenny", "ling"])]
    Specialist {
        name: String,
        #[arg(long)]
        prompt: String,
        /// Output format. Defaults to `json` (machine-readable envelope with
        /// output, thinking, token counts, stopReason, model, specialist,
        /// turns, and toolCalls). Use `plaintext` for streaming terminal output.
        #[arg(long, value_name = "FORMAT")]
        output: Option<OutputFormat>,
    },
    /// Execute a workflow from the `workflows/` folder, by name.
    #[command(alias = "overseer")]
    Workflow {
        name: String,
        /// A workflow input, as `KEY=VALUE`, matching a `# inputs:` entry.
        /// Repeatable.
        #[arg(long = "arg", value_name = "KEY=VALUE")]
        args: Vec<String>,
    },
    /// Run a single tool script directly, by name.
    Tool {
        name: String,
        /// A tool parameter, as `KEY=VALUE`. Repeatable.
        #[arg(long = "arg", value_name = "KEY=VALUE")]
        args: Vec<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum ListKind {
    #[command(aliases = ["specialist", "lenny", "ling", "lennys", "lings"])]
    Specialists,
    #[command(alias = "provider")]
    Providers,
    #[command(alias = "model")]
    Models {
        /// Discover the models a running LM Studio server currently has loaded
        /// (at `$LMSTUDIO_BASE_URL`) instead of listing the registry.
        #[arg(long)]
        remote: bool,
    },
    #[command(alias = "tool")]
    Tools,
}

#[derive(Subcommand)]
pub(crate) enum ShowEntity {
    #[command(aliases = ["lenny", "ling"])]
    Specialist {
        name: String,
    },
    Provider {
        name: String,
    },
    Model {
        name: String,
    },
    Tool {
        name: String,
    },
    /// Show a workflow from the `workflows/` folder, by name.
    #[command(alias = "overseer")]
    Workflow {
        name: String,
        /// Output format. Defaults to `source` (the DSL verbatim); use `mermaid`
        /// for a `flowchart` of the workflow's data flow.
        #[arg(long, value_name = "FORMAT", default_value = "source")]
        format: WorkflowFormat,
    },
}

#[derive(Subcommand)]
pub(crate) enum DefineEntity {
    /// Define a provider (wire protocol + endpoint + key env var).
    Provider {
        name: String,
        #[arg(long)]
        api: String,
        #[arg(long)]
        base_url: String,
        #[arg(long)]
        api_key_env: Option<String>,
        /// Declare that this provider's endpoint supports constrained decoding,
        /// so constrained specialists force their tool call via grammar-constrained
        /// `response_format` instead of `tool_choice`. OpenAI-compatible only.
        #[arg(long)]
        constrained_decoding: bool,
    },
    /// Define a model, keyed by its API id, against a provider.
    Model {
        id: String,
        #[arg(long)]
        provider: String,
        /// Display name; defaults to the id.
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        max_tokens: u32,
        #[arg(long)]
        context_window: u32,
    },
    /// Define a specialist template.
    #[command(aliases = ["lenny", "ling"])]
    Specialist {
        name: String,
        #[arg(long)]
        provider: String,
        #[arg(long)]
        model: String,
        #[arg(long)]
        system_prompt: String,
        /// Comma-separated tool names.
        #[arg(long)]
        tools: Option<String>,
        /// A tool the specialist is forced to call. Realized via the portable
        /// tool-call trick (forced `tool_choice`), or grammar-constrained decoding
        /// if the provider was defined `--constrained-decoding`.
        #[arg(long)]
        constraint: Option<String>,
        #[arg(long, default_value = "off")]
        reasoning: String,
        /// Stream the response incrementally when this specialist runs.
        #[arg(long)]
        stream: bool,
    },
    /// Define a tool from an executable script; its `# desc:` and `# params:`
    /// header comments become the description and parameters.
    Tool {
        name: String,
        #[arg(long)]
        script: PathBuf,
    },
}

#[derive(Subcommand)]
pub(crate) enum DeleteEntity {
    #[command(aliases = ["lenny", "ling"])]
    Specialist {
        name: String,
    },
    Provider {
        name: String,
    },
    Model {
        name: String,
    },
    Tool {
        name: String,
    },
}
