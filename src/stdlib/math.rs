use crate::vm::*;
use statrs::function::{erf, gamma};

macro_rules! unary {
    ($($name:ident => $e:expr),* $(,)?) => {
        $(fn $name(_: &mut Vm, a: Args) -> R {
            let [x] = a.bind(["x"])?;
            let x = need(x, "x")?.num("x")?;
            let f: fn(f64) -> f64 = $e;
            Ok(Value::Float(f(x)))
        })*
    };
}

unary! {
    sqrt => f64::sqrt, exp => f64::exp, log2 => f64::log2, log10 => f64::log10, sin => f64::sin, cos => f64::cos,
    tan => f64::tan, asin => f64::asin, acos => f64::acos, atan => f64::atan, sinh => f64::sinh, cosh => f64::cosh,
    tanh => f64::tanh, erf_ => erf::erf, erfc => erf::erfc, gamma_ => gamma::gamma, lgamma => gamma::ln_gamma,
    expm1 => f64::exp_m1, log1p => f64::ln_1p,
}

pub static FNS: &[Native] = &[
    Native { name: "sqrt", f: sqrt },
    Native { name: "exp", f: exp },
    Native { name: "log", f: log },
    Native { name: "log2", f: log2 },
    Native { name: "log10", f: log10 },
    Native { name: "log1p", f: log1p },
    Native { name: "expm1", f: expm1 },
    Native { name: "sin", f: sin },
    Native { name: "cos", f: cos },
    Native { name: "tan", f: tan },
    Native { name: "asin", f: asin },
    Native { name: "acos", f: acos },
    Native { name: "atan", f: atan },
    Native { name: "atan2", f: atan2 },
    Native { name: "sinh", f: sinh },
    Native { name: "cosh", f: cosh },
    Native { name: "tanh", f: tanh },
    Native { name: "erf", f: erf_ },
    Native { name: "erfc", f: erfc },
    Native { name: "gamma", f: gamma_ },
    Native { name: "lgamma", f: lgamma },
    Native { name: "floor", f: floor },
    Native { name: "ceil", f: ceil },
    Native { name: "trunc", f: trunc },
    Native { name: "pow", f: pow },
    Native { name: "hypot", f: hypot },
    Native { name: "isnan", f: isnan },
    Native { name: "isinf", f: isinf },
    Native { name: "isfinite", f: isfinite },
    Native { name: "clamp", f: clamp },
    Native { name: "sign", f: sign },
    Native { name: "gcd", f: gcd },
    Native { name: "factorial", f: factorial },
    Native { name: "comb", f: comb },
];

pub fn consts() -> Vec<(&'static str, Value)> {
    use std::f64::consts;
    vec![
        ("pi", Value::Float(consts::PI)),
        ("e", Value::Float(consts::E)),
        ("tau", Value::Float(consts::TAU)),
        ("inf", Value::Float(f64::INFINITY)),
        ("nan", Value::Float(f64::NAN)),
    ]
}

fn num(v: Option<Value>, name: &str) -> Result<f64, Flow> {
    need(v, name)?.num(name)
}

fn log(_: &mut Vm, a: Args) -> R {
    let [x, base] = a.bind(["x", "base"])?;
    let x = num(x, "x")?;
    Ok(Value::Float(match base {
        Some(b) => x.ln() / b.num("base")?.ln(),
        None => x.ln(),
    }))
}

fn atan2(_: &mut Vm, a: Args) -> R {
    let [y, x] = a.bind(["y", "x"])?;
    Ok(Value::Float(num(y, "y")?.atan2(num(x, "x")?)))
}

fn pow(_: &mut Vm, a: Args) -> R {
    let [x, y] = a.bind(["x", "y"])?;
    Ok(Value::Float(num(x, "x")?.powf(num(y, "y")?)))
}

fn hypot(_: &mut Vm, a: Args) -> R {
    let [x, y] = a.bind(["x", "y"])?;
    Ok(Value::Float(num(x, "x")?.hypot(num(y, "y")?)))
}

