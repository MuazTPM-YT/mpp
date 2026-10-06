use crate::vm::*;
use std::rc::Rc;

// input generator for property tests
pub const METHODS: &[&str] = &["sample"];

pub enum Gen {
    Int(i64, i64),
    Float(f64, f64),
    Bool,
    Choice(Vec<Value>),
    Str { min: usize, max: usize, chars: Vec<char> },
    List(Rc<Gen>, usize, usize),
    Const(Value),
    OneOf(Vec<Rc<Gen>>),
}

impl Object for Gen {
    fn type_name(&self) -> &'static str {
        "generator"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn display(&self) -> String {
        match self {
            Gen::Int(a, b) => format!("<gen.int({a}, {b})>"),
            Gen::Float(a, b) => format!("<gen.float({}, {})>", fmt_float(*a), fmt_float(*b)),
            Gen::Bool => "<gen.bool()>".into(),
            Gen::Choice(v) => format!("<gen.choice({} items)>", v.len()),
            Gen::Str { min, max, .. } => format!("<gen.str({min}, {max})>"),
            Gen::List(_, a, b) => format!("<gen.list(.., {a}, {b})>"),
            Gen::Const(_) => "<gen.just(..)>".into(),
            Gen::OneOf(v) => format!("<gen.one_of({} gens)>", v.len()),
        }
    }
    fn methods(&self) -> &'static [&'static str] {
        METHODS
    }
    fn call_method(&self, vm: &mut Vm, _this: &Value, name: &str, a: Args) -> R {
        match name {
            "sample" => {
                a.bind([])?;
                Ok(self.sample(&mut vm.rng, 1.0))
            }
            _ => Err(err("AttributeError", format!("generator has no method `{name}`"))),
        }
    }
}

impl Gen {
    // size 0..1 grows with each run: small cases first
    pub fn sample(&self, rng: &mut super::rand::Rng, size: f64) -> Value {
        match self {
            Gen::Int(lo, hi) => {
                // edges often find bugs: try them sometimes
                match rng.below(8) {
                    0 => {
                        let edges = [*lo, *hi, 0i64.clamp(*lo, *hi)];
                        return Value::Int(edges[rng.below(3) as usize]);
                    }
                    // small numbers hide many bugs: try them often
                    1 | 2 => {
                        let (a, b) = ((-10i64).clamp(*lo, *hi), 10i64.clamp(*lo, *hi));
                        return Value::Int(a + rng.below((b - a) as u64 + 1) as i64);
                    }
                    _ => {}
                }
                let span = (*hi as i128 - *lo as i128) as f64;
                let width = (span * size.max(0.05)).ceil().max(1.0) as i128;
                let center = 0i128.clamp(*lo as i128, *hi as i128);
                let a = (center - width).max(*lo as i128);
                let b = (center + width).min(*hi as i128);
                let n = (b - a + 1) as u128;
                let r = if n > u64::MAX as u128 { rng.next_u64() as i128 } else { rng.below(n as u64) as i128 };
                Value::Int((a + r) as i64)
            }
            Gen::Float(lo, hi) => {
                if rng.below(8) == 0 {
                    let edges = [*lo, *hi, 0f64.clamp(*lo, *hi)];
                    return Value::Float(edges[rng.below(3) as usize]);
                }
                Value::Float(lo + (hi - lo) * rng.float())
            }
            Gen::Bool => Value::Bool(rng.below(2) == 1),
            Gen::Choice(v) => v[rng.below(v.len() as u64) as usize].clone(),
            Gen::Str { min, max, chars } => {
                let top = min + ((max - min) as f64 * size).round() as usize;
                let n = min + rng.below((top - min + 1) as u64) as usize;
                Value::str((0..n).map(|_| chars[rng.below(chars.len() as u64) as usize]).collect::<String>())
            }
            Gen::List(g, min, max) => {
                let top = min + ((max - min) as f64 * size).round() as usize;
                let n = min + rng.below((top - min + 1) as u64) as usize;
                Value::list((0..n).map(|_| g.sample(rng, size)).collect())
            }
            Gen::Const(v) => v.clone(),
            Gen::OneOf(gs) => gs[rng.below(gs.len() as u64) as usize].sample(rng, size),
        }
    }

