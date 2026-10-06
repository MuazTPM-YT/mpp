pub mod desc;
pub mod dist;
pub mod regress;
pub mod resample;
pub mod tests;

use crate::stdlib::table::Table;
use crate::stdlib::vec::vec_value;
use crate::vm::*;
use dist::Alt;
use resample::StatFn;
use tests::Res;

// ---- shared argument helpers (also used by ab, power, ml) ----

// numbers from list/vec/range, missing (NaN) dropped
pub fn nums(v: Option<Value>, name: &str) -> Result<Vec<f64>, Flow> {
    let v = need(v, name)?;
    let mut out = super::to_f64s(&v, name)?;
    out.retain(|x| !x.is_nan());
    Ok(out)
}

pub fn alt_arg(v: Option<Value>) -> Result<Alt, Flow> {
    match opt(v) {
        None => Ok(Alt::Two),
        Some(s) => {
            Alt::parse(s.as_str("alternative")?).ok_or_else(|| value_err("alternative must be \"two-sided\", \"less\" or \"greater\""))
        }
    }
}

pub fn level_arg(v: Option<Value>) -> Result<f64, Flow> {
    let l = opt(v).map_or(Ok(0.95), |v| v.num("level"))?;
    if !(0.0 < l && l < 1.0) {
        return Err(value_err("level must be between 0 and 1, like 0.95"));
    }
    Ok(l)
}

pub fn num_or(v: Option<Value>, name: &str, d: f64) -> Result<f64, Flow> {
    opt(v).map_or(Ok(d), |v| v.num(name))
}

pub fn done(r: Res) -> R {
    r.map(tests::Rec::value).map_err(value_err)
}

// groups: f(a, b, c) or f([a, b, c]) or f({"name": data})
pub fn groups(a: &Args) -> Result<Vec<Vec<f64>>, Flow> {
    let raw: Vec<Value> = match a.pos.as_slice() {
        [Value::List(l)] if l.borrow().iter().all(|x| !matches!(x, Value::Int(_) | Value::Float(_))) => l.borrow().clone(),
        [Value::Map(m)] => m.borrow().values().cloned().collect(),
        _ => a.pos.clone(),
    };
    raw.into_iter().map(|g| nums(Some(g), "group")).collect()
}

// 2D numbers from [[..], [..]]
pub fn matrix(v: Option<Value>, name: &str) -> Result<Vec<Vec<f64>>, Flow> {
    let v = need(v, name)?;
    super::to_vec(&v, name)?.iter().map(|row| super::to_f64s(row, name)).collect()
}

fn reps_arg(v: Option<Value>) -> Result<usize, Flow> {
    let n = opt(v).map_or(Ok(10_000), |v| v.int("n"))?;
    if !(1..=10_000_000).contains(&n) {
        return Err(value_err("n must be between 1 and 10000000"));
    }
    Ok(n as usize)
}

macro_rules! natives {
    ($($name:literal => $f:expr),* $(,)?) => {
        &[$(Native { name: $name, f: $f }),*]
    };
}