// float to int, checked
fn to_i(x: f64) -> R {
    if !x.is_finite() || x.abs() >= 9.2e18 {
        return Err(value_err("number too big or not finite"));
    }
    Ok(Value::Int(x as i64))
}

fn floor(_: &mut Vm, a: Args) -> R {
    let [x] = a.bind(["x"])?;
    match need(x, "x")? {
        Value::Int(n) => Ok(Value::Int(n)),
        v => to_i(v.num("x")?.floor()),
    }
}

fn ceil(_: &mut Vm, a: Args) -> R {
    let [x] = a.bind(["x"])?;
    match need(x, "x")? {
        Value::Int(n) => Ok(Value::Int(n)),
        v => to_i(v.num("x")?.ceil()),
    }
}

fn trunc(_: &mut Vm, a: Args) -> R {
    let [x] = a.bind(["x"])?;
    match need(x, "x")? {
        Value::Int(n) => Ok(Value::Int(n)),
        v => to_i(v.num("x")?.trunc()),
    }
}

fn isnan(_: &mut Vm, a: Args) -> R {
    let [x] = a.bind(["x"])?;
    Ok(Value::Bool(num(x, "x")?.is_nan()))
}

fn isinf(_: &mut Vm, a: Args) -> R {
    let [x] = a.bind(["x"])?;
    Ok(Value::Bool(num(x, "x")?.is_infinite()))
}

fn isfinite(_: &mut Vm, a: Args) -> R {
    let [x] = a.bind(["x"])?;
    Ok(Value::Bool(num(x, "x")?.is_finite()))
}

fn clamp(_: &mut Vm, a: Args) -> R {
    let [x, lo, hi] = a.bind(["x", "lo", "hi"])?;
    let (x, lo, hi) = (need(x, "x")?, need(lo, "lo")?, need(hi, "hi")?);
    if compare(&lo, &hi)?.is_gt() {
        return Err(value_err("clamp() needs lo <= hi"));
    }
    Ok(if compare(&x, &lo)?.is_lt() {
        lo
    } else if compare(&x, &hi)?.is_gt() {
        hi
    } else {
        x
    })
}

fn sign(_: &mut Vm, a: Args) -> R {
    let [x] = a.bind(["x"])?;
    let x = num(x, "x")?;
    Ok(Value::Int(if x > 0.0 {
        1
    } else if x < 0.0 {
        -1
    } else {
        0
    }))
}

fn gcd(_: &mut Vm, a: Args) -> R {
    let [x, y] = a.bind(["a", "b"])?;
    let (mut x, mut y) = (need(x, "a")?.int("a")?.unsigned_abs(), need(y, "b")?.int("b")?.unsigned_abs());
    while y != 0 {
        (x, y) = (y, x % y);
    }
    Ok(Value::Int(x as i64))
}

fn factorial(_: &mut Vm, a: Args) -> R {
    let [n] = a.bind(["n"])?;
    let n = need(n, "n")?.int("n")?;
    if n < 0 {
        return Err(value_err("factorial() of negative number"));
    }
    let mut acc: i64 = 1;
    for i in 2..=n {
        acc = acc.checked_mul(i).ok_or_else(|| err("OverflowError", "factorial too big for int; use math.lgamma"))?;
    }
    Ok(Value::Int(acc))
}

fn comb(_: &mut Vm, a: Args) -> R {
    let [n, k] = a.bind(["n", "k"])?;
    let (n, k) = (need(n, "n")?.int("n")?, need(k, "k")?.int("k")?);
    if n < 0 || k < 0 {
        return Err(value_err("comb() needs non-negative numbers"));
    }
    if k > n {
        return Ok(Value::Int(0));
    }
    let k = k.min(n - k);
    let mut acc: i128 = 1;
    for i in 0..k {
        acc = acc * (n - i) as i128 / (i + 1) as i128;
        if acc > i64::MAX as i128 {
            return Err(err("OverflowError", "comb() result too big for int"));
        }
    }
    Ok(Value::Int(acc as i64))
}
