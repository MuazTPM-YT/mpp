pub mod html;
pub mod report;

use crate::compile::{self};
use crate::diag::Sources;
use crate::driver::SharedOut;
use crate::stdlib::generators::Gen;
use crate::syntax::ast::{StmtKind, TestKind};
use crate::vm::*;
use serde::Serialize;
use serde_json::Value as J;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct Options {
    pub filter: Option<String>,
    pub jobs: usize,
    pub seed: u64,
    pub retries: u32,
    pub timeout: f64,
    pub update_snapshots: bool,
    pub fail_fast: bool,
    pub bench: bool,
    // "file::name" -> mean ns
    pub baseline: Option<HashMap<String, f64>>,
    pub max_regress: f64,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            filter: None,
            jobs: 1,
            seed: 0,
            retries: 0,
            timeout: 60.0,
            update_snapshots: false,
            fail_fast: false,
            bench: false,
            baseline: None,
            max_regress: 0.10,
        }
    }
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Passed,
    Failed,
    Error,
    Skipped,
}

#[derive(Serialize, Clone, Debug)]
pub struct ReportItem {
    pub label: String,
    pub text: String,
    pub value: J,
}

#[derive(Serialize, Clone, Debug)]
pub struct BenchStats {
    pub iters: usize,
    pub mean_ns: f64,
    pub sd_ns: f64,
    pub min_ns: f64,
    pub p50_ns: f64,
    pub p95_ns: f64,
    pub max_ns: f64,
    pub ops_per_sec: f64,
    pub baseline_ns: Option<f64>,
    pub change: Option<f64>,
}

#[derive(Serialize, Clone, Debug)]
pub struct TestResult {
    pub file: String,
    pub name: String,
    pub kind: String,
    pub line: u32,
    pub status: Status,
    pub duration_ms: f64,
    pub message: Option<String>,
    pub location: Option<String>,
    pub trace: Option<String>,
    pub output: String,
    pub attempts: u32,
    pub reports: Vec<ReportItem>,
    pub notes: Vec<String>,
    pub bench: Option<BenchStats>,
    pub counterexample: Option<String>,
    pub runs: Option<u32>,
    pub seed: u64,
}

#[derive(Serialize, Clone, Debug)]
pub struct FileResult {
    pub file: String,
    pub duration_ms: f64,
    pub module_output: String,
    pub results: Vec<TestResult>,
}

// saved snapshot values for one test file
pub struct Snapshots {
    pub path: PathBuf,
    pub data: serde_json::Map<String, J>,
    pub dirty: bool,
}

impl Snapshots {
    fn load(test_file: &str) -> Snapshots {
        let p = Path::new(test_file);
        let stem = p.file_stem().map_or("tests".into(), |s| s.to_string_lossy().to_string());
        let path = p.parent().unwrap_or(Path::new("")).join("__snapshots__").join(format!("{stem}.snap.json"));
        let data = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Snapshots { path, data, dirty: false }
    }

    fn save(&self) -> std::io::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        if let Some(d) = self.path.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(&self.path, serde_json::to_string_pretty(&self.data).unwrap_or_default() + "\n")
    }
}

fn kind_of(k: u8) -> TestKind {
    match k {
        0 => TestKind::Test,
        1 => TestKind::Experiment,
        2 => TestKind::Bench,
        _ => TestKind::Property,
    }
}

// files to run: given files, plus .mpp files under dirs that hold matching blocks
pub fn discover(paths: &[String], bench: bool) -> Vec<String> {
    let mut out = Vec::new();
    for p in paths {
        let path = Path::new(p);
        if path.is_dir() {
            walk(path, bench, &mut out);
        } else {
            out.push(p.clone());
        }
    }
    out.sort();
    out.dedup();
    out
}

fn walk(dir: &Path, bench: bool, out: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if p.is_dir() {
            if !name.starts_with('.') && !matches!(name.as_str(), "target" | "node_modules" | "__snapshots__") {
                walk(&p, bench, out);
            }
        } else if name.ends_with(".mpp") && has_blocks(&p, bench) {
            out.push(p.to_string_lossy().replace('\\', "/"));
        }
    }
}

// cheap check without running anything; broken files count so their error shows
fn has_blocks(p: &Path, bench: bool) -> bool {
    let Ok(src) = std::fs::read_to_string(p) else { return false };
    match crate::syntax::parse(&src) {
        Ok(ast) => ast.iter().any(|s| matches!(&s.kind, StmtKind::TestBlock { kind, .. } if (*kind == TestKind::Bench) == bench)),
        Err(_) => true,
    }
}

fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