    // simpler values to try when a case fails, simplest first
    pub fn shrink(&self, v: &Value) -> Vec<Value> {
        match (self, v) {
            (Gen::Int(lo, hi), Value::Int(n)) => {
                let t = 0i64.clamp(*lo, *hi);
                let mut out = Vec::new();
                if *n != t {
                    out.push(Value::Int(t));
                    let half = t + (n - t) / 2;
                    if half != t && half != *n {
                        out.push(Value::Int(half));
                    }
                    out.push(Value::Int(if *n > t { n - 1 } else { n + 1 }));
                }
                out
            }
            (Gen::Float(lo, hi), Value::Float(x)) => {
                let t = 0f64.clamp(*lo, *hi);
                let mut out = Vec::new();
                if *x != t {
                    out.push(Value::Float(t));
                    let tr = x.trunc().clamp(*lo, *hi);
                    if tr != *x && tr != t {
                        out.push(Value::Float(tr));
                    }
                    let half = t + (x - t) / 2.0;
                    if (half - x).abs() > 1e-9 {
                        out.push(Value::Float(half));
                    }
                }
                out
            }
            (Gen::Bool, Value::Bool(true)) => vec![Value::Bool(false)],
            (Gen::Choice(opts), x) => {
                let i = opts.iter().position(|o| equal(o, x)).unwrap_or(0);
                opts[..i].iter().take(3).cloned().collect()
            }
            (Gen::Str { min, .. }, Value::Str(s)) => {
                let chars: Vec<char> = s.chars().collect();
                let mut out = Vec::new();
                if chars.len() > *min {
                    out.push(Value::str(chars[..*min].iter().collect::<String>()));
                    if chars.len() / 2 > *min {
                        out.push(Value::str(chars[..chars.len() / 2].iter().collect::<String>()));
                    }
                    for i in 0..chars.len().min(8) {
                        let mut c = chars.clone();
                        c.remove(i);
                        out.push(Value::str(c.iter().collect::<String>()));
                    }
                }
                out
            }
            (Gen::List(g, min, _), Value::List(l)) => {
                let items = l.borrow().clone();
                let mut out = Vec::new();
                if items.len() > *min {
                    out.push(Value::list(items[..*min].to_vec()));
                    if items.len() / 2 > *min {
                        out.push(Value::list(items[..items.len() / 2].to_vec()));
                    }
                    for i in 0..items.len().min(8) {
                        let mut c = items.clone();
                        c.remove(i);
                        out.push(Value::list(c));
                    }
                }
                for (i, x) in items.iter().enumerate().take(8) {
                    for s in g.shrink(x) {
                        let mut c = items.clone();
                        c[i] = s;
                        out.push(Value::list(c));
                    }
                }
                out
            }
            (Gen::OneOf(gs), x) => gs.iter().flat_map(|g| g.shrink(x)).take(6).collect(),
            _ => Vec::new(),
        }
    }
}

pub static FNS: &[Native] = &[
    Native { name: "int", f: int },
    Native { name: "float", f: float },
    Native { name: "bool", f: boolean },
    Native { name: "choice", f: choice },
    Native { name: "str", f: string },
    Native { name: "list", f: list },
    Native { name: "just", f: just },
    Native { name: "one_of", f: one_of },
];

fn mk(g: Gen) -> R {
    Ok(Value::Object(Rc::new(g)))
}

fn gen_arg(v: Option<Value>, name: &str) -> Result<Rc<Gen>, Flow> {
    let v = need(v, name)?;
    match &v {
        Value::Object(o) if o.as_any().is::<Gen>() => {
            let g = v.object::<Gen>().unwrap();
            // rebuild a shareable copy of the generator
            Ok(Rc::new(clone_gen(g)))
        }
        other => Ok(Rc::new(Gen::Const(other.clone()))),
    }
}

