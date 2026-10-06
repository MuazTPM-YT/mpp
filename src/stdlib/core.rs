use super::{list::sort_values, to_vec};
use crate::compile::chunk::Op;
use crate::vm::ops::binary;
use crate::vm::*;
use std::io::{BufRead, Write};
use std::rc::Rc;

macro_rules! natives {
    ($($name:literal => $f:expr),* $(,)?) => {
        &[$(Native { name: $name, f: $f }),*]
    };
}

pub static BUILTINS: &[Native] = natives![
    "print" => print,
    "len" => len,
    "type" => type_of,
    "str" => to_str,
    "repr" => repr,
    "int" => to_int,
    "float" => to_float,
    "bool" => to_bool,
    "list" => to_list,
    "range" => range,
    "abs" => abs,
    "min" => min,
    "max" => max,
    "sum" => sum,
    "round" => round,
    "sorted" => sorted,
    "reversed" => reversed,
    "enumerate" => enumerate,
    "zip" => zip,
    "any" => any,
    "all" => all,
    "input" => input,
    "error" => error,
    "assert" => assert,
    "env" => env,
    "args" => args,
    "exit" => exit,
    "warn" => warn,
    "chr" => chr,
    "ord" => ord,
    "copy" => copy,
    "isinstance" => isinstance,
    "callable" => callable,
    "skip" => super::testing::skip,
    "expect_throws" => super::testing::expect_throws,
    "expect_snapshot" => super::testing::expect_snapshot,
    "note" => super::testing::note,
];

fn print(vm: &mut Vm, a: Args) -> R {
    let (mut sep, mut end) = (" ".to_string(), "\n".to_string());
    for (k, v) in &a.kw {
        match &**k {
            "sep" => sep = v.as_str("sep")?.to_string(),
            "end" => end = v.as_str("end")?.to_string(),
            _ => return Err(type_err(format!("unknown argument `{k}` (takes sep, end)"))),
        }
    }
    let mut s = String::new();
    for (i, v) in a.pos.iter().enumerate() {
        if i > 0 {
            s.push_str(&sep);
        }
        s.push_str(&vm.display(v, false)?);
    }
    s.push_str(&end);
    vm.out.write_all(s.as_bytes()).map_err(|e| err("IOError", e.to_string()))?;
    Ok(Value::Nil)
}

pub fn length(v: &Value) -> Result<i64, Flow> {
    Ok(match v {
        Value::Str(s) => s.chars().count() as i64,
        Value::List(l) => l.borrow().len() as i64,
        Value::Map(m) => m.borrow().len() as i64,
        Value::Range(a, b) => (b - a).max(0),
        Value::Instance(i) => i.fields.borrow().len() as i64,
        other => return Err(type_err(format!("{} has no length", other.kind_name()))),
    })
}

fn len(_: &mut Vm, a: Args) -> R {
    let [x] = a.bind(["x"])?;
    Ok(Value::Int(length(&need(x, "x")?)?))
}

fn type_of(_: &mut Vm, a: Args) -> R {
    let [x] = a.bind(["x"])?;
    Ok(Value::str(need(x, "x")?.kind_name()))
}

fn to_str(vm: &mut Vm, a: Args) -> R {
    let [x] = a.bind(["x"])?;
    let s = vm.display(&x.unwrap_or(Value::str("")), false)?;
    Ok(Value::str(s))
}

fn repr(vm: &mut Vm, a: Args) -> R {
    let [x] = a.bind(["x"])?;
    let s = vm.display(&need(x, "x")?, true)?;
    Ok(Value::str(s))
}

fn to_int(_: &mut Vm, a: Args) -> R {
    let [x, base] = a.bind(["x", "base"])?;
    let x = need(x, "x")?;
    Ok(Value::Int(match &x {
        Value::Int(n) => *n,
        Value::Bool(b) => *b as i64,
        Value::Float(f) => {
            if !f.is_finite() || f.abs() >= 9.2e18 {
                return Err(value_err(format!("cannot turn {} into int", fmt_float(*f))));
            }
            f.trunc() as i64
        }
        Value::Str(s) => {
            let radix = match base {
                Some(b) => b.int("base")? as u32,
                None => 10,
            };
            let t = s.trim().replace('_', "");
            let (neg, digits) = match t.strip_prefix('-') {
                Some(d) => (true, d.to_string()),
                None => (false, t.trim_start_matches('+').to_string()),
            };
            let n = i64::from_str_radix(&digits, radix).map_err(|_| value_err(format!("not an int: {:?}", &**s)))?;
            if neg { -n } else { n }
        }
        other => {
            return Err(type_err(format!("cannot turn {} into int", other.kind_name())));
        }
    }))
}

