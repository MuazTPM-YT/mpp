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

## Language in one screen

```
# comments use '#'; '//' is integer division
import math, json               # built-in modules: math time io json rand gen
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