pub static FNS: &[Native] = natives![
    "mean" => |_, a| one(a, desc::mean),
    "median" => |_, a| one(a, desc::median),
    "sum" => |_, a| one(a, desc::sum),
    "min" => |_, a| one(a, desc::min),
    "max" => |_, a| one(a, desc::max),
    "skew" => |_, a| one(a, desc::skew),
    "kurtosis" => |_, a| one(a, desc::kurtosis),
    "sem" => |_, a| one(a, desc::sem),
    "var" => var,
    "std" => std_,
    "quantile" => quantile,
    "iqr" => |_, a| one(a, |x| desc::quantile(x, 0.75) - desc::quantile(x, 0.25)),
    "mad" => |_, a| one(a, |x| {
        let m = desc::median(x);
        desc::median(&x.iter().map(|v| (v - m).abs()).collect::<Vec<_>>())
    }),
    "zscore" => |_, a| {
        let [x] = a.bind(["x"])?;
        let x = nums(x, "x")?;
        let (m, s) = (desc::mean(&x), desc::std(&x, 0.0));
        Ok(vec_value(x.iter().map(|v| (v - m) / s).collect()))
    },
    "describe" => |_, a| {
        let [x] = a.bind(["x"])?;
        let v = need(x, "x")?;
        Ok(crate::stdlib::vec::describe(&super::to_f64s(&v, "x")?))
    },
    "ci_mean" => |_, a| {
        let [x, level] = a.bind(["x", "level"])?;
        let x = nums(x, "x")?;
        let r = tests::ttest_1samp(&x, 0.0, Alt::Two, level_arg(level)?).map_err(value_err)?;
        Ok(r.0.into_iter().find(|(k, _)| *k == "ci").map(|(_, v)| v).unwrap_or(Value::Nil))
    },
    "ttest" => ttest,
    "ttest_1samp" => |_, a| {
        let [x, mu, alt, level] = a.bind(["x", "mu", "alternative", "level"])?;
        done(tests::ttest_1samp(&nums(x, "x")?, num_or(mu, "mu", 0.0)?, alt_arg(alt)?, level_arg(level)?))
    },
    "ttest_ind" => ttest,
    "ttest_rel" => |_, a| {
        let [x, y, alt, level] = a.bind(["a", "b", "alternative", "level"])?;
        done(tests::ttest_rel(&paired(x, "a")?, &paired(y, "b")?, alt_arg(alt)?, level_arg(level)?))
    },
    "ztest" => |_, a| {
        let [x, mu, sigma, alt, level] = a.bind(["x", "mu", "sigma", "alternative", "level"])?;
        let sigma = opt(sigma).map(|s| s.num("sigma")).transpose()?;
        done(tests::ztest(&nums(x, "x")?, num_or(mu, "mu", 0.0)?, sigma, alt_arg(alt)?, level_arg(level)?))
    },
    "chi2" => |_, a| {
        let [t, corr] = a.bind(["table", "correction"])?;
        done(tests::chi2_contingency(&matrix(t, "table")?, corr.is_none_or(|c| c.truthy())))
    },
    "chi2_gof" => |_, a| {
        let [o, e] = a.bind(["observed", "expected"])?;
        let e = opt(e).map(|e| nums(Some(e), "expected")).transpose()?;
        done(tests::chi2_gof(&nums(o, "observed")?, e.as_deref()))
    },
    "fisher" => |_, a| {
        let [t, alt] = a.bind(["table", "alternative"])?;
        done(tests::fisher_exact(two_by_two(t)?, alt_arg(alt)?))
    },
    "mannwhitney" => |_, a| {
        let [x, y, alt, method] = a.bind(["a", "b", "alternative", "method"])?;
        let m = method_arg(method)?;
        done(tests::mannwhitneyu(&nums(x, "a")?, &nums(y, "b")?, alt_arg(alt)?, m.as_deref()))
    },
    "wilcoxon" => |_, a| {
        let [x, y, alt, method] = a.bind(["a", "b", "alternative", "method"])?;
        let m = method_arg(method)?;
        let y = opt(y).map(|y| paired(Some(y), "b")).transpose()?;
        let x = if y.is_some() { paired(x, "a")? } else { nums(x, "a")? };
        done(tests::wilcoxon(&x, y.as_deref(), alt_arg(alt)?, m.as_deref()))
    },
    "ks" => |_, a| {
        let [x, y, method] = a.bind(["a", "b", "method"])?;
        let m = method_arg(method)?;
        done(tests::ks_2samp(&nums(x, "a")?, &nums(y, "b")?, m.as_deref()))
    },
    "anova" => |_, a| done(tests::anova(&groups(&a)?)),
    "kruskal" => |_, a| done(tests::kruskal(&groups(&a)?)),
    "levene" => |_, mut a| {
        let center = match a.kw.iter().position(|(k, _)| &**k == "center") {
            Some(i) => a.kw.remove(i).1.as_str("center")?.to_string(),
            None => "median".into(),
        };
        a.no_kw()?;
        done(tests::levene(&groups(&a)?, &center))
    },
    "shapiro" => |_, a| {
        let [x] = a.bind(["x"])?;
        done(tests::shapiro(&nums(x, "x")?))
    },
    "pearson" => |_, a| {
        let [x, y, alt, level] = a.bind(["x", "y", "alternative", "level"])?;
        done(tests::pearson(&paired(x, "x")?, &paired(y, "y")?, alt_arg(alt)?, level_arg(level)?))
    },
    "spearman" => |_, a| {
        let [x, y, alt] = a.bind(["x", "y", "alternative"])?;
        done(tests::spearman(&paired(x, "x")?, &paired(y, "y")?, alt_arg(alt)?))
    },
    "kendall" => |_, a| {
        let [x, y] = a.bind(["x", "y"])?;
        done(tests::kendall(&paired(x, "x")?, &paired(y, "y")?))
    },
    "linregress" => |_, a| {
        let [x, y] = a.bind(["x", "y"])?;
        done(regress::linregress(&paired(x, "x")?, &paired(y, "y")?))
    },
    "ols" => ols,
    "cohens_d" => |_, a| two(a, tests::cohens_d),
    "hedges_g" => |_, a| two(a, tests::hedges_g),
    "cliffs_delta" => |_, a| two(a, tests::cliffs_delta),
    "odds_ratio" => |_, a| {
        let [t, level] = a.bind(["table", "level"])?;
        let [[x, y], [z, w]] = two_by_two(t)?;
        let level = level_arg(level)?;
        // Haldane +0.5 when a cell is zero
        let (x, y, z, w) = if x * y * z * w == 0.0 { (x + 0.5, y + 0.5, z + 0.5, w + 0.5) } else { (x, y, z, w) };
        let or = x * w / (y * z);
        let se = (1.0 / x + 1.0 / y + 1.0 / z + 1.0 / w).sqrt();
        let q = dist::norm_ppf(0.5 + level / 2.0);
        Ok(tests::Rec::default().num("odds_ratio", or).pair("ci", (or.ln() - q * se).exp(), (or.ln() + q * se).exp()).value())
    },
    "relative_risk" => |_, a| {
        let [t, level] = a.bind(["table", "level"])?;
        let [[x, y], [z, w]] = two_by_two(t)?;
        let level = level_arg(level)?;
        let (r1, r2) = (x / (x + y), z / (z + w));
        let rr = r1 / r2;
        let se = (1.0 / x - 1.0 / (x + y) + 1.0 / z - 1.0 / (z + w)).sqrt();
        let q = dist::norm_ppf(0.5 + level / 2.0);
        Ok(tests::Rec::default().num("relative_risk", rr).pair("ci", (rr.ln() - q * se).exp(), (rr.ln() + q * se).exp()).num("risk_a", r1).num("risk_b", r2).value())
    },
    "bootstrap" => |vm, a| {
        let [x, f, n, level, method] = a.bind(["x", "fn", "n", "level", "method"])?;
        let x = nums(x, "x")?;
        let f = StatFn::from_value(f)?;
        let method = opt(method).map_or(Ok("bca".to_string()), |m| Ok::<_, Flow>(m.as_str("method")?.to_string()))?;
        resample::bootstrap(vm, &x, &f, reps_arg(n)?, level_arg(level)?, &method).map(tests::Rec::value)
    },
    "bootstrap_diff" => |vm, a| {
        let [x, y, f, n, level] = a.bind(["a", "b", "fn", "n", "level"])?;
        let f = StatFn::from_value(f)?;
        resample::bootstrap_diff(vm, &nums(x, "a")?, &nums(y, "b")?, &f, reps_arg(n)?, level_arg(level)?).map(tests::Rec::value)
    },
    "permutation_test" => |vm, a| {
        let [x, y, f, n, alt] = a.bind(["a", "b", "fn", "n", "alternative"])?;
        let f = StatFn::from_value(f)?;
        resample::permutation_test(vm, &nums(x, "a")?, &nums(y, "b")?, &f, reps_arg(n)?, alt_arg(alt)?).map(tests::Rec::value)
    },
    "adjust" => |_, a| {
        let [p, method] = a.bind(["p_values", "method"])?;
        let m = opt(method).map_or(Ok("holm".to_string()), |m| Ok::<_, Flow>(m.as_str("method")?.to_string()))?;
        let out = tests::adjust(&nums(p, "p_values")?, &m).map_err(value_err)?;
        Ok(Value::list(out.into_iter().map(Value::Float).collect()))
    },
    "norm_cdf" => |_, a| {
        let [x, m, s] = a.bind(["x", "mean", "sd"])?;
        let (m, s) = (num_or(m, "mean", 0.0)?, num_or(s, "sd", 1.0)?);
        Ok(Value::Float(dist::norm_cdf((need(x, "x")?.num("x")? - m) / s)))
    },
    "norm_ppf" => |_, a| {
        let [p, m, s] = a.bind(["p", "mean", "sd"])?;
        let (m, s) = (num_or(m, "mean", 0.0)?, num_or(s, "sd", 1.0)?);
        Ok(Value::Float(m + s * dist::norm_ppf(need(p, "p")?.num("p")?)))
    },
    "norm_pdf" => |_, a| {
        let [x, m, s] = a.bind(["x", "mean", "sd"])?;
        let (m, s) = (num_or(m, "mean", 0.0)?, num_or(s, "sd", 1.0)?);
        Ok(Value::Float(dist::norm_pdf((need(x, "x")?.num("x")? - m) / s) / s))
    },
    "t_cdf" => |_, a| f2(a, ["x", "df"], dist::t_cdf),
    "t_ppf" => |_, a| f2(a, ["p", "df"], dist::t_ppf),
    "chi2_cdf" => |_, a| f2(a, ["x", "df"], dist::chi2_cdf),
    "chi2_ppf" => |_, a| f2(a, ["p", "df"], dist::chi2_ppf),
    "f_sf" => |_, a| f3(a, ["x", "d1", "d2"], dist::f_sf),
    "beta_cdf" => |_, a| f3(a, ["x", "a", "b"], dist::beta_cdf),
    "beta_ppf" => |_, a| f3(a, ["p", "a", "b"], dist::beta_ppf),
    "binom_pmf" => |_, a| f3(a, ["k", "n", "p"], dist::binom_pmf),
    "binom_cdf" => |_, a| f3(a, ["k", "n", "p"], dist::binom_cdf),
    "poisson_pmf" => |_, a| f2(a, ["k", "lam"], dist::poisson_pmf),
    "poisson_cdf" => |_, a| f2(a, ["k", "lam"], dist::poisson_cdf),
];