fn to_float(_: &mut Vm, a: Args) -> R {
    let [x] = a.bind(["x"])?;
    let x = need(x, "x")?;
    Ok(Value::Float(match &x {
        Value::Str(s) => s.trim().parse::<f64>().map_err(|_| value_err(format!("not a number: {:?}", &**s)))?,
        other => other.num("x")?,
    }))
}

fn to_bool(_: &mut Vm, a: Args) -> R {
    let [x] = a.bind(["x"])?;
    Ok(Value::Bool(x.is_some_and(|v| v.truthy())))
}

fn to_list(_: &mut Vm, a: Args) -> R {
    let [x] = a.bind(["x"])?;
    match x {
        Some(v) => Ok(Value::list(to_vec(&v, "x")?)),
        None => Ok(Value::list(vec![])),
    }
}

fn range(_: &mut Vm, a: Args) -> R {
    let [x, y, step] = a.bind(["start", "end", "step"])?;
    let (lo, hi) = match (x, y) {
        (Some(e), None) => (0, e.int("end")?),
        (Some(s), Some(e)) => (s.int("start")?, e.int("end")?),
        _ => return Err(type_err("range() needs an end")),
    };
    let step = match step {
        Some(s) => s.int("step")?,
        None => 1,
    };
    if step == 1 {
        return Ok(Value::Range(lo, hi));
    }
    if step == 0 {
        return Err(value_err("step cannot be 0"));
    }
    let mut out = Vec::new();
    let mut i = lo;
    while (step > 0 && i < hi) || (step < 0 && i > hi) {
        out.push(Value::Int(i));
        i += step;
    }
    Ok(Value::list(out))
}

fn abs(_: &mut Vm, a: Args) -> R {
    let [x] = a.bind(["x"])?;
    match need(x, "x")? {
        Value::Int(n) => n.checked_abs().map(Value::Int).ok_or_else(|| err("OverflowError", "integer overflow")),
        v => Ok(Value::Float(v.num("x")?.abs())),
    }
}

// values for min/max: many args, or one iterable
fn spread(a: &Args) -> Result<Vec<Value>, Flow> {
    if a.pos.len() == 1 { to_vec(&a.pos[0], "argument") } else { Ok(a.pos.clone()) }
}

fn extreme(vm: &mut Vm, a: Args, want: std::cmp::Ordering, fname: &str) -> R {
    let mut key = None;
    let mut dflt = None;
    for (k, v) in &a.kw {
        match &**k {
            "key" => key = Some(v.clone()),
            "default" => dflt = Some(v.clone()),
            _ => {
                return Err(type_err(format!("unknown argument `{k}` (takes key, default)")));
            }
        }
    }
    let items = spread(&a)?;
    let mut best: Option<(Value, Value)> = None;
    for x in items {
        let k = match &key {
            Some(f) => vm.call(f, std::slice::from_ref(&x))?,
            None => x.clone(),
        };
        let better = match &best {
            None => true,
            Some((bk, _)) => compare(&k, bk)? == want,
        };
        if better {
            best = Some((k, x));
        }
    }
    match (best, dflt) {
        (Some((_, x)), _) => Ok(x),
        (None, Some(d)) => Ok(d),
        (None, None) => Err(value_err(format!("{fname}() of empty list"))),
    }
}

fn min(vm: &mut Vm, a: Args) -> R {
    extreme(vm, a, std::cmp::Ordering::Less, "min")
}

fn max(vm: &mut Vm, a: Args) -> R {
    extreme(vm, a, std::cmp::Ordering::Greater, "max")
}

