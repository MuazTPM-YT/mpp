use super::stats::desc;
use crate::compile::chunk::Op;
use crate::vm::ops::norm_index;
use crate::vm::*;
use indexmap::IndexMap;
use std::rc::Rc;

// immutable f64 array; math runs in Rust
pub struct NumVec(pub Rc<[f64]>);

pub fn vec_value(v: Vec<f64>) -> Value {
    Value::Object(Rc::new(NumVec(v.into())))
}

fn mask(v: impl Iterator<Item = bool>) -> Value {
    vec_value(v.map(|b| b as i64 as f64).collect())
}

fn num_out(x: f64) -> Value {
    Value::Float(x)
}

pub const METHODS: &[&str] = &[
    "len",
    "sum",
    "mean",
    "median",
    "var",
    "std",
    "sem",
    "min",
    "max",
    "quantile",
    "skew",
    "kurtosis",
    "cumsum",
    "diff",
    "abs",
    "sqrt",
    "log",
    "exp",
    "round",
    "clip",
    "sorted",
    "rank",
    "zscore",
    "argmin",
    "argmax",
    "dot",
    "corr",
    "cov",
    "head",
    "tail",
    "list",
    "filter",
    "map",
    "dropna",
    "isnan",
    "unique",
    "describe",
    "histogram",
    "any",
    "all",
    "eq",
    "ne",
    "between",
    "sample",
    "count",
    "prod",
];

