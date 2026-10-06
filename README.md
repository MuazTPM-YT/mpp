# Muaz++

A small, fast language for testing and analysis: A/B tests, statistics,
ML model checks and tests for your own local LLMs.
Files end in `.mpp`. Python feel, C++ braces, compiled to bytecode and run
by a Rust VM. Heavy math runs as native Rust.

## Install

```sh
cargo build --release
cp target/release/mpp ~/.local/bin/
```

## Use

```sh
mpp run file.mpp [-- args]      # compile + run
mpp check file.mpp              # find errors, do not run
mpp build file.mpp              # write file.mppc (bytecode)
mpp build --exe file.mpp        # write a standalone executable
mpp repl                        # interactive prompt
```

## Testing

Test code lives in blocks. `mpp run` skips them; `mpp test` runs them.

```
import gen

test "lift math" {
    expect lift(0.10, 0.12) ~= 0.2             # on failure shows both sides
    expect lift(1, 2) ~= 1.0 within 0.001      # absolute tolerance
    expect_throws(() => lift(0, 1), kind = "ZeroDivisionError")
}

experiment "checkout button" {                 # like test, but its reports always show
    report "conversion": {"a": 0.30, "b": 0.36}
    expect b > a
}

property "sort keeps length" (xs in gen.list(gen.int(-100, 100)), runs = 200) {
    expect len(sorted(xs)) == len(xs)          # failures shrink to the smallest input
}

test "flaky model call" (retries = 2, timeout = 5) { ... }
test "summary stays the same" { expect_snapshot(summary) }   # saved in __snapshots__/
test "later" { skip("no data yet") }

bench "bootstrap" (time = 1.0, warmup = 3) { ... }           # or n = 1000
```

Generators: `gen.int(lo, hi)`, `gen.float(lo, hi)`, `gen.bool()`, `gen.choice(xs)`,
`gen.str(min, max, chars)`, `gen.list(g, min, max)`, `gen.just(v)`, `gen.one_of(g1, g2)`.

```sh
mpp test                         # every .mpp with test blocks under this folder
mpp test tests/ -k checkout      # filter by name
mpp test --seed 42 -j 8 -x       # fixed seed, 8 files at once, stop at first failure
mpp test --report html:report.html --report junit:junit.xml --report json:run.json
mpp test --update-snapshots
mpp test --list
mpp bench --save-baseline base.json
mpp bench --baseline base.json --max-regress 10   # fail if 10% slower
```

Exit code: 0 all passed, 1 something failed, 5 nothing found.
Every run prints its seed; pass it back with `--seed` to repeat the exact run.

## Data, stats and A/B tests

```
data = load_csv("checkout.csv")          # also load_jsonl, load_json, table({...}) / table([rows])
data.group_by("variant", {"n": "count()", "rate": "mean(converted)"})
g = data.groups("variant")               # map of sub-tables: g.A, g.B
r = ab.proportions(g.A.converted, g.B.converted)
print(f"lift {r.lift:+.1%}, p = {r.p_value:.4f}")
```

**Tables**: `col`, `row`, `rows`, `head`, `tail`, `select`, `drop`, `rename`, `filter(fn)`,
`where(col, op, value)`, `t[mask]`, `sort`, `with_col`, `group_by(by, aggs)`, `groups`, `join`,
`describe`, `value_counts`, `unique`, `dropna`, `sample`, `shuffle`, `concat`, `save_csv`.
Aggregates: `count() sum mean median min max std var nunique first last`.
Missing cells (`""`, `NA`, `null`, ...) become `nil`. `true/false` become 1/0.

**Vectors** (`vec(xs)`, table number columns): `+ - * / ** %` element-wise, `< <= > >=` give 0/1 masks,
`v[mask]`; `sum mean median var std sem min max quantile skew kurtosis cumsum diff rank zscore
sorted unique dropna isnan clip between dot corr cov histogram describe sample map filter`.
`==` compares whole vectors.