fn sum(_: &mut Vm, a: Args) -> R {
    let [xs, start] = a.bind(["xs", "start"])?;
    let items = to_vec(&need(xs, "xs")?, "xs")?;
    // all floats: compensated sum, so 0.1 * 10 adds to 1.0
    if start.is_none()
        && !items.is_empty()
        && items.iter().all(|x| matches!(x, Value::Float(_) | Value::Int(_)))
        && items.iter().any(|x| matches!(x, Value::Float(_)))
    {
        let mut s = 0.0f64;
        let mut c = 0.0f64;
        for x in &items {
            let v = x.num("")?;
            let t = s + v;
            c += if s.abs() >= v.abs() { (s - t) + v } else { (v - t) + s };
            s = t;
        }
        return Ok(Value::Float(s + c));
    }
    let mut acc = start.unwrap_or(Value::Int(0));
    for x in &items {
        acc = binary(Op::Add, &acc, x)?;
    }
    Ok(acc)
}

fn round(_: &mut Vm, a: Args) -> R {
    let [x, nd] = a.bind(["x", "ndigits"])?;
    let x = need(x, "x")?;
    if let Value::Int(n) = x {
        return Ok(Value::Int(n));
    }
    let f = x.num("x")?;
    match opt(nd) {
        None => {
            if !f.is_finite() {
                return Err(value_err("cannot round inf or nan to int"));
            }
            Ok(Value::Int(f.round_ties_even() as i64))
        }
        Some(n) => {
            let p = 10f64.powi(n.int("ndigits")? as i32);
            Ok(Value::Float((f * p).round_ties_even() / p))
        }
    }
}

fn sorted(vm: &mut Vm, a: Args) -> R {
    let [xs, key, rev] = a.bind(["xs", "key", "reverse"])?;
    let mut items = to_vec(&need(xs, "xs")?, "xs")?;
    sort_values(vm, &mut items, opt(key), rev.is_some_and(|r| r.truthy()))?;
    Ok(Value::list(items))
}

fn reversed(_: &mut Vm, a: Args) -> R {
    let [xs] = a.bind(["xs"])?;
    let mut items = to_vec(&need(xs, "xs")?, "xs")?;
    items.reverse();
    Ok(Value::list(items))
}

fn enumerate(_: &mut Vm, a: Args) -> R {
    let [xs, start] = a.bind(["xs", "start"])?;
    let s = match start {
        Some(v) => v.int("start")?,
        None => 0,
    };
    let items = to_vec(&need(xs, "xs")?, "xs")?;
    Ok(Value::list(items.into_iter().enumerate().map(|(i, x)| Value::list(vec![Value::Int(s + i as i64), x])).collect()))
}

fn zip(_: &mut Vm, a: Args) -> R {
    a.no_kw()?;
    let lists: Vec<Vec<Value>> = a.pos.iter().map(|v| to_vec(v, "zip argument")).collect::<Result<_, _>>()?;
    let n = lists.iter().map(Vec::len).min().unwrap_or(0);
    Ok(Value::list((0..n).map(|i| Value::list(lists.iter().map(|l| l[i].clone()).collect())).collect()))
}

fn any(_: &mut Vm, a: Args) -> R {
    let [xs] = a.bind(["xs"])?;
    Ok(Value::Bool(to_vec(&need(xs, "xs")?, "xs")?.iter().any(Value::truthy)))
}

fn all(_: &mut Vm, a: Args) -> R {
    let [xs] = a.bind(["xs"])?;
    Ok(Value::Bool(to_vec(&need(xs, "xs")?, "xs")?.iter().all(Value::truthy)))
}

fn input(vm: &mut Vm, a: Args) -> R {
    let [prompt] = a.bind(["prompt"])?;
    if let Some(p) = prompt {
        let s = vm.display(&p, false)?;
        let _ = vm.out.write_all(s.as_bytes());
    }
    let _ = vm.out.flush();
    let mut line = String::new();
    match std::io::stdin().lock().read_line(&mut line) {
        Ok(0) => Ok(Value::Nil),
        Ok(_) => Ok(Value::str(line.trim_end_matches(['\n', '\r']))),
        Err(e) => Err(err("IOError", e.to_string())),
    }
}

fn error(_: &mut Vm, a: Args) -> R {
    let [msg, kind] = a.bind(["message", "kind"])?;
    let msg = need(msg, "message")?;
    let kind = match kind {
        Some(k) => k.as_str("kind")?.to_string(),
        None => "Error".into(),
    };
    match err(&kind, &**msg.as_str("message")?) {
        Flow::Throw(e) => Ok(e),
        _ => unreachable!(),
    }
}