fn clone_gen(g: &Gen) -> Gen {
    match g {
        Gen::Int(a, b) => Gen::Int(*a, *b),
        Gen::Float(a, b) => Gen::Float(*a, *b),
        Gen::Bool => Gen::Bool,
        Gen::Choice(v) => Gen::Choice(v.clone()),
        Gen::Str { min, max, chars } => Gen::Str { min: *min, max: *max, chars: chars.clone() },
        Gen::List(g, a, b) => Gen::List(g.clone(), *a, *b),
        Gen::Const(v) => Gen::Const(v.clone()),
        Gen::OneOf(v) => Gen::OneOf(v.clone()),
    }
}

fn int(_: &mut Vm, a: Args) -> R {
    let [lo, hi] = a.bind(["lo", "hi"])?;
    let lo = lo.map_or(Ok(-1000), |v| v.int("lo"))?;
    let hi = hi.map_or(Ok(1000), |v| v.int("hi"))?;
    if lo > hi {
        return Err(value_err("gen.int needs lo <= hi"));
    }
    mk(Gen::Int(lo, hi))
}

fn float(_: &mut Vm, a: Args) -> R {
    let [lo, hi] = a.bind(["lo", "hi"])?;
    let lo = lo.map_or(Ok(-1000.0), |v| v.num("lo"))?;
    let hi = hi.map_or(Ok(1000.0), |v| v.num("hi"))?;
    if lo > hi || !lo.is_finite() || !hi.is_finite() {
        return Err(value_err("gen.float needs finite lo <= hi"));
    }
    mk(Gen::Float(lo, hi))
}

fn boolean(_: &mut Vm, a: Args) -> R {
    a.bind([])?;
    mk(Gen::Bool)
}

fn choice(_: &mut Vm, a: Args) -> R {
    let [xs] = a.bind(["xs"])?;
    let items = super::to_vec(&need(xs, "xs")?, "xs")?;
    if items.is_empty() {
        return Err(value_err("gen.choice needs at least one item"));
    }
    mk(Gen::Choice(items))
}

fn string(_: &mut Vm, a: Args) -> R {
    let [min, max, chars] = a.bind(["min", "max", "chars"])?;
    let min = min.map_or(Ok(0), |v| v.int("min"))?.max(0) as usize;
    let max = max.map_or(Ok(20), |v| v.int("max"))?.max(0) as usize;
    if min > max {
        return Err(value_err("gen.str needs min <= max"));
    }
    let chars: Vec<char> = match opt(chars) {
        Some(c) => c.as_str("chars")?.chars().collect(),
        None => "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 _-.,!?é漢".chars().collect(),
    };
    if chars.is_empty() {
        return Err(value_err("gen.str needs some chars"));
    }
    mk(Gen::Str { min, max, chars })
}

fn list(_: &mut Vm, a: Args) -> R {
    let [g, min, max] = a.bind(["of", "min", "max"])?;
    let g = gen_arg(g, "of")?;
    let min = min.map_or(Ok(0), |v| v.int("min"))?.max(0) as usize;
    let max = max.map_or(Ok(20), |v| v.int("max"))?.max(0) as usize;
    if min > max {
        return Err(value_err("gen.list needs min <= max"));
    }
    mk(Gen::List(g, min, max))
}

fn just(_: &mut Vm, a: Args) -> R {
    let [v] = a.bind(["value"])?;
    mk(Gen::Const(need(v, "value")?))
}

fn one_of(_: &mut Vm, a: Args) -> R {
    a.no_kw()?;
    if a.pos.is_empty() {
        return Err(value_err("gen.one_of needs generators"));
    }
    let gs = a.pos.into_iter().map(|v| gen_arg(Some(v), "generator")).collect::<Result<_, _>>()?;
    mk(Gen::OneOf(gs))
}
