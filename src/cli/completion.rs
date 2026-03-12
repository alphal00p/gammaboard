use anyhow::Result;
use clap::{Args, CommandFactory, ValueEnum};
use clap_complete::{Generator, Shell, generate};

use super::Cli;

#[derive(Debug, Args)]
pub struct CompletionArgs {
    #[arg(value_enum)]
    pub shell: CompletionShell,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CompletionShell {
    Bash,
    Elvish,
    Fish,
    PowerShell,
    Zsh,
}

pub fn run_completion(args: CompletionArgs) -> Result<()> {
    let mut command = Cli::command();
    let command_name = command.get_name().to_string();
    match args.shell {
        CompletionShell::Bash => print_completion(Shell::Bash, &mut command, &command_name),
        CompletionShell::Elvish => print_completion(Shell::Elvish, &mut command, &command_name),
        CompletionShell::Fish => print_completion(Shell::Fish, &mut command, &command_name),
        CompletionShell::PowerShell => {
            print_completion(Shell::PowerShell, &mut command, &command_name)
        }
        CompletionShell::Zsh => print_completion(Shell::Zsh, &mut command, &command_name),
    }
    Ok(())
}

fn print_completion<G: Generator>(generator: G, command: &mut clap::Command, command_name: &str) {
    generate(generator, command, command_name, &mut std::io::stdout());
}
