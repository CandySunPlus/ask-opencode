mod cli;
mod config;
mod context;
mod filter;
mod generate;
mod io;
mod opencode;
mod parse;
mod picker;
mod select;
mod validate;

use clap::Parser;

fn main() {
    let cli = cli::Cli::parse();
    let code = match cli.command {
        cli::Command::Generate(args) => generate::run(args),
        cli::Command::Parse(args) => parse::run(args),
        cli::Command::Validate(args) => validate::run(args),
        cli::Command::Select(args) => select::run(args),
    };
    std::process::exit(code);
}
