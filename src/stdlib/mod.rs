pub mod ab;
pub mod bandit;
pub mod core;
pub mod fmt;
pub mod generators;
pub mod io;
pub mod json;
pub mod list;
pub mod map;
pub mod math;
pub mod power;
pub mod rand;
pub mod stats;
pub mod str;
pub mod table;
pub mod testing;
pub mod time;
pub mod vec;

use crate::vm::{Args, Flow, Native, R, Value, Vm, err};
use indexmap::IndexMap;
use std::rc::Rc;

pub use self::core::BUILTINS;

pub fn builtin_index(name: &str) -> Option<u16> {
    BUILTINS.iter().position(|n| n.name == name).map(|i| i as u16)
}

pub const MODULES: &[&str] = &["math", "time", "io", "json", "rand", "gen", "stats", "ab", "power", "bandit"];

pub fn is_module(name: &str) -> bool {
    MODULES.contains(&name)
}

// members of a built-in module
pub fn module(name: &str) -> Option<IndexMap<Rc<str>, Value>> {
    let (fns, consts): (&'static [Native], Vec<(&str, Value)>) = match name {
        "math" => (math::FNS, math::consts()),
        "time" => (time::FNS, vec![]),
        "io" => (io::FNS, vec![]),
        "json" => (json::FNS, vec![]),
        "rand" => (rand::FNS, vec![]),
        "gen" => (generators::FNS, vec![]),
        "stats" => (stats::FNS, vec![]),
        "ab" => (ab::FNS, vec![]),
        "power" => (power::FNS, vec![]),
        "bandit" => (bandit::FNS, vec![]),
        _ => return None,
    };
    let mut m: IndexMap<Rc<str>, Value> = fns.iter().map(|n| (Rc::from(n.name), Value::Native(n))).collect();
    m.extend(consts.into_iter().map(|(k, v)| (Rc::from(k), v)));
    Some(m)
}

pub fn has_method(recv: &Value, name: &str) -> bool {
    let names: &[&str] = match recv {
        Value::Str(_) => str::METHODS,
        Value::List(_) => list::METHODS,
        Value::Map(_) => map::METHODS,
        Value::Range(..) => &["len", "list"],
        Value::Object(o) => o.methods(),
        _ => &[],
    };
    names.contains(&name)
}

pub fn call_method(vm: &mut Vm, recv: &Value, name: &str, a: Args) -> R {
    match recv {
        Value::Str(s) => str::call(vm, s, name, a),
        Value::List(l) => list::call(vm, l, name, a),
        Value::Map(m) => map::call(vm, m, name, a),
        Value::Object(o) => o.clone().call_method(vm, recv, name, a),
        Value::Range(lo, hi) => {
            a.bind([])?;
            match name {
                "len" => Ok(Value::Int((hi - lo).max(0))),
                _ => Ok(Value::list((*lo..*hi).map(Value::Int).collect())),
            }
        }
        _ => Err(err("AttributeError", format!("{} has no method `{name}`", recv.type_name()))),
    }
}

// collect any iterable into a Vec
pub fn to_vec(v: &Value, what: &str) -> Result<Vec<Value>, Flow> {
    Ok(match v {
        Value::List(l) => l.borrow().clone(),
        Value::Range(a, b) => {
            if b - a > 100_000_000 {
                return Err(err("MemoryError", "range too big to turn into a list"));
            }
            (*a..*b).map(Value::Int).collect()
        }
        Value::Str(s) => s.chars().map(|c| Value::str(c.to_string())).collect(),
        Value::Map(m) => m.borrow().keys().map(|k| k.value()).collect(),
        Value::Object(o) if o.items().is_some() => o.items().unwrap(),
        other => {
            return Err(crate::vm::type_err(format!("{what} must be a list, range, str or map, got {}", other.kind_name())));
        }
    })
}

// numbers from any iterable
pub fn to_f64s(v: &Value, what: &str) -> Result<Vec<f64>, Flow> {
    match v {
        Value::List(l) => l.borrow().iter().map(|x| x.num(what)).collect(),
        Value::Object(o) if o.numbers().is_some() => Ok(o.numbers().unwrap().to_vec()),
        other => to_vec(other, what)?.iter().map(|x| x.num(what)).collect(),
    }
}
