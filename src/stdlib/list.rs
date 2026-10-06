use super::to_vec;
use crate::vm::ops::norm_index;
use crate::vm::*;
use indexmap::IndexMap;
use std::cmp::Ordering;

pub const METHODS: &[&str] = &[
    "len",
    "append",
    "push",
    "pop",
    "insert",
    "remove",
    "extend",
    "index",
    "count",
    "sort",
    "sorted",
    "reverse",
    "reversed",
    "map",
    "filter",
    "reduce",
    "each",
    "find",
    "any",
    "all",
    "sum",
    "min",
    "max",
    "copy",
    "clear",
    "join",
    "first",
    "last",
    "is_empty",
    "unique",
    "flatten",
    "zip",
    "enumerate",
    "take",
    "skip",
    "chunk",
    "group_by",
    "count_by",
    "contains",
];

// sort with optional key fn; errors on mixed types
pub fn sort_values(vm: &mut Vm, items: &mut Vec<Value>, key: Option<Value>, reverse: bool) -> Result<(), Flow> {
    let keys: Vec<Value> = match &key {
        Some(f) => items.iter().map(|x| vm.call(f, std::slice::from_ref(x))).collect::<Result<_, _>>()?,
        None => items.clone(),
    };
    let mut idx: Vec<usize> = (0..items.len()).collect();
    let mut bad = None;
    // stable both ways: ties keep their order
    idx.sort_by(|&a, &b| match compare(&keys[a], &keys[b]) {
        Ok(o) if reverse => o.reverse(),
        Ok(o) => o,
        Err(e) => {
            bad.get_or_insert(e);
            Ordering::Equal
        }
    });
    if let Some(e) = bad {
        return Err(e);
    }
    let old = std::mem::take(items);
    *items = idx.into_iter().map(|i| old[i].clone()).collect();
    Ok(())
}

fn f_arg(v: Option<Value>) -> Result<Value, Flow> {
    let f = need(v, "fn")?;
    if !f.is_callable() {
        return Err(type_err(format!("expected a function, got {}", f.kind_name())));
    }
    Ok(f)
}