**stats**: `mean median var std quantile iqr mad sem skew kurtosis zscore describe ci_mean`,
`ttest` (Welch; `equal_var = true` for Student), `ttest_1samp`, `ttest_rel`, `ztest`, `chi2`,
`chi2_gof`, `fisher`, `mannwhitney`, `wilcoxon`, `ks`, `anova`, `kruskal`, `levene`, `shapiro`,
`pearson`, `spearman`, `kendall`, `linregress`, `ols(table, "y ~ a + b")`, `cohens_d`, `hedges_g`,
`cliffs_delta`, `odds_ratio`, `relative_risk`, `bootstrap` (BCa), `bootstrap_diff`,
`permutation_test`, `adjust` (bonferroni, holm, hochberg, bh, by), and distributions
(`norm_cdf/ppf/pdf`, `t_cdf/ppf`, `chi2_cdf/ppf`, `f_sf`, `beta_cdf/ppf`, `binom_pmf/cdf`, `poisson_pmf/cdf`).
Every test returns a map: `statistic`, `p_value`, plus effect sizes and intervals.
Numbers are checked against scipy and statsmodels (`tests/stats_ref.rs`, `tests/ab_ref.rs`).

**ab**: `proportions`, `means`, `ratio` (delta method), `cuped`, `srm`, `aa`, `multi` (A/B/n with
correction), `bayes` (Beta-Binomial: P(B better), expected loss), `bayes_means`, `msprt`
(always-valid p-value, safe to peek), `sequential_bounds` (O'Brien-Fleming / Pocock spending),
`segments` (with Simpson's paradox warning), `novelty`, `guardrail` (non-inferiority).

**power**: `proportions`, `means`, `mde_proportions`, `mde_means`, `achieved_proportions`,
`achieved_means`, `duration`.

**bandit**: `thompson(k)`, `ucb1(k)`, `epsilon_greedy(k)` objects with `choose`, `update`, `stats`,
`prob_best`; and `simulate(arms, policy, steps)` for regret.

See `examples/ab_checkout.mpp` for a full A/B analysis.

## ML model tests (`ml`)

Labels can be numbers, strings or bools. Scores are a list/vec (binary) or rows (one column per class).

- **classification**: `accuracy`, `balanced_accuracy`, `precision`, `recall`, `f1`, `fbeta`
  (`average = "binary" | "macro" | "micro" | "weighted" | "none"`), `confusion_matrix`,
  `classification_report`, `roc_auc` (binary or one-vs-rest), `roc_curve`, `pr_curve`,
  `average_precision` / `pr_auc`, `log_loss`, `brier`, `mcc`, `cohen_kappa`, `top_k_accuracy`
- **regression**: `regression` (all at once), `mae`, `mse`, `rmse`, `r2`, `mape`
- **ranking** (relevance in ranked order, one list per query): `ndcg`, `map_at_k`, `mrr`,
  `precision_at_k`, `recall_at_k`, `hit_rate`
- **calibration**: `calibration(y, probs, bins, strategy)` gives ECE, MCE and a bin table
- **drift**: `psi`, `wasserstein`, `kl`, `js`, `drift(ref_table, cur_table)` per column
- **fairness**: `fairness(y, pred, group)`: selection rate, TPR, FPR per group, demographic
  parity, disparate impact (80% rule), equal opportunity, equalized odds
- **compare models**: `delong` (two AUCs), `mcnemar`, `paired_bootstrap` (any metric), `cv5x2`
- **tuning and slices**: `threshold_sweep`, `slices(table, y, pred, by)` (worst slice first)
- **data checks**: `check` (missing, duplicates, constant columns, ranges), `schema`,
  `leakage(train, test)`, `label_balance`

Metric numbers are checked against scikit-learn (`tests/ml_ref.rs`).
See `examples/model_eval.mpp`.

## Local LLM tests (`llm`)

Connect to your own model:

```
model = llm.http("http://localhost:8080", temperature = 0)          # OpenAI-style server: llama.cpp, vLLM, Ollama, LM Studio...
model = llm.http(url, api = "completions")                          # completions API (needed for perplexity)
model = llm.http(url, mode = "raw", field = "output.text")          # your own JSON server: {"prompt": ...} in, text at `field` out
model = llm.process("python infer.py")                              # a script: one JSON line in, one JSON line out ({"text": ...})
model = llm.mock((prompt, params) => "...")                         # fake model for wiring tests
```

Model methods: `ask` (text), `generate` (text, tokens, latency, logprobs), `chat(messages)`,
`stream` (time to first token), `batch(prompts, concurrency = 8)`, `logprobs`, `perplexity`.
Any extra named argument (`max_tokens`, `temperature`, `seed`, `stop`, ...) goes to the model.
Tokens come from `.env`: `headers = {"Authorization": "Bearer " + env("MODEL_TOKEN")}`.

Evaluation:

- `eval_set(model, cases, metric)`: golden set (list of `{"prompt", "expected"}` or a table);
  metrics `exact contains regex json json_schema token_f1 bleu rouge_l chrf similarity refusal`
  or your own `fn(output, expected)`
- `compare_checkpoints(a, b, cases)`: paired bootstrap CI, sign test, verdict, list of regressions
- `judge(model, answer, rubric)`: your own model grades an answer
- `determinism`, `consistency` (paraphrases), `load_test` (p50/p95/p99, TTFT, tokens/s, errors)
- text metrics: `exact_match`, `token_f1`, `bleu`, `rouge`, `rouge_l`, `rouge_n`, `chrf`,
  `similarity`, `edit_distance`, `contains`, `regex_match`, `extract`, `json_valid`, `parse_json`,
  `json_schema`, `refusal`, `pass_at_k`, `perplexity_of` (checked against nltk, rouge-score, sacrebleu)

Training logs (JSONL or CSV, `NaN` and `Infinity` understood):

```
log = llm.train_log("runs/a.jsonl")
r = log.check()          # NaN/inf, loss spikes, divergence, plateau, overfitting,
                         # grad explode/vanish, LR schedule, throughput drops
print(r.issues)          # table: severity, kind, step, message
log.compare(other_log)   # which run is lower over the shared steps
report "loss": log.plot(["loss", "val_loss"])   # chart, drawn in the HTML report
```

Examples: `examples/llm_eval_test.mpp` (runs anywhere), `examples/llm_local.mpp` (set `MODEL_URL`),
`examples/train_debug.mpp`.

## Language in one screen

```
# comments use '#'; '//' is integer division
import math, json               # built-in modules: math time io json rand gen stats ab power bandit ml llm
import "lib/helpers.mpp" as h   # your own files

const ALPHA = 0.05              # cannot change

fn lift(a, b, pct = true) {     # default args, called fresh each time
    d = (b - a) / a
    return pct ? d * 100 : d
}
lift(0.1, 0.12, pct = false)    # keyword args

class Dog : Animal {            # single inheritance
    fn init(self, name) { super.init(name) }
    fn to_str(self) { return f"<Dog {self.name}>" }   # used by print
}

xs = [3, 1, 2]                  # list
m = {"a": 1, "b": 2}            # map (keeps insert order); m.a works too
for i in 0..3 { }               # 0,1,2      (0..=3 includes 3)
for k, v in m.items() { }       # unpack
while x < 10 { x += 1 }
if a { } elif b { } else { }
y = cond ? 1 : 2
sq = x => x * x                 # lambda; (a, b) => a + b ; fn(a) { ... }
data.filter(r => r.ok)          # a line starting with `.` continues the last one
    .map(r => r.value)
try { risky() } catch e { print(e.kind, e.message) }
throw "bad"                     # or throw error("msg", kind = "ValueError")
f"{x:.2f} {p:.1%} {n:,} {s:>8}" # python format specs
a ~= b                          # approx equal (rel 1e-6, abs 1e-12)
```

Rules worth knowing:

- Newline ends a statement; `;` is optional. Inside `( )` and `[ ]` newlines are free.
- Assigning a name inside a function makes it local. Use `global x` / `nonlocal x` to change outer ones.
- Closures see variables, not copies (like Python).
- `/` always gives a float. Ints are 64-bit and overflow is an error, not a wrap.
- Unknown names are compile errors, with "did you mean" help.

## Layout

```
src/syntax    lexer, parser, AST
src/compile   name resolution + bytecode emitter, .mppc format
src/vm        stack VM, values, operators
src/stdlib    built-in functions and modules
tests/cases   golden tests: *.mpp with expected *.out
docs/design.md  full design and roadmap
```

## Develop

```sh
cargo test                      # unit, golden and crash tests
MPP_BLESS=1 cargo test --test golden   # rewrite expected outputs (review the diff!)
cargo clippy --all-targets -- -D warnings
```
