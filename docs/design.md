# Muaz++ (`.mpp`) — language, compiler and testing toolkit

## Context

The user wants a new programming language, Muaz++, with files that end in `.mpp`. Its job is
to make testing and analysis easy: A/B tests, statistics, ML model tests, and tests for the
user's own local LLMs (connect to them, evaluate them, debug their training runs).
It must feel like Python (simple, no type noise) and look like C++ (`{}` blocks). It must be fast.
It must be production grade.

Decisions the user made:
- Engine: Rust compiler to bytecode, run by a fast Rust VM. All heavy math is native Rust.
- Types: dynamic, like Python. No type annotations.
- Tests: built-in language blocks (`test`, `experiment`, `bench`, `property`), not library calls.
- LLM: no cloud LLM APIs. Only the user's own local models, reached through
  (1) a local HTTP server, (2) a script over JSON lines on stdin/stdout, (3) training log files.

Repo state: `/home/Muaz/Documents/Software/Muaz++` is empty except `CLAUDE.md`. Not a git repo.
Tools present: Rust 1.93 / cargo, clang, cmake, python3 (no scipy). No LLVM dev libs (not needed).

Project rules (CLAUDE.md): caveman commit messages, NO Claude attribution anywhere in commits/PRs,
secrets only in `.env`, pin every dependency to a minor version (`~X.Y`), one-line comments above
functions, stop and ask when unsure.

## The language (core syntax)

```
# comments use '#'. '//' is integer division, like Python.
import stats, ab              # built-in modules
import "helpers.mpp"          # local file

const ALPHA = 0.05

fn lift(a, b) {
    return (b - a) / a
}

class Variant {
    fn init(self, name, rate) { self.name = name; self.rate = rate }
    fn better_than(self, other) { return self.rate > other.rate }
}

data = load_csv("checkout.csv")                  # native Table
a = data.filter(r => r.group == "A").col("converted")   # native Vec (f64)
b = data.filter(r => r.group == "B").col("converted")

for i in 0..3 { print(f"run {i}: lift = {lift(a.mean(), b.mean()):.2%}") }

if a.len() < 100 { warn("small sample") } elif a.len() < 1000 { print("ok") } else { print("big") }

try { x = risky() } catch e { print(e.message) }

experiment "new checkout button" {
    r = ab.proportions(a, b)
    expect r.p_value < ALPHA
    expect r.lift > 0.02
    report r                                       # goes into JSON/HTML report
}

test "lift math" { expect lift(0.10, 0.12) ~= 0.2 }      # ~= approx equal

property "lift sign" (x in gen.float(0.01, 1), y in gen.float(0.01, 1)) {
    expect (lift(x, y) > 0) == (y > x)
}

bench "bootstrap 10k" { stats.bootstrap(a, fn = mean, n = 10000) }
```

Rules:
- Newline ends a statement. `;` is optional. Blocks always use `{}`.
- Values: `nil`, `bool`, `int` (i64, overflow = runtime error), `float` (f64), `str` (UTF-8),
  `list`, `map` (keeps insertion order), `Vec` (fast f64 array, element-wise math), `Table`,
  functions/closures (`fn` and `x => expr`), classes with `init` and `self`.
- Words for logic: `and`, `or`, `not`, `in`. Power `**`. Ranges `a..b`, `a..=b`.
- First assignment creates a local in the current function. Closures capture by reference.
- Keyword args and default args: `fn f(a, n = 100)`, `f(x, n = 5)`.
- f-strings with format specs: `f"{x:.3f}"`, `f"{p:.1%}"`.
- Errors: `throw`, `try/catch`. Runtime errors print a stack trace with file:line:col.

## Architecture

One Rust crate `mpp` (lib + one binary). Modules, not many crates. Split only if a real need appears.

