//! The `spawningpool` CLI. Defines providers, models, specialists, and tools
//! into a persisted [`Registry`], and runs a specialist against a prompt.

mod cli;
mod commands;
mod display;
mod log;
mod tui;

use clap::Parser;
use cli::{Cli, Command, RunTarget};

#[tokio::main]
async fn main() {
    if let Err(e) = run(Cli::parse()).await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        // Bare `spawningpool` reads where you are in the provider → model → specialist →
        // run progression and shows the next step.
        None => display::status(),
        Some(Command::Run { target }) => match target {
            RunTarget::Specialist {
                name,
                prompt,
                output,
            } => commands::run::run_specialist(&name, &prompt, output).await,
            RunTarget::Workflow { name, args } => commands::run::run_workflow(&name, &args).await,
            RunTarget::Tool { name, args } => commands::run::run_tool(&name, &args),
        },
        Some(Command::List { kind }) => commands::list::list(kind).await,
        Some(Command::Show { entity }) => commands::show::show(entity),
        Some(Command::Define { entity }) => commands::define::define(entity),
        Some(Command::Delete { entity, yes }) => commands::delete::delete(entity, yes),
        Some(Command::Tui) => tui::launch().await,
    }
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