fn opt_num(opts: &Value, key: &str) -> Option<f64> {
    match opts {
        Value::Map(m) => m.borrow().get(&Key::Str(key.into())).and_then(|v| v.num(key).ok()),
        _ => None,
    }
}

// error value to (status, message, location, trace)
fn classify(e: &Flow) -> (Status, String, Option<String>, Option<String>) {
    match e {
        Flow::Exit(c) => (Status::Error, format!("exit({c}) called inside a test"), None, None),
        Flow::Throw(Value::Error(obj)) => {
            let t = obj.trace.borrow();
            let loc = t.first().map(|l| format!("{}:{}:{}", l.file, l.line, l.col));
            let trace = (t.len() > 2).then(|| format_trace(&t));
            match &*obj.kind {
                "ExpectFailed" => (Status::Failed, obj.message.to_string(), loc, trace),
                "Skipped" => (Status::Skipped, obj.message.to_string(), None, None),
                k => (Status::Error, format!("{k}: {}", obj.message), loc, trace),
            }
        }
        Flow::Throw(v) => (Status::Error, format!("{v:?}"), None, None),
    }
}

fn file_error(file: &str, msg: String, output: String) -> FileResult {
    let r = TestResult {
        file: file.into(),
        name: "<file>".into(),
        kind: "file".into(),
        line: 0,
        status: Status::Error,
        duration_ms: 0.0,
        message: Some(msg),
        location: None,
        trace: None,
        output,
        attempts: 1,
        reports: vec![],
        notes: vec![],
        bench: None,
        counterexample: None,
        runs: None,
        seed: 0,
    };
    FileResult { file: file.into(), duration_ms: 0.0, module_output: String::new(), results: vec![r] }
}

pub fn run_file(file: &str, opts: &Options) -> FileResult {
    let t0 = Instant::now();
    let mut sources = Sources::default();
    let program = match compile::load_program(file, &mut sources) {
        Ok(p) => p,
        Err(ds) => {
            let msg = ds.iter().map(|(f, d)| sources.render(f, d)).collect::<String>();
            return file_error(file, format!("does not compile:\n{}", msg.trim_end()), String::new());
        }
    };
    let buf = SharedOut::default();
    let mut vm = Vm::new(program, Box::new(buf.clone()));
    vm.rng = crate::stdlib::rand::Rng::new(opts.seed);
    vm.deadline = Some(Instant::now() + Duration::from_secs_f64(opts.timeout));
    if let Err(e) = vm.run_main() {
        let (_, msg, loc, _) = classify(&e);
        let at = loc.map(|l| format!("\n  at {l}")).unwrap_or_default();
        return file_error(file, format!("file crashed before tests ran: {msg}{at}"), buf.text());
    }
    let module_output = buf.text();
    let defs = std::mem::take(&mut vm.tests);
    let snaps = Rc::new(RefCell::new(Snapshots::load(file)));
    let filter = opts.filter.as_ref().map(|f| f.to_lowercase());
    let mut results = Vec::new();
    for def in &defs {
        let kind = kind_of(def.kind);
        if (kind == TestKind::Bench) != opts.bench {
            continue;
        }
        if filter.as_ref().is_some_and(|f| !def.name.to_lowercase().contains(f.as_str())) {
            continue;
        }
        let max_attempts = 1 + opt_num(&def.opts, "retries").map_or(opts.retries, |r| r.max(0.0) as u32);
        let mut attempt = 0;
        let r = loop {
            attempt += 1;
            let mut r = run_one(&mut vm, def, kind, opts, &snaps);
            r.attempts = attempt;
            if matches!(r.status, Status::Passed | Status::Skipped) || attempt >= max_attempts {
                break r;
            }
        };
        let stop = opts.fail_fast && matches!(r.status, Status::Failed | Status::Error);
        results.push(r);
        if stop {
            break;
        }
    }
    if let Err(e) = snaps.borrow().save() {
        eprintln!("warning: cannot save snapshots for {file}: {e}");
    }
    FileResult { file: file.into(), duration_ms: t0.elapsed().as_secs_f64() * 1000.0, module_output, results }
}