fn one(a: Args, f: fn(&[f64]) -> f64) -> R {
    let [x] = a.bind(["x"])?;
    Ok(Value::Float(f(&nums(x, "x")?)))
}

fn two(a: Args, f: fn(&[f64], &[f64]) -> f64) -> R {
    let [x, y] = a.bind(["a", "b"])?;
    Ok(Value::Float(f(&nums(x, "a")?, &nums(y, "b")?)))
}

fn f2(a: Args, names: [&str; 2], f: fn(f64, f64) -> f64) -> R {
    let [x, y] = a.bind(names)?;
    Ok(Value::Float(f(need(x, names[0])?.num(names[0])?, need(y, names[1])?.num(names[1])?)))
}

fn f3(a: Args, names: [&str; 3], f: fn(f64, f64, f64) -> f64) -> R {
    let [x, y, z] = a.bind(names)?;
    Ok(Value::Float(f(need(x, names[0])?.num(names[0])?, need(y, names[1])?.num(names[1])?, need(z, names[2])?.num(names[2])?)))
}

// paired data keeps positions (no NaN dropping)
fn paired(v: Option<Value>, name: &str) -> Result<Vec<f64>, Flow> {
    super::to_f64s(&need(v, name)?, name)
}

fn method_arg(v: Option<Value>) -> Result<Option<String>, Flow> {
    match opt(v) {
        None => Ok(None),
        Some(m) => {
            let m = m.as_str("method")?.to_string();
            if m != "exact" && m != "asymptotic" {
                return Err(value_err("method must be \"exact\" or \"asymptotic\""));
            }
            Ok(Some(m))
        }
    }
}

