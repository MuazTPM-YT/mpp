use clap::{Parser, Subcommand};
use mpp::compile::{self, emit};
use mpp::diag::Sources;
use mpp::driver;
use mpp::syntax;
use mpp::vm::Vm;
use std::process::ExitCode;
use std::rc::Rc;

#[derive(Parser)]
#[command(name = "mpp", version, about = "Muaz++: compiler and runtime for .mpp test and analysis programs")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Compile and run a .mpp (or .mppc) file
    Run {
        file: String,
        /// Seed for the random generator (default: random)
        #[arg(long)]
        seed: Option<u64>,
        /// Arguments passed to the program, see args()
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Check files for errors without running them
    Check {
        #[arg(required = true)]
        files: Vec<String>,
    },
    /// Compile to bytecode (.mppc) or a standalone executable
    Build {
        file: String,
        /// Output path
        #[arg(short, long)]
        out: Option<String>,
        /// Make a standalone executable instead of .mppc
        #[arg(long)]
        exe: bool,
    },
    /// Interactive prompt
    Repl,
}

fn random_seed() -> u64 {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    t ^ (std::process::id() as u64).rotate_left(32)
}

fn code(n: i32) -> ExitCode {
    ExitCode::from(n.clamp(0, 255) as u8)
}

fn main() -> ExitCode {
    mpp::stdlib::core::load_dotenv(std::path::Path::new(".env"));
    if let Some(program) = driver::embedded_program() {
        let argv = std::env::args().skip(1).collect();
        return code(driver::run_program(program, &Sources::default(), argv, random_seed()));
    }
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Run { file, seed, args } => {
            if let Some(dir) = std::path::Path::new(&file).parent() {
                mpp::stdlib::core::load_dotenv(&dir.join(".env"));
            }
            code(driver::run_file(&file, args, seed.unwrap_or_else(random_seed)))
        }
        Cmd::Check { files } => {
            let mut bad = 0;
            for f in &files {
                let mut sources = Sources::default();
                match compile::load_program(f, &mut sources) {
                    Ok(_) => println!("ok  {f}"),
                    Err(d) => {
                        bad += 1;
                        driver::report_diags(&sources, &d);
                    }
                }
            }
            code(if bad > 0 { 2 } else { 0 })
        }
        Cmd::Build { file, out, exe } => {
            let mut sources = Sources::default();
            let program = match compile::load_program(&file, &mut sources) {
                Ok(p) => p,
                Err(d) => {
                    driver::report_diags(&sources, &d);
                    return code(2);
                }
            };
            let stem = std::path::Path::new(&file).with_extension("");
            let res = if exe {
                let out = out.unwrap_or_else(|| stem.to_string_lossy().to_string());
                driver::build_exe(&program, &out).map(|_| out)
            } else {
                let out = out.unwrap_or_else(|| stem.with_extension("mppc").to_string_lossy().to_string());
                std::fs::write(&out, program.to_bytes()).map(|_| out)
            };
            match res {
                Ok(o) => {
                    println!("built {o}");
                    code(0)
                }
                Err(e) => {
                    eprintln!("error: cannot write output: {e}");
                    code(1)
                }
            }
        }
        Cmd::Repl => repl(),
    }
}

fn repl() -> ExitCode {
    let Ok(mut ed) = rustyline::DefaultEditor::new() else {
        eprintln!("error: cannot open terminal");
        return code(1);
    };
    println!("Muaz++ {} — type code, `exit()` to quit", env!("CARGO_PKG_VERSION"));
    let mut sources = Sources::default();
    let program = compile::chunk::Program { entry: "<repl>".into(), modules: vec![] };
    let mut vm = Vm::new(program, Box::new(std::io::stdout()));
    vm.rng = mpp::stdlib::rand::Rng::new(random_seed());
    let mut globals: Vec<Rc<str>> = Vec::new();
    let mut buf = String::new();
    let mut n = 0;
    loop {
        let prompt = if buf.is_empty() { ">>> " } else { "... " };
        let line = match ed.readline(prompt) {
            Ok(l) => l,
            Err(rustyline::error::ReadlineError::Interrupted) => {
                buf.clear();
                continue;
            }
            Err(_) => break,
        };
        buf.push_str(&line);
        buf.push('\n');
        if syntax::lexer::lex(&buf, 0).is_err_and(|d| d.msg.contains("never closed")) {
            continue;
        }
        let _ = ed.add_history_entry(buf.trim_end());
        n += 1;
        let name = format!("<repl:{n}>");
        let src = std::mem::take(&mut buf);
        sources.add(&name, src.clone());
        let compiled = syntax::parse(&src).and_then(|ast| emit::compile_module(&ast, &name, &src, &globals, true));
        let m = match compiled {
            Ok(m) => m,
            Err(ds) => {
                for d in &ds {
                    sources.report(&name, d);
                }
                continue;
            }
        };
        let queue = m.imports.iter().map(|(k, s)| (k.clone(), Some((Rc::from(name.as_str()), *s)))).collect();
        if let Err(ds) = compile::load_into(queue, &mut sources, &mut vm.program.modules) {
            driver::report_diags(&sources, &ds);
            continue;
        }
        globals = m.proto.globals.clone();
        if let Err(f) = vm.run_repl(m.proto) {
            if let mpp::vm::Flow::Exit(c) = f {
                return code(c);
            }
            driver::report_flow(&sources, &f);
        }
    }
    code(0)
}
