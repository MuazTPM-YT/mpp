# Muaz++ language reference

Muaz++ files end in `.mpp`. The `mpp` compiler turns them into bytecode for a
Rust virtual machine. Heavy numeric work (vectors, tables, statistics, metrics)
runs as native Rust.

## Lexical rules

- Comments start with `#` and run to the end of the line.
- A newline ends a statement. `;` also ends one, so `a = 1; b = 2` is fine.
- Inside `( )` and `[ ]`, newlines are ignored, so long calls and lists can span lines.
- A line that starts with `.` continues the previous line (method chains).
- Names: letters, digits and `_`, not starting with a digit. Unicode letters are allowed.
- Blocks always use `{ }`. Indentation has no meaning; `mpp fmt` keeps it tidy.

### Literals

| Kind | Examples |
|---|---|
| int (64-bit) | `42`, `1_000_000`, `0xff`, `0b1010`, `0o17` |
| float | `3.14`, `1e-6`, `2.5E3` |
| bool, nil | `true`, `false`, `nil` |
| string | `"hi"`, `'hi'`, `"""many\nlines"""` |
| raw string | `r"\d+"` (no escapes) |
| f-string | `f"{name} has {score:.1%}"` |
| list | `[1, 2, 3]` |
| map | `{"a": 1, 2: "two"}` (keys: str, int, bool, nil; insertion order kept) |
| range | `0..5` (0 to 4), `1..=5` (1 to 5) |

Escapes: `\n \t \r \0 \\ \" \' \{ \} \u{1F600}`. In f-strings write `{{` and `}}` for braces.

### f-string format specs

`{value:spec}` with spec `[[fill]align][sign][0][width][,][.precision][type]`:

| Spec | Result |
|---|---|
| `{x:.2f}` / `{x:.2}` | `3.14` (fixed decimals) |
| `{p:.1%}` | `12.3%` |
| `{n:,}` | `1,234,567` |
| `{x:e}` / `{x:g}` | scientific / general |
| `{n:05d}` | `00042` |
| `{n:x}` `{n:b}` `{n:o}` | hex, binary, octal |
| `{s:>10}` `{s:<10}` `{s:^10}` `{s:*^10}` | align with fill |
| `{x:+.1f}` | always show the sign |

## Values and types

`nil bool int float str list map range function class instance module error`,
plus native objects: `vec`, `table`, `generator`, `bandit`, `model`, `train_log`, `chart`.

- `type(x)` gives the name; for instances it gives the class name.
- Truthiness: `nil`, `false`, `0`, `0.0`, `""`, `[]`, `{}` and empty ranges are false.
- `==` compares by value (lists and maps deeply). `1 == 1.0` is true.
- Ints never wrap: overflow is an `OverflowError`.

## Operators (lowest to highest precedence)

| Operators | Notes |
|---|---|
| `c ? a : b` | ternary, right-associative |
| `or` | returns the first truthy operand |
| `and` | returns the first falsy operand |
| `not` | |
| `== != < <= > >= in not in ~=` | no chaining (`a < b < c` is an error) |
| `..` `..=` | ranges |
| `+ -` | `+` joins strings and lists |
| `* / // %` | `/` always gives a float; `//` floors; `%` follows the divisor's sign |
| unary `-` `+` | |
| `**` | right-associative; `-2 ** 2` is `-4` |
| `f(x)` `a[i]` `a[i:j]` `a.b` | call, index, slice, field |

`a ~= b` is approximately equal: relative 1e-6 or absolute 1e-12 (lists compare item by item).
Negative indexes count from the end. Slices clamp like Python.

## Statements

```
x = 1                    # assign (first assignment in a function makes a local)
a, b = b, a              # unpack a list
x += 1                   # += -= *= /= %=
const LIMIT = 10         # cannot be assigned again

if a { } elif b { } else { }
while cond { }
for x in items { }       # lists, ranges, strings (chars), maps (keys), vecs, tables (rows)
for k, v in m.items() { }
break; continue

fn name(a, b = 2) { return a + b }    # defaults are evaluated on every call
f = x => x * 2                         # lambda; (a, b) => a + b ; x => { ... }
g = fn(a) { return a }                 # anonymous function

class Point : Base {                   # single inheritance
    fn init(self, x) { self.x = x }    # constructor; `self` is explicit
    fn to_str(self) { return f"P({self.x})" }   # used by print and f-strings
}
p = Point(3)                           # call the class to make an instance

try { risky() } catch e { print(e.kind, e.message, e.trace) }
throw "message"                        # or throw error("message", kind = "ValueError")

import math, stats as s                # built-in modules
import "lib/helpers.mpp"               # file module; bound as `helpers`
import "lib/helpers.mpp" as h
```

### Calls

Positional arguments come first, then named ones: `f(1, 2, scale = 3)`.
Unknown or repeated names are errors. Missing arguments are errors.

### Scope

- Top-level names are module globals. A function sees them, but assigning a name
  inside a function makes a local; write `global x` to change the global.
- Nested functions see outer variables. Write `nonlocal x` to assign one.
- Closures capture variables (not copies), like Python.
- Using a name before it gets a value is a `NameError`; a name that is never
  defined anywhere is a compile error with a "did you mean" hint.

### Errors

Runtime errors are values with `kind`, `message` and `trace`. Kinds include
`TypeError`, `ValueError`, `IndexError`, `KeyError`, `ZeroDivisionError`,
`OverflowError`, `AttributeError`, `NameError`, `IOError`, `ModelError`,
`TimeoutError`, `RecursionError`, `AssertionError`, `ExpectFailed`.
`exit()` cannot be caught.

## Test blocks

Only at the top level of a file. `mpp run` skips them; `mpp test` runs them.

```
test "name" (retries = 1, timeout = 5) { ... }
experiment "name" { report "label": value }
property "name" (x in gen.int(0, 9), xs in gen.list(gen.float()), runs = 200) { ... }
bench "name" (n = 1000) { ... }        # or time = 1.0, warmup = 3; run with `mpp bench`
```

Inside any code:

```
expect cond                    # fails with the value shown
expect a == b                  # fails with left and right shown (also != < <= > >= in, not in)
expect a ~= b within 0.01      # absolute tolerance
report value                   # label is the source text
report "label": value
```

Helpers: `expect_throws(fn, kind, contains)`, `expect_snapshot(value, name)`,
`skip(reason)`, `note(text)`.

Each block gets its own seed derived from the run seed and the block name, so
results do not depend on run order. `mpp test --seed N` repeats a run exactly.

## Programs

| Command | What it does |
|---|---|
| `mpp run file.mpp [-- args]` | compile and run (`.mppc` files too) |
| `mpp check [--short] files` | compile only, report errors |
| `mpp build file.mpp [-o out]` | write bytecode `.mppc` |
| `mpp build --exe file.mpp` | standalone executable (bytecode glued to the runtime) |
| `mpp test` / `mpp bench` | run blocks, write reports |
| `mpp fmt [--check] [paths]` | format files (`-` = stdin to stdout) |
| `mpp doc NAME` | docs for a built-in (`mpp doc --markdown` for all) |
| `mpp new NAME` | new project folder |
| `mpp repl` | interactive prompt |
| `mpp lsp` | language server for editors |

`.env` in the current folder (and next to the script) is loaded at start;
real environment variables win.
