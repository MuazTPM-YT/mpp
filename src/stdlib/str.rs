use crate::vm::*;
use std::rc::Rc;

pub const METHODS: &[&str] = &[
    "len",
    "upper",
    "lower",
    "strip",
    "lstrip",
    "rstrip",
    "split",
    "join",
    "replace",
    "startswith",
    "endswith",
    "contains",
    "find",
    "count",
    "lines",
    "chars",
    "isdigit",
    "isalpha",
    "isspace",
    "title",
    "capitalize",
    "ljust",
    "rjust",
    "center",
    "format",
];

fn s_arg(v: Option<Value>, name: &str) -> Result<Rc<str>, Flow> {
    Ok(need(v, name)?.as_str(name)?.clone())
}

fn pad(s: &str, width: Option<Value>, fill: Option<Value>, how: u8) -> R {
    let w = need(width, "width")?.int("width")?.max(0) as usize;
    let f = match fill {
        Some(f) => f.as_str("fill")?.chars().next().unwrap_or(' '),
        None => ' ',
    };
    let n = s.chars().count();
    if n >= w {
        return Ok(Value::str(s));
    }
    let gap = w - n;
    let (l, r) = match how {
        b'l' => (0, gap),
        b'r' => (gap, 0),
        _ => (gap / 2, gap - gap / 2),
    };
    let fs = |k: usize| std::iter::repeat_n(f, k).collect::<String>();
    Ok(Value::str(format!("{}{s}{}", fs(l), fs(r))))
}

pub fn call(vm: &mut Vm, s: &Rc<str>, name: &str, a: Args) -> R {
    match name {
        "len" => {
            a.bind([])?;
            Ok(Value::Int(s.chars().count() as i64))
        }
        "upper" | "lower" | "title" | "capitalize" | "lines" | "chars" | "isdigit" | "isalpha" | "isspace" => {
            a.bind([])?;
            Ok(match name {
                "upper" => Value::str(s.to_uppercase()),
                "lower" => Value::str(s.to_lowercase()),
                "title" => Value::str(
                    s.split(' ')
                        .map(|w| {
                            let mut c = w.chars();
                            c.next().map_or(String::new(), |f| f.to_uppercase().chain(c.flat_map(char::to_lowercase)).collect())
                        })
                        .collect::<Vec<_>>()
                        .join(" "),
                ),
                "capitalize" => {
                    let mut c = s.chars();
                    Value::str(c.next().map_or(String::new(), |f| f.to_uppercase().chain(c.flat_map(char::to_lowercase)).collect()))
                }
                "lines" => Value::list(s.lines().map(Value::str).collect()),
                "chars" => Value::list(s.chars().map(|c| Value::str(c.to_string())).collect()),
                "isdigit" => Value::Bool(!s.is_empty() && s.chars().all(|c| c.is_ascii_digit())),
                "isalpha" => Value::Bool(!s.is_empty() && s.chars().all(char::is_alphabetic)),
                _ => Value::Bool(!s.is_empty() && s.chars().all(char::is_whitespace)),
            })
        }
        "strip" | "lstrip" | "rstrip" => {
            let [chars] = a.bind(["chars"])?;
            let set: Option<Vec<char>> = match opt(chars) {
                Some(c) => Some(c.as_str("chars")?.chars().collect()),
                None => None,
            };
            let hit = |c: char| set.as_ref().map_or(c.is_whitespace(), |v| v.contains(&c));
            Ok(Value::str(match name {
                "strip" => s.trim_matches(hit),
                "lstrip" => s.trim_start_matches(hit),
                _ => s.trim_end_matches(hit),
            }))
        }
        "split" => {
            let [sep, max] = a.bind(["sep", "max"])?;
            let max = match opt(max) {
                Some(m) => Some(m.int("max")?.max(0) as usize),
                None => None,
            };
            let parts: Vec<Value> = match opt(sep) {
                None => match max {
                    Some(m) => {
                        let mut out: Vec<Value> = Vec::new();
                        let mut rest = s.trim_start();
                        while !rest.is_empty() {
                            if out.len() == m {
                                out.push(Value::str(rest));
                                break;
                            }
                            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                            out.push(Value::str(&rest[..end]));
                            rest = rest[end..].trim_start();
                        }
                        out
                    }
                    None => s.split_whitespace().map(Value::str).collect(),
                },
                Some(sep) => {
                    let sep = sep.as_str("sep")?.clone();
                    if sep.is_empty() {
                        return Err(value_err("empty separator"));
                    }
                    match max {
                        Some(m) => s.splitn(m + 1, &*sep).map(Value::str).collect(),
                        None => s.split(&*sep).map(Value::str).collect(),
                    }
                }
            };
            Ok(Value::list(parts))
        }
        "join" => {
            let [items] = a.bind(["items"])?;
            let items = super::to_vec(&need(items, "items")?, "items")?;
            let mut parts = Vec::with_capacity(items.len());
            for x in &items {
                parts.push(vm.display(x, false)?);
            }
            Ok(Value::str(parts.join(s)))
        }
        "replace" => {
            let [from, to, count] = a.bind(["old", "new", "count"])?;
            let (from, to) = (s_arg(from, "old")?, s_arg(to, "new")?);
            Ok(Value::str(match opt(count) {
                Some(c) => s.replacen(&*from, &to, c.int("count")?.max(0) as usize),
                None => s.replace(&*from, &to),
            }))
        }
        "startswith" | "endswith" | "contains" | "find" | "count" => {
            let [sub] = a.bind(["sub"])?;
            let sub = s_arg(sub, "sub")?;
            Ok(match name {
                "startswith" => Value::Bool(s.starts_with(&*sub)),
                "endswith" => Value::Bool(s.ends_with(&*sub)),
                "contains" => Value::Bool(s.contains(&*sub)),
                "find" => Value::Int(s.find(&*sub).map_or(-1, |b| s[..b].chars().count() as i64)),
                _ => {
                    if sub.is_empty() {
                        return Err(value_err("cannot count empty string"));
                    }
                    Value::Int(s.matches(&*sub).count() as i64)
                }
            })
        }
        "ljust" | "rjust" | "center" => {
            let [w, f] = a.bind(["width", "fill"])?;
            pad(
                s,
                w,
                f,
                match name {
                    "ljust" => b'l',
                    "rjust" => b'r',
                    _ => b'c',
                },
            )
        }
        "format" => {
            let [spec] = a.bind(["spec"])?;
            let spec = s_arg(spec, "spec")?;
            Ok(Value::str(super::fmt::format_spec(vm, &Value::Str(s.clone()), &spec)?))
        }
        _ => Err(err("AttributeError", format!("str has no method `{name}`"))),
    }
}
