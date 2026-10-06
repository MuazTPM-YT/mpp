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

## Language in one screen

```
# comments use '#'; '//' is integer division
import math, json               # built-in modules: math time io json rand
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
