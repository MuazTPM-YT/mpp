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
    /// Run test, experiment and property blocks
    Test(TestArgs),
    /// Run bench blocks
    Bench(TestArgs),
}

#[derive(clap::Args)]
struct TestArgs {
    /// Files or folders (default: current folder)
    paths: Vec<String>,
    /// Only run blocks whose name contains this text
    #[arg(short = 'k', long)]
    filter: Option<String>,
    /// Files to run at the same time (default: number of CPUs; bench always 1)
    #[arg(short, long)]
    jobs: Option<usize>,
    /// Seed for random numbers (default: random, printed at the end)
    #[arg(long)]
    seed: Option<u64>,
    /// Re-run a failing block up to N more times
    #[arg(long, default_value_t = 0)]
    retries: u32,
    /// Seconds before a block is stopped
    #[arg(long, default_value_t = 60.0)]
    timeout: f64,
    /// Accept changed snapshots
    #[arg(long)]
    update_snapshots: bool,
    /// Stop after the first failure
    #[arg(short = 'x', long)]
    fail_fast: bool,
    /// Show output and reports for passing blocks too
    #[arg(short, long)]
    verbose: bool,
    /// Write a report file: json:PATH, junit:PATH or html:PATH (repeat for more)
    #[arg(long = "report", value_name = "FORMAT:PATH")]
    reports: Vec<String>,
    /// List blocks without running them
    #[arg(long)]
    list: bool,
    /// Compare bench results with this saved baseline file
    #[arg(long)]
    baseline: Option<String>,
    /// Save bench results as a baseline file
    #[arg(long)]
    save_baseline: Option<String>,
    /// Fail a bench that is this many percent slower than baseline
    #[arg(long, default_value_t = 10.0)]
    max_regress: f64,
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
        Cmd::Test(a) => run_tests(a, false),
        Cmd::Bench(a) => run_tests(a, true),
    }
}

fn run_tests(a: TestArgs, bench: bool) -> ExitCode {
    use mpp::runner::{self, Options, RunReport, report};
    use std::io::IsTerminal;
    let mode = if bench { "bench" } else { "test" };
    let mut outs = Vec::new();
    for r in &a.reports {
        match r.split_once(':') {
            Some((f @ ("json" | "junit" | "html"), path)) if !path.is_empty() => outs.push((f.to_string(), path.to_string())),
            _ => {
                eprintln!("error: --report wants json:PATH, junit:PATH or html:PATH, got `{r}`");
                return code(2);
            }
        }
    }
    let paths = if a.paths.is_empty() { vec![".".to_string()] } else { a.paths.clone() };
    let files = runner::discover(&paths, bench);
    if files.is_empty() {
        eprintln!("no {mode} blocks found in {}", paths.join(", "));
        return code(5);
    }
    if a.list {
        for f in &files {
            let Ok(src) = std::fs::read_to_string(f) else { continue };
            let Ok(ast) = syntax::parse(&src) else {
                println!("{f}: does not parse");
                continue;
            };
            let lines = mpp::diag::LineIndex::new(&src);
            for s in &ast {
                if let syntax::ast::StmtKind::TestBlock { kind, name, .. } = &s.kind
                    && (*kind == syntax::ast::TestKind::Bench) == bench
                {
                    println!("{f}:{} {} {name:?}", lines.line_col(&src, s.span.start).0, kind.word());
                }
            }
        }
        return code(0);
    }
    let baseline = match &a.baseline {
        Some(p) => match std::fs::read_to_string(p).ok().and_then(|t| serde_json::from_str(&t).ok()) {
            Some(b) => Some(b),
            None => {
                eprintln!("error: cannot read baseline file {p}");
                return code(2);
            }
        },
        None => None,
    };
    let seed = a.seed.unwrap_or_else(|| random_seed() % 1_000_000);
    let jobs =
        if bench { 1 } else { a.jobs.unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get())) }.clamp(1, files.len());
    let opts = Options {
        filter: a.filter.clone(),
        jobs,
        seed,
        retries: a.retries,
        timeout: a.timeout,
        update_snapshots: a.update_snapshots,
        fail_fast: a.fail_fast,
        bench,
        baseline,
        max_regress: a.max_regress / 100.0,
    };
    let term = report::Term { color: std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none(), verbose: a.verbose };
    let started =
        mpp::stdlib::time::iso_utc(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64()));
    let t0 = std::time::Instant::now();
    println!(
        "mpp {mode} · {} file{} · seed {seed} · {jobs} job{}\n",
        files.len(),
        if files.len() == 1 { "" } else { "s" },
        if jobs == 1 { "" } else { "s" }
    );
    let results = runner::run_all(&files, &opts, |f| {
        if !f.results.is_empty() {
            print!("{}", term.file(f))
        }
    });
    let secs = t0.elapsed().as_secs_f64();
    let summary = runner::summarize(&results);
    print!("{}", term.summary(&summary, secs, seed, mode));
    let bad = summary.failed + summary.errors > 0;
    let run = RunReport {
        mpp_version: env!("CARGO_PKG_VERSION").into(),
        mode: mode.into(),
        seed,
        started,
        duration_s: secs,
        summary,
        files: results,
    };
    for (fmt, path) in &outs {
        let text = match fmt.as_str() {
            "json" => report::json(&run),
            "junit" => report::junit(&run),
            _ => runner::html::render(&run),
        };
        match std::fs::write(path, text) {
            Ok(()) => println!("wrote {fmt} report to {path}"),
            Err(e) => eprintln!("error: cannot write {path}: {e}"),
        }
    }
    if let Some(p) = &a.save_baseline {
        let map: serde_json::Map<String, serde_json::Value> = run
            .files
            .iter()
            .flat_map(|f| &f.results)
            .filter_map(|t| t.bench.as_ref().map(|b| (format!("{}::{}", t.file, t.name), serde_json::json!(b.mean_ns))))
            .collect();
        match std::fs::write(p, serde_json::to_string_pretty(&map).unwrap_or_default() + "\n") {
            Ok(()) => println!("saved baseline to {p}"),
            Err(e) => eprintln!("error: cannot write {p}: {e}"),
        }
    }
    code(if bad { 1 } else { 0 })
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