fn run_one(vm: &mut Vm, def: &TestDef, kind: TestKind, opts: &Options, snaps: &Rc<RefCell<Snapshots>>) -> TestResult {
    let seed = opts.seed ^ fnv(&format!("{}::{}", def.file, def.name));
    vm.rng = crate::stdlib::rand::Rng::new(seed);
    let buf = SharedOut::default();
    vm.out = Box::new(buf.clone());
    vm.test_ctx = Some(TestCtx {
        name: def.name.to_string(),
        snapshots: Some(snaps.clone()),
        update_snapshots: opts.update_snapshots,
        ..Default::default()
    });
    let limit = opt_num(&def.opts, "timeout").unwrap_or(opts.timeout);
    vm.deadline = Some(Instant::now() + Duration::from_secs_f64(limit.max(0.001)));
    let t0 = Instant::now();
    let mut counterexample = None;
    let mut runs = None;
    let mut bench = None;
    let outcome = match kind {
        TestKind::Test | TestKind::Experiment => vm.call(&def.func, &[]).map(|_| ()),
        TestKind::Property => {
            let (r, n, cx) = run_property(vm, def);
            runs = Some(n);
            counterexample = cx;
            r
        }
        TestKind::Bench => match run_bench(vm, def) {
            Ok(stats) => {
                bench = Some(stats);
                Ok(())
            }
            Err(e) => Err(e),
        },
    };
    let duration_ms = t0.elapsed().as_secs_f64() * 1000.0;
    vm.deadline = None;
    let _ = vm.out.flush();
    let ctx = vm.test_ctx.take().unwrap_or_default();
    let (mut status, mut message, location, trace) = match &outcome {
        Ok(()) => (Status::Passed, None, None, None),
        Err(e) => {
            let (s, m, l, t) = classify(e);
            (s, Some(m), l, t)
        }
    };
    if let (Some(b), Some(base)) = (&mut bench, &opts.baseline)
        && let Some(&old) = base.get(&format!("{}::{}", def.file, def.name))
    {
        let change = b.mean_ns / old - 1.0;
        b.baseline_ns = Some(old);
        b.change = Some(change);
        if change > opts.max_regress && status == Status::Passed {
            status = Status::Failed;
            message = Some(format!("{:.1}% slower than baseline (limit {:.0}%)", change * 100.0, opts.max_regress * 100.0));
        }
    }
    let mut reports = Vec::new();
    for (label, v) in ctx.reports {
        let text = vm.display(&v, false).unwrap_or_else(|_| "<unprintable>".into());
        let value = crate::stdlib::json::to_json(&v, 0).unwrap_or(J::String(text.clone()));
        reports.push(ReportItem { label, text, value });
    }
    TestResult {
        file: def.file.to_string(),
        name: def.name.to_string(),
        kind: kind.word().into(),
        line: def.line,
        status,
        duration_ms,
        message,
        location,
        trace,
        output: buf.text(),
        attempts: 1,
        reports,
        notes: ctx.notes,
        bench,
        counterexample,
        runs,
        seed,
    }
}

fn gens_of(def: &TestDef) -> Result<Vec<&Gen>, Flow> {
    def.gens
        .iter()
        .map(|g| {
            g.object::<Gen>()
                .ok_or_else(|| type_err(format!("property input must be a generator like gen.int(0, 9), got {}", g.kind_name())))
        })
        .collect()
}

// many random cases; on failure shrink to the smallest failing input
fn run_property(vm: &mut Vm, def: &TestDef) -> (Result<(), Flow>, u32, Option<String>) {
    let gens = match gens_of(def) {
        Ok(g) => g,
        Err(e) => return (Err(e), 0, None),
    };
    let names: Vec<Rc<str>> = match &def.func {
        Value::Func(c) => c.proto.params.clone(),
        _ => vec![],
    };
    let runs = opt_num(&def.opts, "runs").map_or(100, |n| n.max(1.0) as u32);
    for i in 0..runs {
        let size = (i as f64 + 1.0) / runs as f64;
        let args: Vec<Value> = gens.iter().map(|g| g.sample(&mut vm.rng, size)).collect();
        let Err(first) = vm.call(&def.func, &args) else { continue };
        if !is_case_failure(&first) {
            return (Err(first), i + 1, None);
        }
        let (args, err) = shrink(vm, def, &gens, args, first);
        let shown: Vec<String> =
            names.iter().zip(&args).map(|(n, v)| format!("{n} = {}", vm.display(v, true).unwrap_or_default())).collect();
        return (Err(err), i + 1, Some(shown.join(", ")));
    }
    (Ok(()), runs, None)
}

// errors that mean "this input breaks it" (not timeouts or skips)
fn is_case_failure(e: &Flow) -> bool {
    match e {
        Flow::Throw(Value::Error(o)) => !matches!(&*o.kind, "Skipped" | "TimeoutError"),
        Flow::Throw(_) => true,
        Flow::Exit(_) => false,
    }
}

fn shrink(vm: &mut Vm, def: &TestDef, gens: &[&Gen], mut args: Vec<Value>, mut err: Flow) -> (Vec<Value>, Flow) {
    let mut budget = 400;
    'outer: while budget > 0 {
        for i in 0..args.len() {
            for cand in gens[i].shrink(&args[i]) {
                budget -= 1;
                let mut trial = args.clone();
                trial[i] = cand;
                if let Err(e) = vm.call(&def.func, &trial)
                    && is_case_failure(&e)
                {
                    args = trial;
                    err = e;
                    continue 'outer;
                }
                if budget == 0 {
                    break 'outer;
                }
            }
        }
        break;
    }
    (args, err)
}

