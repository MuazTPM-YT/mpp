// bootstrap and permutation tests; statistic is a built-in name or a function
use super::desc;
use super::dist::{norm_cdf, norm_ppf};
use super::tests::Rec;
use crate::stdlib::vec::vec_value;
use crate::vm::*;

pub enum StatFn {
    Fast(fn(&[f64]) -> f64),
    Call(Value),
}

impl StatFn {
    pub fn from_value(v: Option<Value>) -> Result<StatFn, Flow> {
        let Some(v) = v else { return Ok(StatFn::Fast(desc::mean)) };
        if let Value::Str(s) = &v {
            return Ok(StatFn::Fast(match &**s {
                "mean" => desc::mean,
                "median" => desc::median,
                "sum" => desc::sum,
                "std" => |x| desc::std(x, 1.0),
                "var" => |x| desc::var(x, 1.0),
                "min" => desc::min,
                "max" => desc::max,
                other => {
                    return Err(value_err(format!(
                        "unknown statistic {other:?} (use mean, median, sum, std, var, min, max, or a function)"
                    )));
                }
            }));
        }
        if v.is_callable() { Ok(StatFn::Call(v)) } else { Err(type_err("statistic must be a name like \"mean\" or a function")) }
    }

    pub fn eval(&self, vm: &mut Vm, x: &[f64]) -> Result<f64, Flow> {
        match self {
            StatFn::Fast(f) => Ok(f(x)),
            StatFn::Call(f) => vm.call(f, &[vec_value(x.to_vec())])?.num("statistic result"),
        }
    }
}

fn resample(vm: &mut Vm, x: &[f64], buf: &mut Vec<f64>) {
    buf.clear();
    let n = x.len() as u64;
    buf.extend((0..x.len()).map(|_| x[vm.rng.below(n) as usize]));
}

fn pct(s: &[f64], q: f64) -> f64 {
    desc::quantile_sorted(s, q.clamp(0.0, 1.0))
}

pub fn bootstrap(vm: &mut Vm, x: &[f64], f: &StatFn, reps: usize, level: f64, method: &str) -> Result<Rec, Flow> {
    if x.len() < 2 {
        return Err(value_err("bootstrap needs at least 2 values"));
    }
    let est = f.eval(vm, x)?;
    let mut buf = Vec::with_capacity(x.len());
    let mut boots = Vec::with_capacity(reps);
    for _ in 0..reps {
        resample(vm, x, &mut buf);
        boots.push(f.eval(vm, &buf)?);
    }
    let se = desc::std(&boots, 1.0);
    let s = desc::sorted(&boots);
    let alpha = 1.0 - level;
    let (lo, hi) = match method {
        "percentile" => (pct(&s, alpha / 2.0), pct(&s, 1.0 - alpha / 2.0)),
        "bca" => {
            let below = boots.iter().filter(|b| **b < est).count() as f64 / reps as f64;
            let z0 = norm_ppf(below.clamp(1e-10, 1.0 - 1e-10));
            // jackknife for acceleration
            let n = x.len();
            let mut jack = Vec::with_capacity(n);
            let mut loo = Vec::with_capacity(n - 1);
            for i in 0..n {
                loo.clear();
                loo.extend(x[..i].iter().chain(&x[i + 1..]));
                jack.push(f.eval(vm, &loo)?);
            }
            let jm = desc::mean(&jack);
            let num: f64 = jack.iter().map(|j| (jm - j).powi(3)).sum();
            let den: f64 = jack.iter().map(|j| (jm - j).powi(2)).sum();
            let acc = if den == 0.0 { 0.0 } else { num / (6.0 * den.powf(1.5)) };
            let adj = |z: f64| norm_cdf(z0 + (z0 + z) / (1.0 - acc * (z0 + z)));
            (pct(&s, adj(norm_ppf(alpha / 2.0))), pct(&s, adj(norm_ppf(1.0 - alpha / 2.0))))
        }
        other => return Err(value_err(format!("unknown method {other:?} (use \"bca\" or \"percentile\")"))),
    };
    Ok(Rec::default()
        .text("method", &format!("bootstrap ({method})"))
        .num("estimate", est)
        .pair("ci", lo, hi)
        .num("se", se)
        .int("reps", reps as i64)
        .num("level", level))
}

// f(b) - f(a), each resampled on its own
pub fn bootstrap_diff(vm: &mut Vm, a: &[f64], b: &[f64], f: &StatFn, reps: usize, level: f64) -> Result<Rec, Flow> {
    if a.len() < 2 || b.len() < 2 {
        return Err(value_err("bootstrap_diff needs at least 2 values in each sample"));
    }
    let est = f.eval(vm, b)? - f.eval(vm, a)?;
    let (mut ba, mut bb) = (Vec::new(), Vec::new());
    let mut diffs = Vec::with_capacity(reps);
    for _ in 0..reps {
        resample(vm, a, &mut ba);
        resample(vm, b, &mut bb);
        diffs.push(f.eval(vm, &bb)? - f.eval(vm, &ba)?);
    }
    let s = desc::sorted(&diffs);
    let alpha = 1.0 - level;
    let le = diffs.iter().filter(|d| **d <= 0.0).count() as f64 / reps as f64;
    let ge = diffs.iter().filter(|d| **d >= 0.0).count() as f64 / reps as f64;
    Ok(Rec::default()
        .text("method", "bootstrap difference (percentile)")
        .num("estimate", est)
        .pair("ci", pct(&s, alpha / 2.0), pct(&s, 1.0 - alpha / 2.0))
        .num("se", desc::std(&diffs, 1.0))
        .num("p_value", (2.0 * le.min(ge)).min(1.0))
        .int("reps", reps as i64))
}

// statistic = f(a) - f(b); labels shuffled `reps` times
pub fn permutation_test(vm: &mut Vm, a: &[f64], b: &[f64], f: &StatFn, reps: usize, alt: super::dist::Alt) -> Result<Rec, Flow> {
    if a.is_empty() || b.is_empty() {
        return Err(value_err("permutation_test needs two non-empty samples"));
    }
    let obs = f.eval(vm, a)? - f.eval(vm, b)?;
    let mut pool: Vec<f64> = a.iter().chain(b).copied().collect();
    let na = a.len();
    let mut hits = 0usize;
    for _ in 0..reps {
        vm.rng.shuffle(&mut pool);
        let s = f.eval(vm, &pool[..na])? - f.eval(vm, &pool[na..])?;
        let eps = 1e-12 * obs.abs().max(1.0);
        hits += match alt {
            super::dist::Alt::Two => s.abs() >= obs.abs() - eps,
            super::dist::Alt::Greater => s >= obs - eps,
            super::dist::Alt::Less => s <= obs + eps,
        } as usize;
    }
    Ok(Rec::default()
        .text("method", "permutation test")
        .num("statistic", obs)
        .num("p_value", (hits + 1) as f64 / (reps + 1) as f64)
        .int("reps", reps as i64)
        .text("alternative", alt.name()))
}