impl Object for NumVec {
    fn type_name(&self) -> &'static str {
        "vec"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn display(&self) -> String {
        let n = self.0.len();
        let shown: Vec<String> = self.0.iter().take(8).map(|x| fmt_float(*x)).collect();
        if n > 8 { format!("vec([{}, ...] n={n})", shown.join(", ")) } else { format!("vec([{}])", shown.join(", ")) }
    }
    fn get(&self, name: &str) -> Option<Value> {
        (name == "size").then(|| Value::Int(self.0.len() as i64))
    }
    fn methods(&self) -> &'static [&'static str] {
        METHODS
    }
    fn len(&self) -> Option<usize> {
        Some(self.0.len())
    }
    fn items(&self) -> Option<Vec<Value>> {
        Some(self.0.iter().map(|x| Value::Float(*x)).collect())
    }
    fn numbers(&self) -> Option<Rc<[f64]>> {
        Some(self.0.clone())
    }
    fn equals(&self, other: &Value) -> bool {
        other.object::<NumVec>().is_some_and(|o| o.0.len() == self.0.len() && o.0.iter().zip(self.0.iter()).all(|(a, b)| a == b))
    }
    fn index(&self, idx: &Value) -> Option<R> {
        let n = self.0.len();
        Some(match idx {
            Value::Int(_) | Value::Float(_) | Value::Bool(_) => idx.int("vec index").and_then(|i| {
                norm_index(i, n)
                    .map(|j| Value::Float(self.0[j]))
                    .ok_or_else(|| err("IndexError", format!("index {i} out of range (length {n})")))
            }),
            other => {
                // mask (same length) or list of positions
                let picks = match super::to_f64s(other, "vec index") {
                    Ok(p) => p,
                    Err(e) => return Some(Err(e)),
                };
                let is_mask = other.object::<NumVec>().is_some() && picks.len() == n && picks.iter().all(|x| *x == 0.0 || *x == 1.0);
                if is_mask {
                    Ok(vec_value(self.0.iter().zip(&picks).filter(|(_, m)| **m == 1.0).map(|(x, _)| *x).collect()))
                } else {
                    let mut out = Vec::with_capacity(picks.len());
                    for p in picks {
                        match norm_index(p as i64, n) {
                            Some(j) => out.push(self.0[j]),
                            None => return Some(Err(err("IndexError", format!("index {p} out of range (length {n})")))),
                        }
                    }
                    Ok(vec_value(out))
                }
            }
        })
    }
    fn slice(&self, lo: &Value, hi: &Value) -> Option<R> {
        let n = self.0.len() as i64;
        let fix = |v: &Value, d: i64| -> Result<usize, Flow> {
            match v {
                Value::Nil => Ok(d as usize),
                v => {
                    let i = v.int("slice bound")?;
                    Ok((if i < 0 { i + n } else { i }).clamp(0, n) as usize)
                }
            }
        };
        Some((|| {
            let (a, b) = (fix(lo, 0)?, fix(hi, n)?);
            Ok(vec_value(self.0[a..b.max(a)].to_vec()))
        })())
    }
    fn binary(&self, op: Op, other: &Value, swapped: bool) -> Option<R> {
        if matches!(op, Op::Eq | Op::Ne | Op::In | Op::NotIn | Op::Approx) {
            return None;
        }
        let a = &self.0;
        let b: Result<Vec<f64>, Flow> = match other {
            Value::Int(_) | Value::Float(_) | Value::Bool(_) => other.num("").map(|x| vec![x; a.len()]),
            o => super::to_f64s(o, "right side"),
        };
        let b = match b {
            Ok(b) => b,
            Err(e) => return Some(Err(e)),
        };
        if b.len() != a.len() {
            return Some(Err(value_err(format!("vec lengths differ: {} and {}", a.len(), b.len()))));
        }
        let pairs = a.iter().zip(b.iter()).map(|(x, y)| if swapped { (*y, *x) } else { (*x, *y) });
        Some(Ok(match op {
            Op::Add => vec_value(pairs.map(|(x, y)| x + y).collect()),
            Op::Sub => vec_value(pairs.map(|(x, y)| x - y).collect()),
            Op::Mul => vec_value(pairs.map(|(x, y)| x * y).collect()),
            Op::Div => vec_value(pairs.map(|(x, y)| x / y).collect()),
            Op::IntDiv => vec_value(pairs.map(|(x, y)| (x / y).floor()).collect()),
            Op::Mod => vec_value(pairs.map(|(x, y)| x - y * (x / y).floor()).collect()),
            Op::Pow => vec_value(pairs.map(|(x, y)| x.powf(y)).collect()),
            Op::Lt => mask(pairs.map(|(x, y)| x < y)),
            Op::Le => mask(pairs.map(|(x, y)| x <= y)),
            Op::Gt => mask(pairs.map(|(x, y)| x > y)),
            Op::Ge => mask(pairs.map(|(x, y)| x >= y)),
            _ => return None,
        }))
    }
    fn call_method(&self, vm: &mut Vm, _this: &Value, name: &str, a: Args) -> R {
        let x = &self.0;
        let n = x.len();
        match name {
            "len" | "sum" | "mean" | "median" | "min" | "max" | "skew" | "kurtosis" | "sem" | "prod" | "argmin" | "argmax" => {
                a.bind([])?;
                Ok(match name {
                    "len" => Value::Int(n as i64),
                    "sum" => num_out(desc::sum(x)),
                    "mean" => num_out(desc::mean(x)),
                    "median" => num_out(desc::median(x)),
                    "min" => num_out(desc::min(x)),
                    "max" => num_out(desc::max(x)),
                    "skew" => num_out(desc::skew(x)),
                    "kurtosis" => num_out(desc::kurtosis(x)),
                    "sem" => num_out(desc::sem(x)),
                    "prod" => num_out(x.iter().product()),
                    _ => {
                        if n == 0 {
                            return Err(value_err(format!("{name}() of empty vec")));
                        }
                        let best = (0..n)
                            .fold(0, |b, i| if (name == "argmin" && x[i] < x[b]) || (name == "argmax" && x[i] > x[b]) { i } else { b });
                        Value::Int(best as i64)
                    }
                })
            }
            "var" | "std" => {
                let [ddof] = a.bind(["ddof"])?;
                let d = ddof.map_or(Ok(1.0), |v| v.num("ddof"))?;
                Ok(num_out(if name == "var" { desc::var(x, d) } else { desc::std(x, d) }))
            }
            "quantile" => {
                let [q] = a.bind(["q"])?;
                let q = need(q, "q")?;
                let s = desc::sorted(x);
                match &q {
                    Value::List(_) | Value::Object(_) => {
                        Ok(vec_value(super::to_f64s(&q, "q")?.iter().map(|p| desc::quantile_sorted(&s, *p)).collect()))
                    }
                    other => Ok(num_out(desc::quantile_sorted(&s, other.num("q")?))),
                }
            }
            "cumsum" | "diff" | "abs" | "sqrt" | "log" | "exp" | "rank" | "zscore" | "isnan" | "dropna" | "unique" | "list" => {
                a.bind([])?;
                Ok(match name {
                    "cumsum" => vec_value(
                        x.iter()
                            .scan(0.0, |s, v| {
                                *s += v;
                                Some(*s)
                            })
                            .collect(),
                    ),
                    "diff" => vec_value(x.windows(2).map(|w| w[1] - w[0]).collect()),
                    "abs" => vec_value(x.iter().map(|v| v.abs()).collect()),
                    "sqrt" => vec_value(x.iter().map(|v| v.sqrt()).collect()),
                    "log" => vec_value(x.iter().map(|v| v.ln()).collect()),
                    "exp" => vec_value(x.iter().map(|v| v.exp()).collect()),
                    "rank" => vec_value(desc::rank(x)),
                    "zscore" => {
                        let (m, s) = (desc::mean(x), desc::std(x, 0.0));
                        vec_value(x.iter().map(|v| (v - m) / s).collect())
                    }
                    "isnan" => mask(x.iter().map(|v| v.is_nan())),
                    "dropna" => vec_value(x.iter().copied().filter(|v| !v.is_nan()).collect()),
                    "unique" => {
                        let mut s = desc::sorted(x);
                        s.dedup();
                        vec_value(s)
                    }
                    _ => Value::list(x.iter().map(|v| Value::Float(*v)).collect()),
                })
            }
            "round" => {
                let [nd] = a.bind(["ndigits"])?;
                let p = 10f64.powi(nd.map_or(Ok(0), |v| v.int("ndigits"))? as i32);
                Ok(vec_value(x.iter().map(|v| (v * p).round_ties_even() / p).collect()))
            }
            "clip" | "between" => {
                let [lo, hi] = a.bind(["lo", "hi"])?;
                let lo = lo.map_or(Ok(f64::NEG_INFINITY), |v| v.num("lo"))?;
                let hi = hi.map_or(Ok(f64::INFINITY), |v| v.num("hi"))?;
                Ok(if name == "clip" {
                    vec_value(x.iter().map(|v| v.clamp(lo, hi)).collect())
                } else {
                    mask(x.iter().map(|v| *v >= lo && *v <= hi))
                })
            }
            "eq" | "ne" => {
                let [v] = a.bind(["x"])?;
                let v = need(v, "x")?.num("x")?;
                Ok(mask(x.iter().map(|y| (*y == v) == (name == "eq"))))
            }
            "sorted" => {
                let [rev] = a.bind(["reverse"])?;
                let mut s = desc::sorted(x);
                if rev.is_some_and(|r| r.truthy()) {
                    s.reverse();
                }
                Ok(vec_value(s))
            }
            "dot" | "corr" | "cov" => {
                let [o] = a.bind(["other"])?;
                let y = super::to_f64s(&need(o, "other")?, "other")?;
                if y.len() != n {
                    return Err(value_err(format!("vec lengths differ: {n} and {}", y.len())));
                }
                Ok(num_out(match name {
                    "dot" => desc::sum(&x.iter().zip(&y).map(|(a, b)| a * b).collect::<Vec<_>>()),
                    "corr" => desc::pearson_r(x, &y),
                    _ => desc::cov(x, &y),
                }))
            }
            "head" | "tail" => {
                let [k] = a.bind(["n"])?;
                let k = k.map_or(Ok(5), |v| v.int("n"))?.max(0) as usize;
                let k = k.min(n);
                Ok(vec_value(if name == "head" { x[..k].to_vec() } else { x[n - k..].to_vec() }))
            }
            "filter" | "map" => {
                let [f] = a.bind(["fn"])?;
                let f = need(f, "fn")?;
                let mut out = Vec::with_capacity(n);
                for v in x.iter() {
                    let r = vm.call(&f, &[Value::Float(*v)])?;
                    if name == "map" {
                        out.push(r.num("map result")?);
                    } else if r.truthy() {
                        out.push(*v);
                    }
                }
                Ok(vec_value(out))
            }
            "any" | "all" => {
                a.bind([])?;
                Ok(Value::Bool(if name == "any" { x.iter().any(|v| *v != 0.0) } else { x.iter().all(|v| *v != 0.0) }))
            }
            "count" => {
                a.bind([])?;
                Ok(Value::Int(x.iter().filter(|v| **v != 0.0 && !v.is_nan()).count() as i64))
            }
            "describe" => {
                a.bind([])?;
                Ok(describe(x))
            }
            "histogram" => {
                let [bins] = a.bind(["bins"])?;
                let b = bins.map_or(Ok(10), |v| v.int("bins"))?.max(1) as usize;
                let (lo, hi) = (desc::min(x), desc::max(x));
                let w = if hi > lo { (hi - lo) / b as f64 } else { 1.0 };
                let mut counts = vec![0i64; b];
                for v in x.iter().filter(|v| !v.is_nan()) {
                    let i = (((v - lo) / w) as usize).min(b - 1);
                    counts[i] += 1;
                }
                let edges: Vec<f64> = (0..=b).map(|i| lo + w * i as f64).collect();
                Ok(Value::record([("edges", vec_value(edges)), ("counts", Value::list(counts.into_iter().map(Value::Int).collect()))]))
            }
            "sample" => {
                let [k, replace] = a.bind(["n", "replace"])?;
                let k = need(k, "n")?.int("n")?.max(0) as usize;
                if replace.is_some_and(|r| r.truthy()) {
                    if n == 0 {
                        return Err(value_err("sample from empty vec"));
                    }
                    Ok(vec_value((0..k).map(|_| x[vm.rng.below(n as u64) as usize]).collect()))
                } else {
                    if k > n {
                        return Err(value_err(format!("sample size {k} bigger than vec ({n})")));
                    }
                    let mut v = x.to_vec();
                    vm.rng.shuffle(&mut v);
                    v.truncate(k);
                    Ok(vec_value(v))
                }
            }
            _ => Err(err("AttributeError", format!("vec has no method `{name}`"))),
        }
    }
}