pub fn call(vm: &mut Vm, l: &ListRef, name: &str, a: Args) -> R {
    match name {
        "len" => {
            a.bind([])?;
            Ok(Value::Int(l.borrow().len() as i64))
        }
        "append" | "push" => {
            let [x] = a.bind(["x"])?;
            l.borrow_mut().push(need(x, "x")?);
            Ok(Value::Nil)
        }
        "pop" => {
            let [i] = a.bind(["index"])?;
            let mut l = l.borrow_mut();
            let len = l.len();
            if len == 0 {
                return Err(err("IndexError", "pop from empty list"));
            }
            let i = match i {
                Some(v) => {
                    let n = v.int("index")?;
                    norm_index(n, len).ok_or_else(|| err("IndexError", format!("index {n} out of range (length {len})")))?
                }
                None => len - 1,
            };
            Ok(l.remove(i))
        }
        "insert" => {
            let [i, x] = a.bind(["index", "x"])?;
            let mut l = l.borrow_mut();
            let n = need(i, "index")?.int("index")?;
            let len = l.len() as i64;
            let at = if n < 0 { (n + len).max(0) } else { n.min(len) };
            l.insert(at as usize, need(x, "x")?);
            Ok(Value::Nil)
        }
        "remove" => {
            let [x] = a.bind(["x"])?;
            let x = need(x, "x")?;
            let mut l = l.borrow_mut();
            match l.iter().position(|y| equal(y, &x)) {
                Some(i) => {
                    l.remove(i);
                    Ok(Value::Nil)
                }
                None => Err(value_err(format!("{x:?} not in list"))),
            }
        }
        "extend" => {
            let [xs] = a.bind(["xs"])?;
            let items = to_vec(&need(xs, "xs")?, "xs")?;
            l.borrow_mut().extend(items);
            Ok(Value::Nil)
        }
        "index" | "count" | "contains" => {
            let [x] = a.bind(["x"])?;
            let x = need(x, "x")?;
            let l = l.borrow();
            Ok(match name {
                "index" => Value::Int(l.iter().position(|y| equal(y, &x)).ok_or_else(|| value_err(format!("{x:?} not in list")))? as i64),
                "count" => Value::Int(l.iter().filter(|y| equal(y, &x)).count() as i64),
                _ => Value::Bool(l.iter().any(|y| equal(y, &x))),
            })
        }
        "sort" | "sorted" => {
            let [key, rev] = a.bind(["key", "reverse"])?;
            let mut items = l.borrow().clone();
            sort_values(vm, &mut items, opt(key), rev.is_some_and(|r| r.truthy()))?;
            if name == "sort" {
                *l.borrow_mut() = items;
                Ok(Value::Nil)
            } else {
                Ok(Value::list(items))
            }
        }
        "reverse" => {
            a.bind([])?;
            l.borrow_mut().reverse();
            Ok(Value::Nil)
        }
        "reversed" => {
            a.bind([])?;
            let mut v = l.borrow().clone();
            v.reverse();
            Ok(Value::list(v))
        }
        "map" | "filter" | "each" | "find" | "group_by" | "count_by" => {
            let [f] = a.bind(["fn"])?;
            let f = f_arg(f)?;
            let items = l.borrow().clone();
            match name {
                "map" => {
                    let mut out = Vec::with_capacity(items.len());
                    for x in items {
                        out.push(vm.call(&f, &[x])?);
                    }
                    Ok(Value::list(out))
                }
                "filter" => {
                    let mut out = Vec::new();
                    for x in items {
                        if vm.call(&f, std::slice::from_ref(&x))?.truthy() {
                            out.push(x);
                        }
                    }
                    Ok(Value::list(out))
                }
                "each" => {
                    for x in items {
                        vm.call(&f, &[x])?;
                    }
                    Ok(Value::Nil)
                }
                "find" => {
                    for x in items {
                        if vm.call(&f, std::slice::from_ref(&x))?.truthy() {
                            return Ok(x);
                        }
                    }
                    Ok(Value::Nil)
                }
                "group_by" => {
                    let mut out: IndexMap<Key, Value> = IndexMap::new();
                    for x in items {
                        let k = Key::from(&vm.call(&f, std::slice::from_ref(&x))?)?;
                        match out.entry(k).or_insert_with(|| Value::list(vec![])) {
                            Value::List(g) => g.borrow_mut().push(x),
                            _ => unreachable!(),
                        }
                    }
                    Ok(Value::map(out))
                }
                _ => {
                    let mut out: IndexMap<Key, Value> = IndexMap::new();
                    for x in items {
                        let k = Key::from(&vm.call(&f, &[x])?)?;
                        let e = out.entry(k).or_insert(Value::Int(0));
                        if let Value::Int(n) = e {
                            *n += 1;
                        }
                    }
                    Ok(Value::map(out))
                }
            }
        }
        "reduce" => {
            let [f, init] = a.bind(["fn", "init"])?;
            let f = f_arg(f)?;
            let items = l.borrow().clone();
            let mut it = items.into_iter();
            let mut acc = match init {
                Some(v) => v,
                None => it.next().ok_or_else(|| value_err("reduce() of empty list with no init"))?,
            };
            for x in it {
                acc = vm.call(&f, &[acc, x])?;
            }
            Ok(acc)
        }
        "any" | "all" => {
            let [f] = a.bind(["fn"])?;
            let items = l.borrow().clone();
            let f = opt(f);
            for x in items {
                let t = match &f {
                    Some(f) => vm.call(f, &[x])?.truthy(),
                    None => x.truthy(),
                };
                if name == "any" && t {
                    return Ok(Value::Bool(true));
                }
                if name == "all" && !t {
                    return Ok(Value::Bool(false));
                }
            }
            Ok(Value::Bool(name == "all"))
        }
        "sum" => {
            let [start] = a.bind(["start"])?;
            let mut args = vec![Value::List(l.clone())];
            if let Some(s) = start {
                args.push(s);
            }
            (super::BUILTINS[super::builtin_index("sum").unwrap() as usize].f)(vm, Args::new(args))
        }
        "min" | "max" => {
            let [key] = a.bind(["key"])?;
            let mut args = Args::new(vec![Value::List(l.clone())]);
            if let Some(k) = key {
                args.kw.push(("key".into(), k));
            }
            (super::BUILTINS[super::builtin_index(name).unwrap() as usize].f)(vm, args)
        }
        "copy" => {
            a.bind([])?;
            Ok(Value::list(l.borrow().clone()))
        }
        "clear" => {
            a.bind([])?;
            l.borrow_mut().clear();
            Ok(Value::Nil)
        }
        "join" => {
            let [sep] = a.bind(["sep"])?;
            let sep = match sep {
                Some(s) => s.as_str("sep")?.to_string(),
                None => String::new(),
            };
            let items = l.borrow().clone();
            let mut parts = Vec::with_capacity(items.len());
            for x in &items {
                parts.push(vm.display(x, false)?);
            }
            Ok(Value::str(parts.join(&sep)))
        }
        "first" | "last" => {
            a.bind([])?;
            let l = l.borrow();
            let v = if name == "first" { l.first() } else { l.last() };
            Ok(v.cloned().unwrap_or(Value::Nil))
        }
        "is_empty" => {
            a.bind([])?;
            Ok(Value::Bool(l.borrow().is_empty()))
        }
        "unique" => {
            a.bind([])?;
            let mut out: Vec<Value> = Vec::new();
            for x in l.borrow().iter() {
                if !out.iter().any(|y| equal(x, y)) {
                    out.push(x.clone());
                }
            }
            Ok(Value::list(out))
        }
        "flatten" => {
            a.bind([])?;
            let mut out = Vec::new();
            for x in l.borrow().iter() {
                match x {
                    Value::List(inner) => out.extend(inner.borrow().iter().cloned()),
                    other => out.push(other.clone()),
                }
            }
            Ok(Value::list(out))
        }
        "zip" => {
            let [other] = a.bind(["other"])?;
            let other = to_vec(&need(other, "other")?, "other")?;
            let items = l.borrow();
            Ok(Value::list(items.iter().zip(other).map(|(x, y)| Value::list(vec![x.clone(), y])).collect()))
        }
        "enumerate" => {
            a.bind([])?;
            Ok(Value::list(l.borrow().iter().enumerate().map(|(i, x)| Value::list(vec![Value::Int(i as i64), x.clone()])).collect()))
        }
        "take" | "skip" | "chunk" => {
            let [n] = a.bind(["n"])?;
            let n = need(n, "n")?.int("n")?.max(0) as usize;
            let items = l.borrow();
            Ok(match name {
                "take" => Value::list(items.iter().take(n).cloned().collect()),
                "skip" => Value::list(items.iter().skip(n).cloned().collect()),
                _ => {
                    if n == 0 {
                        return Err(value_err("chunk size must be at least 1"));
                    }
                    Value::list(items.chunks(n).map(|c| Value::list(c.to_vec())).collect())
                }
            })
        }
        _ => Err(err("AttributeError", format!("list has no method `{name}`"))),
    }
}