fn two_by_two(t: Option<Value>) -> Result<[[f64; 2]; 2], Flow> {
    let m = matrix(t, "table")?;
    if m.len() != 2 || m.iter().any(|r| r.len() != 2) {
        return Err(value_err("needs a 2x2 table like [[a, b], [c, d]]"));
    }
    Ok([[m[0][0], m[0][1]], [m[1][0], m[1][1]]])
}

fn var(_: &mut Vm, a: Args) -> R {
    let [x, ddof] = a.bind(["x", "ddof"])?;
    Ok(Value::Float(desc::var(&nums(x, "x")?, num_or(ddof, "ddof", 1.0)?)))
}

fn std_(_: &mut Vm, a: Args) -> R {
    let [x, ddof] = a.bind(["x", "ddof"])?;
    Ok(Value::Float(desc::std(&nums(x, "x")?, num_or(ddof, "ddof", 1.0)?)))
}

fn quantile(_: &mut Vm, a: Args) -> R {
    let [x, q] = a.bind(["x", "q"])?;
    let s = desc::sorted(&nums(x, "x")?);
    let q = need(q, "q")?;
    match &q {
        Value::Int(_) | Value::Float(_) => Ok(Value::Float(desc::quantile_sorted(&s, q.num("q")?))),
        other => Ok(vec_value(super::to_f64s(other, "q")?.iter().map(|p| desc::quantile_sorted(&s, *p)).collect())),
    }
}