fn assert(vm: &mut Vm, a: Args) -> R {
    let [cond, msg] = a.bind(["cond", "message"])?;
    if need(cond, "cond")?.truthy() {
        return Ok(Value::Nil);
    }
    let m = match msg {
        Some(m) => vm.display(&m, false)?,
        None => "assertion failed".into(),
    };
    Err(err("AssertionError", m))
}

fn env(_: &mut Vm, a: Args) -> R {
    let [name, dflt] = a.bind(["name", "default"])?;
    let n = need(name, "name")?;
    match std::env::var(&**n.as_str("name")?) {
        Ok(v) => Ok(Value::str(v)),
        Err(_) => Ok(dflt.unwrap_or(Value::Nil)),
    }
}

fn args(vm: &mut Vm, a: Args) -> R {
    a.bind([])?;
    Ok(Value::list(vm.argv.iter().map(Value::str).collect()))
}

fn exit(vm: &mut Vm, a: Args) -> R {
    let [code] = a.bind(["code"])?;
    let _ = vm.out.flush();
    Err(Flow::Exit(code.map_or(Ok(0), |c| c.int("code"))? as i32))
}

fn warn(vm: &mut Vm, a: Args) -> R {
    a.no_kw()?;
    let parts: Vec<String> = a.pos.iter().map(|v| vm.display(v, false)).collect::<Result<_, _>>()?;
    let _ = vm.out.flush();
    eprintln!("warning: {}", parts.join(" "));
    Ok(Value::Nil)
}

fn chr(_: &mut Vm, a: Args) -> R {
    let [n] = a.bind(["code"])?;
    let n = need(n, "code")?.int("code")?;
    let c = u32::try_from(n).ok().and_then(char::from_u32).ok_or_else(|| value_err(format!("{n} is not a valid character code")))?;
    Ok(Value::str(c.to_string()))
}

fn ord(_: &mut Vm, a: Args) -> R {
    let [s] = a.bind(["ch"])?;
    let s = need(s, "ch")?;
    let s = s.as_str("ch")?;
    let mut it = s.chars();
    match (it.next(), it.next()) {
        (Some(c), None) => Ok(Value::Int(c as i64)),
        _ => Err(value_err("ord() needs exactly one character")),
    }
}

fn copy(_: &mut Vm, a: Args) -> R {
    let [x] = a.bind(["x"])?;
    Ok(match need(x, "x")? {
        Value::List(l) => Value::list(l.borrow().clone()),
        Value::Map(m) => Value::map(m.borrow().clone()),
        Value::Instance(i) => {
            Value::Instance(Rc::new(Instance { class: i.class.clone(), fields: std::cell::RefCell::new(i.fields.borrow().clone()) }))
        }
        other => other,
    })
}

fn isinstance(_: &mut Vm, a: Args) -> R {
    let [x, t] = a.bind(["x", "type"])?;
    let (x, t) = (need(x, "x")?, need(t, "type")?);
    Ok(Value::Bool(match (&x, &t) {
        (Value::Instance(i), Value::Class(c)) => {
            let mut cur = Some(i.class.clone());
            let mut hit = false;
            while let Some(k) = cur {
                if Rc::ptr_eq(&k, c) {
                    hit = true;
                    break;
                }
                cur = k.sup.clone();
            }
            hit
        }
        (_, Value::Str(name)) => x.type_name() == &**name || x.kind_name() == **name,
        _ => false,
    }))
}

fn callable(_: &mut Vm, a: Args) -> R {
    let [x] = a.bind(["x"])?;
    Ok(Value::Bool(need(x, "x")?.is_callable()))
}

// read KEY=VALUE lines from .env; real env wins
pub fn load_dotenv(path: &std::path::Path) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let k = k.trim();
        let v = v.trim();
        let v = v
            .strip_prefix('"')
            .and_then(|x| x.strip_suffix('"'))
            .or_else(|| v.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')))
            .unwrap_or(v);
        if std::env::var_os(k).is_none() {
            // SAFETY: called once at startup before any threads exist.
            unsafe { std::env::set_var(k, v) };
        }
    }
}
