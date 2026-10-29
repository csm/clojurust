//! `cljrs repl` — the interactive read-eval-print loop.

use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;

use cljrs_runtime::tiered::{Env, GlobalEnv};
use cljrs_value::Value;

use crate::session::{self, VersioningFlags};

#[derive(clap::Args)]
pub struct Args {
    /// Source directories to search when resolving `require`.
    #[arg(long = "src-path", value_name = "DIR")]
    pub src_paths: Vec<PathBuf>,
    /// GC soft memory limit in MB (each isolate heap collects when it exceeds this).
    #[arg(long)]
    pub gc_soft_limit_mb: Option<usize>,
    /// Process managed-memory hard limit in MB (enforced only with CLJRS_MEMORY_STRICT=1).
    #[arg(long)]
    pub gc_hard_limit_mb: Option<usize>,
}

pub fn run(args: Args, versioning: VersioningFlags) -> miette::Result<i32> {
    let gc_config = session::build_gc_config(args.gc_soft_limit_mb, args.gc_hard_limit_mb)?;
    let globals = session::setup_globals(args.src_paths, gc_config, versioning);
    run_repl(globals);
    Ok(0)
}

fn run_repl(globals: Arc<GlobalEnv>) {
    println!("clojurust REPL (type :quit to exit)");
    println!();

    #[cfg(feature = "enable-rustyline")]
    let mut rl = rustyline::DefaultEditor::new().unwrap();

    let mut env = Env::new(globals, "user");

    let stdin = io::stdin();
    let mut input_buf = String::new();
    let mut depth: i32 = 0;

    #[cfg(feature = "enable-rustyline")]
    loop {
        let readline = rl.readline("=> ");
        match readline {
            Ok(line) => {
                rl.add_history_entry(line.as_str());
                if line.is_empty() {
                    continue;
                } else if line.starts_with(":quit") {
                    break;
                } else {
                    #[cfg(feature = "no-gc")]
                    let _reads = cljrs_gc::region::ReadScope::enter();
                    match session::eval_in(&mut env, &line, "<repl>") {
                        Ok(Value::Nil) => println!("nil"),
                        Ok(v) => println!("{}", v),
                        Err(e) => println!("error: {}", e),
                    }
                }
            }
            Err(rustyline::error::ReadlineError::Interrupted) => break,
            Err(rustyline::error::ReadlineError::Eof) => break,
            Err(err) => {
                eprintln!("error: {}", err);
                break;
            }
        }
    }

    #[cfg(not(feature = "enable-rustyline"))]
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
            #[cfg(feature = "no-gc")]
            let _reads = cljrs_gc::region::ReadScope::enter();
            match session::eval_in(&mut env, &src, "<repl>") {
                Ok(Value::Nil) => {}
                Ok(v) => println!("{}", v),
                Err(e) => eprintln!("Error: {e}"),
            }
        }
    }

    println!("Bye.");
}