fn run_bench(vm: &mut Vm, def: &TestDef) -> Result<BenchStats, Flow> {
    let warmup = opt_num(&def.opts, "warmup").map_or(3, |n| n.max(0.0) as usize);
    let n = opt_num(&def.opts, "n").map(|n| n.max(1.0) as usize);
    let budget = Duration::from_secs_f64(opt_num(&def.opts, "time").unwrap_or(1.0).max(0.001));
    for _ in 0..warmup {
        vm.call(&def.func, &[])?;
    }
    let mut samples = Vec::new();
    let start = Instant::now();
    loop {
        let t = Instant::now();
        vm.call(&def.func, &[])?;
        samples.push(t.elapsed().as_nanos() as f64);
        let done = match n {
            Some(n) => samples.len() >= n,
            None => (start.elapsed() >= budget && samples.len() >= 10) || samples.len() >= 1_000_000,
        };
        if done {
            break;
        }
    }
    Ok(bench_stats(&mut samples))
}

pub fn bench_stats(samples: &mut [f64]) -> BenchStats {
    samples.sort_by(|a, b| a.total_cmp(b));
    let n = samples.len();
    let mean = samples.iter().sum::<f64>() / n as f64;
    let var = if n > 1 { samples.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1) as f64 } else { 0.0 };
    let q = |p: f64| samples[((n - 1) as f64 * p).round() as usize];
    BenchStats {
        iters: n,
        mean_ns: mean,
        sd_ns: var.sqrt(),
        min_ns: samples[0],
        p50_ns: q(0.5),
        p95_ns: q(0.95),
        max_ns: samples[n - 1],
        ops_per_sec: if mean > 0.0 { 1e9 / mean } else { f64::INFINITY },
        baseline_ns: None,
        change: None,
    }
}

// run files on `jobs` threads; on_done sees results in file order
pub fn run_all(files: &[String], opts: &Options, mut on_done: impl FnMut(&FileResult)) -> Vec<FileResult> {
    let jobs = opts.jobs.clamp(1, files.len().max(1));
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let (tx, rx) = std::sync::mpsc::channel::<(usize, FileResult)>();
    let mut done: Vec<(usize, FileResult)> = Vec::new();
    std::thread::scope(|s| {
        for _ in 0..jobs {
            let tx = tx.clone();
            let (next, stop) = (&next, &stop);
            std::thread::Builder::new()
                .stack_size(64 << 20)
                .spawn_scoped(s, move || {
                    loop {
                        let i = next.fetch_add(1, Ordering::SeqCst);
                        if i >= files.len() || stop.load(Ordering::SeqCst) {
                            break;
                        }
                        let r = run_file(&files[i], opts);
                        if opts.fail_fast && r.results.iter().any(|t| matches!(t.status, Status::Failed | Status::Error)) {
                            stop.store(true, Ordering::SeqCst);
                        }
                        if tx.send((i, r)).is_err() {
                            break;
                        }
                    }
                })
                .expect("spawn test thread");
        }
        drop(tx);
        let mut pending: HashMap<usize, FileResult> = HashMap::new();
        let mut want = 0;
        for (i, r) in rx {
            pending.insert(i, r);
            while let Some(r) = pending.remove(&want) {
                on_done(&r);
                done.push((want, r));
                want += 1;
            }
        }
        let mut rest: Vec<(usize, FileResult)> = pending.into_iter().collect();
        rest.sort_by_key(|(i, _)| *i);
        for (i, r) in rest {
            on_done(&r);
            done.push((i, r));
        }
    });
    done.into_iter().map(|(_, r)| r).collect()
}

#[derive(Serialize)]
pub struct Summary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub errors: usize,
    pub skipped: usize,
}

pub fn summarize(files: &[FileResult]) -> Summary {
    let all: Vec<&TestResult> = files.iter().flat_map(|f| &f.results).collect();
    let count = |s: Status| all.iter().filter(|t| t.status == s).count();
    Summary {
        total: all.len(),
        passed: count(Status::Passed),
        failed: count(Status::Failed),
        errors: count(Status::Error),
        skipped: count(Status::Skipped),
    }
}

// everything a report file needs
#[derive(Serialize)]
pub struct RunReport {
    pub mpp_version: String,
    pub mode: String,
    pub seed: u64,
    pub started: String,
    pub duration_s: f64,
    pub summary: Summary,
    pub files: Vec<FileResult>,
}