fn ttest(_: &mut Vm, a: Args) -> R {
    let [x, y, eq, alt, level] = a.bind(["a", "b", "equal_var", "alternative", "level"])?;
    done(tests::ttest_ind(&nums(x, "a")?, &nums(y, "b")?, eq.is_some_and(|e| e.truthy()), alt_arg(alt)?, level_arg(level)?))
}

// ols(table, "y ~ a + b") or ols(y, [x1, x2], names = [...])
fn ols(_: &mut Vm, a: Args) -> R {
    let [first, second, names, intercept, level] = a.bind(["data", "formula", "names", "intercept", "level"])?;
    let first = need(first, "data")?;
    let second = need(second, "formula")?;
    let intercept = intercept.is_none_or(|v| v.truthy());
    let level = level_arg(level)?;
    let (y, cols, labels) = if let Some(t) = first.object::<Table>() {
        let f = second.as_str("formula")?;
        let (lhs, rhs) = f.split_once('~').ok_or_else(|| value_err("formula looks like \"y ~ x1 + x2\""))?;
        let xs: Vec<String> = rhs.split('+').map(|s| s.trim().to_string()).filter(|s| !s.is_empty() && s != "1").collect();
        let ycol = t.col(lhs.trim())?;
        let ycol = match ycol {
            crate::stdlib::table::Col::Num(v, _) => v.to_vec(),
            _ => return Err(type_err("y column must be numbers")),
        };
        let mut cols = Vec::new();
        for x in &xs {
            match t.col(x)? {
                crate::stdlib::table::Col::Num(v, _) => cols.push(v.to_vec()),
                _ => return Err(type_err(format!("column `{x}` must be numbers"))),
            }
        }
        // drop rows with any missing value
        let keep: Vec<usize> = (0..ycol.len()).filter(|&i| !ycol[i].is_nan() && cols.iter().all(|c| !c[i].is_nan())).collect();
        let y: Vec<f64> = keep.iter().map(|&i| ycol[i]).collect();
        let cols: Vec<Vec<f64>> = cols.iter().map(|c| keep.iter().map(|&i| c[i]).collect()).collect();
        (y, cols, xs)
    } else {
        let y = super::to_f64s(&first, "y")?;
        let raw = super::to_vec(&second, "x columns")?;
        let cols: Vec<Vec<f64>> = if raw.iter().all(|v| matches!(v, Value::Int(_) | Value::Float(_))) {
            vec![super::to_f64s(&second, "x")?]
        } else {
            raw.iter().map(|c| super::to_f64s(c, "x column")).collect::<Result<_, _>>()?
        };
        let labels = match opt(names) {
            Some(n) => super::to_vec(&n, "names")?.iter().map(|v| Ok(v.as_str("name")?.to_string())).collect::<Result<_, Flow>>()?,
            None => (1..=cols.len()).map(|i| format!("x{i}")).collect(),
        };
        (y, cols, labels)
    };
    done(regress::ols(&y, &cols, &labels, intercept, level))
}