// count, mean, std, min, quartiles, max
pub fn describe(x: &[f64]) -> Value {
    let clean: Vec<f64> = x.iter().copied().filter(|v| !v.is_nan()).collect();
    let s = desc::sorted(&clean);
    let mut m = IndexMap::new();
    let mut put = |k: &str, v: Value| {
        m.insert(Key::Str(k.into()), v);
    };
    put("count", Value::Int(clean.len() as i64));
    put("missing", Value::Int((x.len() - clean.len()) as i64));
    put("mean", Value::Float(desc::mean(&clean)));
    put("std", Value::Float(desc::std(&clean, 1.0)));
    put("min", Value::Float(desc::quantile_sorted(&s, 0.0)));
    put("p25", Value::Float(desc::quantile_sorted(&s, 0.25)));
    put("median", Value::Float(desc::quantile_sorted(&s, 0.5)));
    put("p75", Value::Float(desc::quantile_sorted(&s, 0.75)));
    put("max", Value::Float(desc::quantile_sorted(&s, 1.0)));
    Value::map(m)
}

pub fn vec_fn(_: &mut Vm, a: Args) -> R {
    let [xs] = a.bind(["xs"])?;
    match xs {
        // nil becomes NaN (missing)
        Some(v) => Ok(vec_value(
            super::to_vec(&v, "xs")?
                .iter()
                .map(|x| if matches!(x, Value::Nil) { Ok(f64::NAN) } else { x.num("xs") })
                .collect::<Result<_, _>>()?,
        )),
        None => Ok(vec_value(vec![])),
    }
}

pub fn linspace(_: &mut Vm, a: Args) -> R {
    let [lo, hi, n] = a.bind(["lo", "hi", "n"])?;
    let (lo, hi) = (need(lo, "lo")?.num("lo")?, need(hi, "hi")?.num("hi")?);
    let n = n.map_or(Ok(50), |v| v.int("n"))?.max(0) as usize;
    if n == 1 {
        return Ok(vec_value(vec![lo]));
    }
    Ok(vec_value((0..n).map(|i| lo + (hi - lo) * i as f64 / (n - 1) as f64).collect()))
}
