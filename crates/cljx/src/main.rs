use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use miette::IntoDiagnostic as _;

use cljx_eval::{Env, EvalError, eval};
use cljx_stdlib::{standard_env, standard_env_with_paths};
use cljx_value::Value;

/// clojurust — a Rust-hosted dialect of the Clojure programming language.
#[derive(Parser)]
#[command(name = "cljx", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Interpret a .cljrs or .cljc source file.
    Run {
        /// Path to the source file.
        file: PathBuf,
        /// Source directories to search when resolving `require`.
        #[arg(long = "src-path", value_name = "DIR")]
        src_paths: Vec<PathBuf>,
    },
    /// Start an interactive REPL.
    Repl {
        /// Source directories to search when resolving `require`.
        #[arg(long = "src-path", value_name = "DIR")]
        src_paths: Vec<PathBuf>,
    },
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
        Commands::Run { file, src_paths } => {
            let src = std::fs::read_to_string(&file)
                .map_err(|e| miette::miette!("{}: {}", file.display(), e))?;
            let filename = file.display().to_string();
            run_source(&src, &filename, src_paths)?;
        }
        Commands::Repl { src_paths } => {
            run_repl(src_paths);
        }
        Commands::Compile { file, out } => {
            eprintln!(
                "[not yet implemented] cljx compile {} -o {}",
                file.display(),
                out.display()
            );
        }
        Commands::Eval { expr } => {
            let result = eval_source(&expr, "<eval>")?;
            if result != Value::Nil {
                println!("{}", result);
            }
        }
    }
    Ok(())
}

// ── Source evaluation ─────────────────────────────────────────────────────────

/// Evaluate all forms in `src`, printing nothing. Returns the last value.
fn eval_source(src: &str, filename: &str) -> miette::Result<Value> {
    let globals = standard_env();
    let mut env = Env::new(globals, "user");
    eval_in(&mut env, src, filename)
}

/// Run a source file: evaluate all top-level forms, print nothing on success.
fn run_source(src: &str, filename: &str, src_paths: Vec<PathBuf>) -> miette::Result<()> {
    let globals = standard_env_with_paths(src_paths);
    let mut env = Env::new(globals, "user");
    eval_in(&mut env, src, filename)?;
    Ok(())
}

/// Evaluate `src` in an existing `Env`. Returns the last value.
fn eval_in(env: &mut Env, src: &str, filename: &str) -> miette::Result<Value> {
    let mut parser = cljx_reader::Parser::new(src.to_string(), filename.to_string());
    let forms = parser
        .parse_all()
        .map_err(miette::Report::from)?;

    let mut result = Value::Nil;
    for form in forms {
        result = eval(&form, env).map_err(format_eval_error)?;
    }
    Ok(result)
}

fn format_eval_error(e: EvalError) -> miette::Report {
    match e {
        EvalError::Thrown(val) => miette::miette!("Unhandled exception: {}", val),
        EvalError::UnboundSymbol(s) => miette::miette!("Unable to resolve symbol: {}", s),
        EvalError::Arity {
            name,
            expected,
            got,
        } => miette::miette!("Wrong number of args ({got}) passed to {name}; expected {expected}"),
        EvalError::NotCallable(s) => miette::miette!("Not a function: {}", s),
        EvalError::Runtime(msg) => miette::miette!("{}", msg),
        EvalError::Read(e) => miette::Report::from(e),
        EvalError::Recur(_) => miette::miette!("recur outside of loop/fn"),
    }
}

// ── REPL ──────────────────────────────────────────────────────────────────────

fn run_repl(src_paths: Vec<PathBuf>) {
    println!("clojurust REPL (type :quit to exit)");
    println!();

    let globals = standard_env_with_paths(src_paths);
    let mut env = Env::new(globals, "user");

    let stdin = io::stdin();
    let mut input_buf = String::new();
    let mut depth: i32 = 0;

    loop {
        let prompt = if input_buf.is_empty() { "=> " } else { ".. " };
        print!("{}", prompt);
        io::stdout().flush().unwrap();

        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {}
            Err(e) => {
                eprintln!("I/O error: {e}");
                break;
            }
        }

        let trimmed = line.trim_end();

        if input_buf.is_empty() && trimmed == ":quit" {
            break;
        }

        // Track paren depth to support multi-line input.
        for ch in trimmed.chars() {
            match ch {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                _ => {}
            }
        }

        if !input_buf.is_empty() {
            input_buf.push('\n');
        }
        input_buf.push_str(trimmed);

        // Only evaluate when parens are balanced (or we have a bare atom).
        if depth <= 0 && !input_buf.trim().is_empty() {
            depth = 0;
            let src = std::mem::take(&mut input_buf);
            match eval_in(&mut env, &src, "<repl>") {
                Ok(Value::Nil) => {}
                Ok(v) => println!("{}", v),
                Err(e) => eprintln!("Error: {e}"),
            }
        }
    }

    println!("Bye.");
}