```
Cargo.toml  Cargo.lock  rust-toolchain.toml  .gitignore  .env.example  README.md
src/
  main.rs            CLI (clap): run, test, build, check, fmt, repl, lsp, new
  syntax/            lexer.rs, token.rs, parser.rs (Pratt), ast.rs, span.rs
  diag.rs            error reports with source snippets (codespan-reporting)
  compile/           resolver.rs (scopes, upvalues, undefined names), emit.rs (bytecode),
                     chunk.rs (opcodes + serialize to .mppc)
  vm/                vm.rs (stack VM, call frames), value.rs, object.rs (closures, classes),
                     error.rs (stack traces)
  runner/            test discovery + execution, expect failure diff, seeds, snapshots,
                     report/{term,json,junit,html}.rs
  std/               core.rs, str.rs, list.rs, map.rs, io.rs, json.rs, time.rs, rand.rs,
                     vec.rs, table.rs, stats/, ab.rs, bandit.rs, power.rs,
                     ml/, llm/ (connect.rs, metrics.rs, train_log.rs), perf.rs, gen.rs
  fmt.rs             formatter
  lsp.rs             language server
editors/vscode/      TextMate grammar + tiny LSP client
examples/            one runnable .mpp per kit
tests/               golden tests: tests/cases/*.mpp + *.out, plus stats fixtures
docs/                language reference + kit reference (markdown)
.github/workflows/ci.yml
```

Pipeline: source -> lexer -> parser (AST + spans) -> resolver -> bytecode emitter -> VM.
`mpp build x.mpp` writes `x.mppc` (bytecode). `mpp build --exe x.mpp` writes a standalone
executable: copy of the `mpp` binary with the bytecode appended + a magic trailer it checks on start.

Key design choices:
- Values use `Rc`. Ponytail note: reference cycles leak until exit; fine for short test runs.
- Speed: hot math never runs in bytecode. `Vec`, `Table`, stats, metrics are Rust code.
- Parallel work happens inside Rust only: `mpp test --jobs N` runs each test in its own VM on its
  own thread; `model.batch(prompts, concurrency = 8)` runs requests on Rust threads.
- Reproducible: one global seed (`--seed`, default printed). Each report stores seed, mpp version,
  and sha256 of every data file loaded.

Dependencies (pin `~X.Y`; check latest minor with `cargo search` at build time):
clap, codespan-reporting, statrs, rand, rand_pcg, serde, serde_json, csv, ureq, sha2,
rustyline, lsp-server, lsp-types. Dev only: none beyond std test harness.

## Testing kits (full list of what ships)

**Test runner (`mpp test`)**: `test`, `experiment`, `bench`, `property` blocks; `expect` with
both sides printed on failure; `~=` approx with `within`; `expect_throws`; snapshot tests
(`expect_snapshot(v)`, stored in `__snapshots__/`, `--update-snapshots`); `--filter`, `--jobs`,
`--seed`, `--retries`, timeouts; reports: terminal, JSON, JUnit XML (CI), single-file HTML
with inline SVG charts.

**Data**: `Vec` (element-wise ops, mean/median/quantile/std/var/skew/kurtosis, masks),
`Table` (load CSV/JSONL, filter, select, col, sort, group_by + agg, join, describe, head, save).

**Stats (`stats`)**: one-sample / Welch / Student / paired t-tests; z-test; chi-square
(independence, goodness of fit); Fisher exact; Mann-Whitney U; Wilcoxon signed-rank; KS 2-sample;
one-way ANOVA; Kruskal-Wallis; Levene; Shapiro-Wilk; Pearson / Spearman / Kendall correlation;
OLS regression with CIs; effect sizes (Cohen d, Hedges g, odds ratio, relative risk, Cliff delta);
bootstrap CI (percentile, BCa); permutation test; multiple-comparison fixes
(Bonferroni, Holm, Benjamini-Hochberg).

