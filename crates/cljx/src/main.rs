use clap::{Parser, Subcommand};
use miette::IntoDiagnostic as _;
use std::path::PathBuf;

/// clojurust — a Rust-hosted dialect of the Clojure programming language.
#[derive(Parser)]
#[command(name = "cljx", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Interpret a .cljx or .cljc source file.
    Run {
        /// Path to the source file.
        file: PathBuf,
    },
    /// Start an interactive REPL.
    Repl,
    /// AOT-compile a source file to a native binary.
    Compile {
        /// Path to the source file.
        file: PathBuf,
        /// Output binary path.
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Evaluate a single Clojure expression and print the result.
    Eval {
        /// The expression to evaluate.
        expr: String,
    },
}

fn main() -> miette::Result<()> {
    miette::set_hook(Box::new(|_| {
        Box::new(
            miette::MietteHandlerOpts::new()
                .terminal_links(true)
                .build(),
        )
    }))
    .into_diagnostic()?;

    let cli = Cli::parse();
    run(cli)
}

fn run(cli: Cli) -> miette::Result<()> {
    match cli.command {
        Commands::Run { file } => {
            eprintln!("[not yet implemented] cljx run {}", file.display());
        }
        Commands::Repl => {
            eprintln!("[not yet implemented] cljx repl");
        }
        Commands::Compile { file, out } => {
            eprintln!(
                "[not yet implemented] cljx compile {} -o {}",
                file.display(),
                out.display()
            );
        }
        Commands::Eval { expr } => {
            eprintln!("[not yet implemented] cljx eval {:?}", expr);
        }
    }
    Ok(())
}