**A/B (`ab`)**: proportions and means tests with lift + CI; ratio metrics (delta method); CUPED;
sample ratio mismatch (SRM) check; A/A check; A/B/n with corrections; Bayesian A/B
(Beta-Binomial and Normal: P(B>A), expected loss, credible interval); sequential testing
(mSPRT always-valid p-values, O'Brien-Fleming group-sequential bounds); segment / stratified
analysis; novelty / time-trend check; guardrail metrics; power, sample size and MDE (`power`);
bandit simulation (epsilon-greedy, UCB1, Thompson).

**ML models (`ml`)**: classification (accuracy, precision, recall, F1 micro/macro/weighted,
confusion matrix, ROC-AUC, PR-AUC, log loss, Brier, MCC, Cohen kappa, top-k);
regression (MAE, MSE, RMSE, R2, MAPE, SMAPE, median AE); ranking (NDCG@k, MAP, MRR,
precision@k, recall@k, hit rate); calibration (ECE, MCE, reliability bins); drift (PSI, KS,
JS divergence, KL, Wasserstein, chi-square for categories); fairness (demographic parity,
equalized odds, equal opportunity, disparate impact, per-group metrics); model comparison
(McNemar, DeLong AUC, paired bootstrap on any metric, 5x2cv t-test); threshold sweep;
slice analysis; data checks (nulls, duplicates, schema, ranges, train/test leakage, label balance).

**Local LLMs (`llm`)**:
- Connect: `llm.http(url, mode = "openai" | "raw", headers = env(...))`,
  `llm.process("python infer.py")` (JSON lines over stdin/stdout). Same API on both:
  `generate`, `batch(concurrency)`, `logprobs`, streaming for time-to-first-token.
- Eval: exact match, contains, regex, JSON-valid + schema check, BLEU, ROUGE-L, chrF,
  edit-distance similarity, perplexity from logprobs, pass@k (runs a check function),
  determinism check (same seed same output), paraphrase consistency, refusal patterns,
  local-model-as-judge (your own model scores outputs with a rubric).
- Checkpoint compare: run one golden set on two checkpoints, paired bootstrap + sign test,
  list of regressions.
- Speed: latency p50/p95/p99, time to first token, tokens/sec, load test with concurrency.
- Training log debug (`llm.train_log("run.jsonl")`, CSV too): NaN/inf loss, loss spikes
  (rolling z-score), divergence, plateau, train/val overfit gap, grad-norm explode/vanish,
  LR schedule shape check (warmup, decay), throughput drops; compare two runs; plot to HTML report.

**Perf (`perf`)**: `bench` blocks with warmup, iterations, mean/stddev/p95, compare to saved
baseline and fail on regression.

## Build order (each phase ends runnable, tested, committed, shown to user)

0. **Setup**: `git init`, crate, `.gitignore`, `.env.example`, `rust-toolchain.toml`, CI workflow.
1. **Core language**: lexer, parser, resolver, emitter, VM, core std (print, len, str/list/map
   methods, math, io, json, time, env, rand), imports, f-strings, classes, closures, try/catch,
   `mpp run|check|build|repl`, error reports with source snippets. Golden tests.
2. **Test runner + reports**: the four blocks, `expect`, snapshots, jobs, seeds, all report formats.
3. **Data + stats + A/B + power + bandits.**
4. **ML model kit.**
5. **Local LLM kit** (connect, eval, checkpoint compare, speed, training logs).
6. **Tooling**: `mpp fmt`, `mpp lsp` (diagnostics, hover, completion, go-to-definition),
   VS Code extension, `docs/` reference, `examples/`.

Stop after each phase for the user to look. Copy this plan to `docs/design.md` in phase 0.

## Verification

- `cargo test`: unit tests per module + golden tests (`tests/cases/*.mpp` run, stdout compared
  to `*.out`; error cases compare the diagnostic text).
- Stats/ML numbers: build a throwaway venv in the scratchpad with scipy + scikit-learn, compute
  reference values once, save them as JSON fixtures in `tests/fixtures/`. Rust tests compare to
  them (tolerance 1e-9 or documented). scipy is dev-only, never shipped.
- LLM kit: Rust tests start a tiny mock HTTP server (std `TcpListener`) and a mock script
  (`tests/fixtures/mock_model.py`); real local server smoke test is manual (`examples/llm_local.mpp`).
- End to end: `cargo build --release && ./target/release/mpp test examples/` passes;
  `mpp build --exe examples/ab_checkout.mpp` makes a binary that runs alone.
- `cargo clippy -- -D warnings` and `cargo fmt --check` clean. CI runs all of the above.
- Parser crash test: random input fuzz loop in a test (no panics allowed, only diagnostics).
