// A/B testing kit: frequentist, bayesian, sequential, variance reduction, checks
use super::stats::desc::{self, mean, var};
use super::stats::dist::*;
use super::stats::tests::{self as st, Rec};
use super::stats::{alt_arg, num_or, nums};
use super::table::{Col, Table};
use crate::vm::*;
use indexmap::IndexMap;

macro_rules! natives {
    ($($name:literal => $f:expr),* $(,)?) => {
        &[$(Native { name: $name, f: $f }),*]
    };
}

pub static FNS: &[Native] = natives![
    "proportions" => proportions,
    "means" => means,
    "ratio" => ratio,
    "cuped" => cuped,
    "srm" => srm,
    "aa" => aa,
    "multi" => multi,
    "bayes" => bayes,
    "bayes_means" => bayes_means,
    "msprt" => msprt,
    "sequential_bounds" => sequential_bounds,
    "segments" => segments,
    "novelty" => novelty,
    "guardrail" => guardrail,
];

fn alpha_arg(v: Option<Value>) -> Result<f64, Flow> {
    let a = num_or(v, "alpha", 0.05)?;
    if !(0.0 < a && a < 1.0) {
        return Err(value_err("alpha must be between 0 and 1, like 0.05"));
    }
    Ok(a)
}

// (successes, trials) from 0/1 data
fn counts(x: &[f64]) -> (f64, f64) {
    (x.iter().sum(), x.len() as f64)
}

fn z_crit(alpha: f64, alt: Alt) -> f64 {
    if alt == Alt::Two { norm_ppf(1.0 - alpha / 2.0) } else { norm_ppf(1.0 - alpha) }
}

fn ci(est: f64, se: f64, q: f64, alt: Alt) -> (f64, f64) {
    match alt {
        Alt::Two => (est - q * se, est + q * se),
        Alt::Greater => (est - q * se, f64::INFINITY),
        Alt::Less => (f64::NEG_INFINITY, est + q * se),
    }
}

// two-proportion z test (pooled test, unpooled interval); lift CI by delta method on log ratio
pub fn prop_test(xa: f64, na: f64, xb: f64, nb: f64, alpha: f64, alt: Alt) -> Result<Rec, String> {
    if na <= 0.0 || nb <= 0.0 {
        return Err("each group needs at least one trial".into());
    }
    if xa < 0.0 || xb < 0.0 || xa > na || xb > nb {
        return Err("successes must be between 0 and trials".into());
    }
    let (pa, pb) = (xa / na, xb / nb);
    let pool = (xa + xb) / (na + nb);
    let se0 = (pool * (1.0 - pool) * (1.0 / na + 1.0 / nb)).sqrt();
    let diff = pb - pa;
    let z = if se0 == 0.0 { 0.0 } else { diff / se0 };
    let p = if se0 == 0.0 { 1.0 } else { p_from_z(z, alt) };
    let se = (pa * (1.0 - pa) / na + pb * (1.0 - pb) / nb).sqrt();
    let q = z_crit(alpha, alt);
    let (lo, hi) = ci(diff, se, q, alt);
    let lift = if pa > 0.0 { pb / pa - 1.0 } else { f64::NAN };
    let (llo, lhi) = if pa > 0.0 && pb > 0.0 {
        let sl = ((1.0 - pa) / (na * pa) + (1.0 - pb) / (nb * pb)).sqrt();
        let (a, b) = ci((pb / pa).ln(), sl, q, alt);
        (a.exp() - 1.0, b.exp() - 1.0)
    } else {
        (f64::NAN, f64::NAN)
    };
    Ok(Rec::default()
        .text("method", "two-proportion z-test")
        .num("rate_a", pa)
        .num("rate_b", pb)
        .num("diff", diff)
        .pair("ci_diff", lo, hi)
        .num("lift", lift)
        .pair("ci_lift", llo, lhi)
        .num("z", z)
        .num("p_value", p)
        .flag("significant", p < alpha)
        .int("n_a", na as i64)
        .int("n_b", nb as i64)
        .num("alpha", alpha)
        .text("alternative", alt.name()))
}

// welch test on means, with relative lift (delta method)
pub fn mean_test(a: &[f64], b: &[f64], alpha: f64, alt: Alt) -> Result<Rec, String> {
    if a.len() < 2 || b.len() < 2 {
        return Err("each group needs at least 2 values".into());
    }
    let (na, nb) = (a.len() as f64, b.len() as f64);
    let (ma, mb) = (mean(a), mean(b));
    let (va, vb) = (var(a, 1.0) / na, var(b, 1.0) / nb);
    let se = (va + vb).sqrt();
    let df = (va + vb).powi(2) / (va * va / (na - 1.0) + vb * vb / (nb - 1.0));
    let diff = mb - ma;
    let t = diff / se;
    let p = p_from_t(t, df, alt);
    let q = if alt == Alt::Two { t_ppf(1.0 - alpha / 2.0, df) } else { t_ppf(1.0 - alpha, df) };
    let (lo, hi) = ci(diff, se, q, alt);
    let lift = mb / ma - 1.0;
    // var of mb/ma by delta method
    let sl = ((vb / (ma * ma)) + (mb * mb * va / ma.powi(4))).sqrt();
    let (llo, lhi) = ci(lift, sl, q, alt);
    Ok(Rec::default()
        .text("method", "Welch t-test on means")
        .num("mean_a", ma)
        .num("mean_b", mb)
        .num("diff", diff)
        .pair("ci_diff", lo, hi)
        .num("lift", lift)
        .pair("ci_lift", llo, lhi)
        .num("t", t)
        .num("df", df)
        .num("p_value", p)
        .flag("significant", p < alpha)
        .num("cohens_d", -st::cohens_d(a, b))
        .int("n_a", na as i64)
        .int("n_b", nb as i64)
        .num("alpha", alpha)
        .text("alternative", alt.name()))
}

fn rec(r: Result<Rec, String>) -> R {
    r.map(Rec::value).map_err(value_err)
}

// proportions(a, b) with 0/1 data, or proportions(x_a=, n_a=, x_b=, n_b=)
fn proportions(_: &mut Vm, a: Args) -> R {
    let [da, db, xa, na, xb, nb, alpha, alt] = a.bind(["a", "b", "x_a", "n_a", "x_b", "n_b", "alpha", "alternative"])?;
    let (alpha, alt) = (alpha_arg(alpha)?, alt_arg(alt)?);
    let (xa, na, xb, nb) = match (da, db) {
        (Some(da), Some(db)) => {
            let (ca, cb) = (nums(Some(da), "a")?, nums(Some(db), "b")?);
            if ca.iter().chain(&cb).any(|v| *v != 0.0 && *v != 1.0) {
                return Err(value_err("proportions() data must be 0/1 (or true/false); for other numbers use ab.means"));
            }
            let ((xa, na), (xb, nb)) = (counts(&ca), counts(&cb));
            (xa, na, xb, nb)
        }
        _ => (need(xa, "x_a")?.num("x_a")?, need(na, "n_a")?.num("n_a")?, need(xb, "x_b")?.num("x_b")?, need(nb, "n_b")?.num("n_b")?),
    };
    rec(prop_test(xa, na, xb, nb, alpha, alt))
}

fn means(_: &mut Vm, a: Args) -> R {
    let [x, y, alpha, alt] = a.bind(["a", "b", "alpha", "alternative"])?;
    rec(mean_test(&nums(x, "a")?, &nums(y, "b")?, alpha_arg(alpha)?, alt_arg(alt)?))
}

// ratio metric sum(num)/sum(den) per group, delta-method variance
fn ratio_stats(num: &[f64], den: &[f64]) -> Result<(f64, f64), String> {
    if num.len() != den.len() || num.len() < 2 {
        return Err("numerator and denominator need the same length (one per unit), at least 2".into());
    }
    let n = num.len() as f64;
    let (mn, md) = (mean(num), mean(den));
    if md == 0.0 {
        return Err("denominator mean is zero".into());
    }
    let r = mn / md;
    let vn = var(num, 1.0);
    let vd = var(den, 1.0);
    let c = desc::cov(num, den);
    let v = (vn - 2.0 * r * c + r * r * vd) / (n * md * md);
    Ok((r, v))
}

fn ratio(_: &mut Vm, a: Args) -> R {
    let [na, da, nb, db, alpha, alt] = a.bind(["num_a", "den_a", "num_b", "den_b", "alpha", "alternative"])?;
    let (alpha, alt) = (alpha_arg(alpha)?, alt_arg(alt)?);
    let (ra, va) = ratio_stats(&nums(na, "num_a")?, &nums(da, "den_a")?).map_err(value_err)?;
    let (rb, vb) = ratio_stats(&nums(nb, "num_b")?, &nums(db, "den_b")?).map_err(value_err)?;
    let diff = rb - ra;
    let se = (va + vb).sqrt();
    let z = diff / se;
    let p = p_from_z(z, alt);
    let (lo, hi) = ci(diff, se, z_crit(alpha, alt), alt);
    Ok(Rec::default()
        .text("method", "ratio metric z-test (delta method)")
        .num("ratio_a", ra)
        .num("ratio_b", rb)
        .num("diff", diff)
        .pair("ci_diff", lo, hi)
        .num("lift", rb / ra - 1.0)
        .num("z", z)
        .num("p_value", p)
        .flag("significant", p < alpha)
        .value())
}

// CUPED: remove what a pre-period covariate explains, then test
fn cuped(_: &mut Vm, a: Args) -> R {
    let [ya, xa, yb, xb, alpha, alt] = a.bind(["y_a", "x_a", "y_b", "x_b", "alpha", "alternative"])?;
    let (ya, xa, yb, xb) = (nums(ya, "y_a")?, nums(xa, "x_a")?, nums(yb, "y_b")?, nums(xb, "x_b")?);
    if ya.len() != xa.len() || yb.len() != xb.len() {
        return Err(value_err("each y needs a matching x (same length)"));
    }
    let y: Vec<f64> = ya.iter().chain(&yb).copied().collect();
    let x: Vec<f64> = xa.iter().chain(&xb).copied().collect();
    let vx = var(&x, 1.0);
    if vx == 0.0 {
        return Err(value_err("covariate x has no variation"));
    }
    let theta = desc::cov(&y, &x) / vx;
    let mx = mean(&x);
    let adj = |ys: &[f64], xs: &[f64]| -> Vec<f64> { ys.iter().zip(xs).map(|(y, x)| y - theta * (x - mx)).collect() };
    let (aa, ab) = (adj(&ya, &xa), adj(&yb, &xb));
    let mut r = mean_test(&aa, &ab, alpha_arg(alpha)?, alt_arg(alt)?).map_err(value_err)?;
    let raw = mean_test(&ya, &yb, 0.05, Alt::Two).map_err(value_err)?;
    let before = var(&ya, 1.0) / ya.len() as f64 + var(&yb, 1.0) / yb.len() as f64;
    let after = var(&aa, 1.0) / aa.len() as f64 + var(&ab, 1.0) / ab.len() as f64;
    r.0[0].1 = Value::str("CUPED-adjusted Welch t-test");
    Ok(r.num("theta", theta).num("variance_reduction", 1.0 - after / before).num("p_value_unadjusted", raw.get("p_value")).value())
}

// sample ratio mismatch: did traffic split as planned?
fn srm(_: &mut Vm, a: Args) -> R {
    let [c, e, threshold] = a.bind(["counts", "expected", "threshold"])?;
    let obs = nums(c, "counts")?;
    let exp = opt(e).map(|e| nums(Some(e), "expected")).transpose()?;
    let thr = num_or(threshold, "threshold", 0.001)?;
    let r = st::chi2_gof(&obs, exp.as_deref()).map_err(value_err)?;
    let total: f64 = obs.iter().sum();
    let shares: Vec<Value> = obs.iter().map(|o| Value::Float(o / total)).collect();
    let p = r.get("p_value");
    Ok(Rec::default()
        .text("method", "sample ratio mismatch (chi-square)")
        .num("statistic", r.get("statistic"))
        .num("p_value", p)
        .flag("mismatch", p < thr)
        .val("observed_share", Value::list(shares))
        .num("threshold", thr)
        .value())
}

// A/A check: random halves of one group should rarely look different
fn aa(vm: &mut Vm, a: Args) -> R {
    let [x, n, alpha, metric] = a.bind(["data", "n", "alpha", "metric"])?;
    let x = nums(x, "data")?;
    if x.len() < 4 {
        return Err(value_err("A/A check needs at least 4 values"));
    }
    let reps = opt(n).map_or(Ok(1000), |v| v.int("n"))?.clamp(10, 1_000_000) as usize;
    let alpha = alpha_arg(alpha)?;
    let binary = x.iter().all(|v| *v == 0.0 || *v == 1.0);
    let metric = opt(metric).map_or(Ok(if binary { "proportion".to_string() } else { "mean".to_string() }), |m| {
        Ok::<_, Flow>(m.as_str("metric")?.to_string())
    })?;
    let mut pool = x.clone();
    let half = pool.len() / 2;
    let mut hits = 0;
    let mut ps = Vec::with_capacity(reps);
    for _ in 0..reps {
        vm.rng.shuffle(&mut pool);
        let (l, r) = pool.split_at(half);
        let p = if metric == "proportion" {
            let ((xa, na), (xb, nb)) = (counts(l), counts(r));
            prop_test(xa, na, xb, nb, alpha, Alt::Two).map_err(value_err)?.get("p_value")
        } else {
            mean_test(l, r, alpha, Alt::Two).map_err(value_err)?.get("p_value")
        };
        hits += (p < alpha) as usize;
        ps.push(p);
    }
    let fpr = hits as f64 / reps as f64;
    // binomial band around alpha
    let band = 3.0 * (alpha * (1.0 - alpha) / reps as f64).sqrt();
    let ks =
        st::ks_2samp(&ps, &(0..reps).map(|i| (i as f64 + 0.5) / reps as f64).collect::<Vec<_>>(), Some("asymptotic")).map_err(value_err)?;
    Ok(Rec::default()
        .text("method", &format!("A/A simulation ({metric})"))
        .num("false_positive_rate", fpr)
        .num("expected", alpha)
        .flag("ok", (fpr - alpha).abs() <= band)
        .num("p_uniform", ks.get("p_value"))
        .int("runs", reps as i64)
        .value())
}

// many variants vs one control, with multiple-testing correction
fn multi(_: &mut Vm, a: Args) -> R {
    let [g, control, metric, correction, alpha] = a.bind(["groups", "control", "metric", "correction", "alpha"])?;
    let g = need(g, "groups")?;
    let Value::Map(m) = &g else { return Err(type_err("groups must be a map like {\"A\": data, \"B\": data}")) };
    let groups: Vec<(Key, Vec<f64>)> =
        m.borrow().iter().map(|(k, v)| Ok((k.clone(), nums(Some(v.clone()), "group")?))).collect::<Result<_, Flow>>()?;
    if groups.len() < 2 {
        return Err(value_err("need a control and at least one variant"));
    }
    let alpha = alpha_arg(alpha)?;
    let ctrl_key = match opt(control) {
        Some(c) => Key::from(&c)?,
        None => groups[0].0.clone(),
    };
    let ctrl = groups.iter().find(|(k, _)| *k == ctrl_key).ok_or_else(|| value_err("control group not found"))?.1.clone();
    let binary = groups.iter().all(|(_, v)| v.iter().all(|x| *x == 0.0 || *x == 1.0));
    let metric = opt(metric).map_or(Ok(if binary { "proportion".to_string() } else { "mean".to_string() }), |m| {
        Ok::<_, Flow>(m.as_str("metric")?.to_string())
    })?;
    let corr = opt(correction).map_or(Ok("holm".to_string()), |c| Ok::<_, Flow>(c.as_str("correction")?.to_string()))?;
    let mut recs = Vec::new();
    for (k, v) in &groups {
        if *k == ctrl_key {
            continue;
        }
        let r = if metric == "proportion" {
            let ((xa, na), (xb, nb)) = (counts(&ctrl), counts(v));
            prop_test(xa, na, xb, nb, alpha, Alt::Two)
        } else {
            mean_test(&ctrl, v, alpha, Alt::Two)
        }
        .map_err(value_err)?;
        recs.push((k.clone(), r));
    }
    let ps: Vec<f64> = recs.iter().map(|(_, r)| r.get("p_value")).collect();
    let adj = st::adjust(&ps, &corr).map_err(value_err)?;
    let mut out = IndexMap::new();
    for ((k, r), p) in recs.into_iter().zip(adj) {
        let mut r = r.num("p_adjusted", p);
        for (name, v) in r.0.iter_mut() {
            if *name == "significant" {
                *v = Value::Bool(p < alpha);
            }
        }
        out.insert(k, r.text("correction", &corr).value());
    }
    Ok(Value::map(out))
}

// exact P(B > A) for beta posteriors with integer-ish params (Evan Miller)
fn prob_b_beats_a(aa: f64, ba: f64, ab: f64, bb: f64) -> f64 {
    use statrs::function::beta::ln_beta;
    let mut total = 0.0;
    for i in 0..(ab.floor() as usize) {
        let i = i as f64;
        total += (ln_beta(aa + i, ba + bb) - (bb + i).ln() - ln_beta(1.0 + i, bb) - ln_beta(aa, ba)).exp();
    }
    // formula gives P(pB > pA) when ab is integer
    total.clamp(0.0, 1.0)
}

fn bayes(vm: &mut Vm, a: Args) -> R {
    let [da, db, xa, na, xb, nb, prior, draws, level] = a.bind(["a", "b", "x_a", "n_a", "x_b", "n_b", "prior", "draws", "level"])?;
    let (xa, na, xb, nb) = match (da, db) {
        (Some(da), Some(db)) => {
            let ((xa, na), (xb, nb)) = (counts(&nums(Some(da), "a")?), counts(&nums(Some(db), "b")?));
            (xa, na, xb, nb)
        }
        _ => (need(xa, "x_a")?.num("x_a")?, need(na, "n_a")?.num("n_a")?, need(xb, "x_b")?.num("x_b")?, need(nb, "n_b")?.num("n_b")?),
    };
    let (p0, p1) = match opt(prior) {
        Some(p) => {
            let p = nums(Some(p), "prior")?;
            if p.len() != 2 || p.iter().any(|v| *v <= 0.0) {
                return Err(value_err("prior must be [alpha, beta], both > 0"));
            }
            (p[0], p[1])
        }
        None => (1.0, 1.0),
    };
    let draws = opt(draws).map_or(Ok(100_000), |v| v.int("draws"))?.clamp(1000, 10_000_000) as usize;
    let level = super::stats::level_arg(level)?;
    let (aa, ba, ab, bb) = (p0 + xa, p1 + na - xa, p0 + xb, p1 + nb - xb);
    let mut lifts = Vec::with_capacity(draws);
    let (mut loss_a, mut loss_b, mut wins) = (0.0, 0.0, 0usize);
    for _ in 0..draws {
        let (ra, rb) = (vm.rng.beta(aa, ba), vm.rng.beta(ab, bb));
        wins += (rb > ra) as usize;
        loss_a += (rb - ra).max(0.0);
        loss_b += (ra - rb).max(0.0);
        lifts.push(rb / ra - 1.0);
    }
    let exact = ab.fract() == 0.0 && ab <= 2_000_000.0;
    let p_better = if exact { prob_b_beats_a(aa, ba, ab, bb) } else { wins as f64 / draws as f64 };
    let s = desc::sorted(&lifts);
    let lo = desc::quantile_sorted(&s, (1.0 - level) / 2.0);
    let hi = desc::quantile_sorted(&s, 1.0 - (1.0 - level) / 2.0);
    Ok(Rec::default()
        .text("method", "Bayesian Beta-Binomial")
        .num("prob_b_better", p_better)
        .num("expected_loss_a", loss_a / draws as f64)
        .num("expected_loss_b", loss_b / draws as f64)
        .num("rate_a", aa / (aa + ba))
        .num("rate_b", ab / (ab + bb))
        .num("lift", desc::median(&lifts))
        .pair("ci_lift", lo, hi)
        .int("draws", draws as i64)
        .value())
}

fn bayes_means(_: &mut Vm, a: Args) -> R {
    let [x, y, level] = a.bind(["a", "b", "level"])?;
    let (x, y) = (nums(x, "a")?, nums(y, "b")?);
    if x.len() < 2 || y.len() < 2 {
        return Err(value_err("each group needs at least 2 values"));
    }
    let level = super::stats::level_arg(level)?;
    let d = mean(&y) - mean(&x);
    let s = (var(&x, 1.0) / x.len() as f64 + var(&y, 1.0) / y.len() as f64).sqrt();
    let q = norm_ppf(0.5 + level / 2.0);
    // E[max(0, A - B)] and E[max(0, B - A)] for normal difference
    let loss_b = s * norm_pdf(d / s) - d * norm_cdf(-d / s);
    let loss_a = s * norm_pdf(d / s) + d * norm_cdf(d / s);
    Ok(Rec::default()
        .text("method", "Bayesian normal approximation")
        .num("prob_b_better", norm_cdf(d / s))
        .num("expected_loss_a", loss_a)
        .num("expected_loss_b", loss_b)
        .num("diff", d)
        .pair("ci_diff", d - q * s, d + q * s)
        .value())
}

// mixture SPRT: always-valid p-value, safe to peek every step
fn msprt(_: &mut Vm, a: Args) -> R {
    let [x, y, tau, alpha] = a.bind(["a", "b", "tau", "alpha"])?;
    let (x, y) = (nums(x, "a")?, nums(y, "b")?);
    let alpha = alpha_arg(alpha)?;
    let n = x.len().min(y.len());
    if n < 3 {
        return Err(value_err("msprt needs at least 3 values per group"));
    }
    let all: Vec<f64> = x.iter().chain(&y).copied().collect();
    let sd = var(&all, 1.0).sqrt();
    let tau = num_or(tau, "tau", 0.1 * sd)?;
    if tau <= 0.0 {
        return Err(value_err("tau must be > 0"));
    }
    let tau2 = tau * tau;
    let (mut sa, mut sb, mut qa, mut qb) = (0.0, 0.0, 0.0, 0.0);
    let mut p = 1.0f64;
    let mut first: Option<usize> = None;
    for i in 0..n {
        sa += x[i];
        sb += y[i];
        qa += x[i] * x[i];
        qb += y[i] * y[i];
        let k = (i + 1) as f64;
        if i < 1 {
            continue;
        }
        let (ma, mb) = (sa / k, sb / k);
        let s2 = ((qa - k * ma * ma) + (qb - k * mb * mb)) / (2.0 * k - 2.0);
        if s2 <= 0.0 {
            continue;
        }
        let v = 2.0 * s2 / k;
        let d = mb - ma;
        let lambda = (v / (v + tau2)).sqrt() * (tau2 * d * d / (2.0 * v * (v + tau2))).exp();
        p = p.min(1.0 / lambda);
        if first.is_none() && p < alpha {
            first = Some(i + 1);
        }
    }
    Ok(Rec::default()
        .text("method", "mixture sequential probability ratio test")
        .num("p_value", p)
        .flag("significant", p < alpha)
        .val("first_significant_at", first.map_or(Value::Nil, |f| Value::Int(f as i64)))
        .num("tau", tau)
        .int("n_per_group", n as i64)
        .value())
}

// Lan-DeMets alpha spending, alpha/2 per side (same as gsDesign, ldbounds)
fn spend(t: f64, alpha: f64, kind: &str) -> f64 {
    if t <= 0.0 {
        return 0.0;
    }
    match kind {
        "pocock" => alpha * (1.0 + (std::f64::consts::E - 1.0) * t).ln(),
        _ => 2.0 * (2.0 - 2.0 * norm_cdf(norm_ppf(1.0 - alpha / 4.0) / t.sqrt())),
    }
}

// boundaries |Z_k| >= c_k by numeric integration over the Brownian path
pub fn bounds(info: &[f64], alpha: f64, kind: &str) -> Vec<(f64, f64, f64)> {
    let grid = 1201;
    let mut out = Vec::new();
    let mut prev_t = 0.0;
    let mut spent = 0.0;
    // density of S_k on a grid (continuation region only)
    let mut pts: Vec<f64> = Vec::new();
    let mut dens: Vec<f64> = Vec::new();
    for (k, &t) in info.iter().enumerate() {
        let target = spend(t, alpha, kind) - spent;
        let dt = t - prev_t;
        let sdt = dt.sqrt();
        // probability of exit beyond |S| >= c*sqrt(t)
        let exit = |c: f64| -> f64 {
            let b = c * t.sqrt();
            if k == 0 {
                return 2.0 * norm_sf(c);
            }
            let h = pts[1] - pts[0];
            let mut s = 0.0;
            for (i, (&u, &f)) in pts.iter().zip(&dens).enumerate() {
                let w = if i == 0 || i == pts.len() - 1 {
                    1.0
                } else if i % 2 == 1 {
                    4.0
                } else {
                    2.0
                };
                s += w * f * (norm_cdf((-b - u) / sdt) + norm_sf((b - u) / sdt));
            }
            s * h / 3.0
        };
        let (mut lo, mut hi) = (0.0, 20.0);
        if target <= 1e-15 {
            lo = 20.0;
        } else {
            for _ in 0..100 {
                let mid = (lo + hi) / 2.0;
                if exit(mid) > target { lo = mid } else { hi = mid }
            }
        }
        let c = (lo + hi) / 2.0;
        spent += exit(c).min(target.max(0.0));
        out.push((c, 2.0 * norm_sf(c), spent));
        // new density on continuation region
        let b = c * t.sqrt();
        let npts: Vec<f64> = (0..grid).map(|i| -b + 2.0 * b * i as f64 / (grid - 1) as f64).collect();
        let nd: Vec<f64> = if k == 0 {
            npts.iter().map(|s| norm_pdf(s / t.sqrt()) / t.sqrt()).collect()
        } else {
            let h = pts[1] - pts[0];
            npts.iter()
                .map(|s| {
                    let mut acc = 0.0;
                    for (i, (&u, &f)) in pts.iter().zip(&dens).enumerate() {
                        let w = if i == 0 || i == pts.len() - 1 {
                            1.0
                        } else if i % 2 == 1 {
                            4.0
                        } else {
                            2.0
                        };
                        acc += w * f * norm_pdf((s - u) / sdt) / sdt;
                    }
                    acc * h / 3.0
                })
                .collect()
        };
        pts = npts;
        dens = nd;
        prev_t = t;
    }
    out
}

fn sequential_bounds(_: &mut Vm, a: Args) -> R {
    let [k, alpha, kind, info] = a.bind(["looks", "alpha", "spending", "info"])?;
    let alpha = alpha_arg(alpha)?;
    let kind = opt(kind).map_or(Ok("obrien-fleming".to_string()), |s| Ok::<_, Flow>(s.as_str("spending")?.to_string()))?;
    if kind != "obrien-fleming" && kind != "pocock" {
        return Err(value_err("spending must be \"obrien-fleming\" or \"pocock\""));
    }
    let info: Vec<f64> = match opt(info) {
        Some(i) => nums(Some(i), "info")?,
        None => {
            let k = opt(k).map_or(Ok(5), |v| v.int("looks"))?.clamp(1, 30);
            (1..=k).map(|i| i as f64 / k as f64).collect()
        }
    };
    if info.windows(2).any(|w| w[1] <= w[0]) || info.first().is_none_or(|t| *t <= 0.0) || (info.last().unwrap() - 1.0).abs() > 1e-9 {
        return Err(value_err("info must rise from >0 to exactly 1.0, like [0.25, 0.5, 0.75, 1.0]"));
    }
    let rows = bounds(&info, alpha, &kind);
    let looks: Vec<Value> = info
        .iter()
        .zip(rows)
        .enumerate()
        .map(|(i, (t, (z, p, s)))| {
            Rec::default().int("look", i as i64 + 1).num("info", *t).num("z_bound", z).num("p_bound", p).num("alpha_spent", s).value()
        })
        .collect();
    Ok(Rec::default()
        .text("method", &format!("group sequential, {kind} spending"))
        .num("alpha", alpha)
        .val("looks", Value::list(looks))
        .value())
}

fn col_nums(t: &Table, name: &str, rows: &[usize]) -> Result<Vec<f64>, Flow> {
    match t.col(name)? {
        Col::Num(v, _) => Ok(rows.iter().map(|&i| v[i]).filter(|x| !x.is_nan()).collect()),
        Col::Text(_) => Err(type_err(format!("metric column `{name}` must be numbers"))),
    }
}

fn key_of(t: &Table, name: &str, i: usize) -> Result<Value, Flow> {
    Ok(t.col(name)?.get(i))
}

// split a table by variant; returns (control rows, treatment rows)
fn split(t: &Table, variant: &str, rows: &[usize], control: &Value) -> Result<(Vec<usize>, Vec<usize>, Value), Flow> {
    let mut seen: Vec<Value> = Vec::new();
    for &i in rows {
        let v = key_of(t, variant, i)?;
        if !seen.iter().any(|s| equal(s, &v)) {
            seen.push(v);
        }
    }
    let ctrl = if matches!(control, Value::Nil) {
        let mut s = seen.clone();
        s.sort_by(|a, b| compare(a, b).unwrap_or(std::cmp::Ordering::Equal));
        s.first().cloned().unwrap_or(Value::Nil)
    } else {
        control.clone()
    };
    let others: Vec<&Value> = seen.iter().filter(|s| !equal(s, &ctrl)).collect();
    if others.len() != 1 {
        return Err(value_err(format!("variant column needs exactly 2 values (control + one variant), found {}", seen.len())));
    }
    let treat = others[0].clone();
    let mut a = Vec::new();
    let mut b = Vec::new();
    for &i in rows {
        let v = key_of(t, variant, i)?;
        if equal(&v, &ctrl) {
            a.push(i);
        } else {
            b.push(i);
        }
    }
    Ok((a, b, treat))
}

fn test_rows(t: &Table, metric: &str, a: &[usize], b: &[usize], alpha: f64) -> Result<Rec, Flow> {
    let (xa, xb) = (col_nums(t, metric, a)?, col_nums(t, metric, b)?);
    let binary = xa.iter().chain(&xb).all(|v| *v == 0.0 || *v == 1.0);
    if binary {
        let ((ca, na), (cb, nb)) = (counts(&xa), counts(&xb));
        prop_test(ca, na, cb, nb, alpha, Alt::Two).map_err(value_err)
    } else {
        mean_test(&xa, &xb, alpha, Alt::Two).map_err(value_err)
    }
}

// effect per segment, plus a Simpson's paradox warning
fn segments(_: &mut Vm, a: Args) -> R {
    let [data, variant, metric, by, control, alpha] = a.bind(["data", "variant", "metric", "by", "control", "alpha"])?;
    let data = need(data, "data")?;
    let t = data.object::<Table>().ok_or_else(|| type_err("data must be a table"))?;
    let (variant, metric, by) = (
        need(variant, "variant")?.as_str("variant")?.to_string(),
        need(metric, "metric")?.as_str("metric")?.to_string(),
        need(by, "by")?.as_str("by")?.to_string(),
    );
    let control = control.unwrap_or(Value::Nil);
    let alpha = alpha_arg(alpha)?;
    let all: Vec<usize> = (0..t.nrows).collect();
    let (a0, b0, _) = split(t, &variant, &all, &control)?;
    let overall = test_rows(t, &metric, &a0, &b0, alpha)?;
    let mut segs: IndexMap<Key, Vec<usize>> = IndexMap::new();
    for i in 0..t.nrows {
        segs.entry(Key::from(&key_of(t, &by, i)?)?).or_default().push(i);
    }
    let mut out = IndexMap::new();
    let mut signs = Vec::new();
    for (k, rows) in segs {
        let (sa, sb, _) = split(t, &variant, &rows, &control)?;
        if sa.len() < 2 || sb.len() < 2 {
            continue;
        }
        let r = test_rows(t, &metric, &sa, &sb, alpha)?;
        signs.push(r.get("diff").signum());
        out.insert(k, r.value());
    }
    let od = overall.get("diff").signum();
    let simpson = !signs.is_empty() && signs.iter().all(|s| *s != 0.0 && *s != od) && od != 0.0;
    Ok(Rec::default().val("overall", overall.value()).val("segments", Value::map(out)).flag("simpson_warning", simpson).value())
}

// lift by time period; a falling trend hints at a novelty effect
fn novelty(_: &mut Vm, a: Args) -> R {
    let [data, variant, metric, time, control, periods] = a.bind(["data", "variant", "metric", "time", "control", "periods"])?;
    let data = need(data, "data")?;
    let t = data.object::<Table>().ok_or_else(|| type_err("data must be a table"))?;
    let (variant, metric, time) = (
        need(variant, "variant")?.as_str("variant")?.to_string(),
        need(metric, "metric")?.as_str("metric")?.to_string(),
        need(time, "time")?.as_str("time")?.to_string(),
    );
    let control = control.unwrap_or(Value::Nil);
    let mut by_time: IndexMap<Key, Vec<usize>> = IndexMap::new();
    let mut order: Vec<(Value, Key)> = Vec::new();
    for i in 0..t.nrows {
        let v = key_of(t, &time, i)?;
        let k = Key::from(&v)?;
        if !by_time.contains_key(&k) {
            order.push((v, k.clone()));
        }
        by_time.entry(k).or_default().push(i);
    }
    order.sort_by(|a, b| compare(&a.0, &b.0).unwrap_or(std::cmp::Ordering::Equal));
    // optional bucketing into `periods` equal groups
    let nper = opt(periods).map(|p| p.int("periods")).transpose()?.map(|p| p.max(2) as usize);
    let buckets: Vec<Vec<usize>> = match nper {
        Some(p) if p < order.len() => {
            let per = order.len().div_ceil(p);
            order.chunks(per).map(|c| c.iter().flat_map(|(_, k)| by_time[k].clone()).collect()).collect()
        }
        _ => order.iter().map(|(_, k)| by_time[k].clone()).collect(),
    };
    let mut lifts = Vec::new();
    let mut rows_out = Vec::new();
    for (i, rows) in buckets.iter().enumerate() {
        let (a, b, _) = split(t, &variant, rows, &control)?;
        if a.len() < 2 || b.len() < 2 {
            continue;
        }
        let r = test_rows(t, &metric, &a, &b, 0.05)?;
        lifts.push((i as f64, r.get("lift")));
        rows_out.push(Rec::default().int("period", i as i64 + 1).num("lift", r.get("lift")).num("p_value", r.get("p_value")).value());
    }
    if lifts.len() < 3 {
        return Err(value_err("need at least 3 periods with both variants"));
    }
    let xs: Vec<f64> = lifts.iter().map(|l| l.0).collect();
    let ys: Vec<f64> = lifts.iter().map(|l| l.1).collect();
    let lr = super::stats::regress::linregress(&xs, &ys).map_err(value_err)?;
    let (slope, p) = (lr.get("slope"), lr.get("p_value"));
    Ok(Rec::default()
        .val("periods", Value::list(rows_out))
        .num("trend_slope", slope)
        .num("trend_p_value", p)
        .flag("novelty_suspected", slope < 0.0 && p < 0.05 && ys[0] > *ys.last().unwrap())
        .value())
}

// non-inferiority: is B no worse than A by more than `margin`?
fn guardrail(_: &mut Vm, a: Args) -> R {
    let [x, y, margin, relative, higher, alpha] = a.bind(["a", "b", "margin", "relative", "higher_is_better", "alpha"])?;
    let (x, y) = (nums(x, "a")?, nums(y, "b")?);
    let alpha = alpha_arg(alpha)?;
    let mut margin = need(margin, "margin")?.num("margin")?.abs();
    let higher = higher.is_none_or(|h| h.truthy());
    if x.len() < 2 || y.len() < 2 {
        return Err(value_err("each group needs at least 2 values"));
    }
    let (ma, mb) = (mean(&x), mean(&y));
    if relative.is_some_and(|r| r.truthy()) {
        margin *= ma.abs();
    }
    let se = (var(&x, 1.0) / x.len() as f64 + var(&y, 1.0) / y.len() as f64).sqrt();
    let diff = mb - ma;
    // shift so H0 is "B is worse by at least margin"
    let z = if higher { (diff + margin) / se } else { (margin - diff) / se };
    let p = norm_sf(z);
    let q = norm_ppf(1.0 - alpha);
    let bound = if higher { diff - q * se } else { diff + q * se };
    Ok(Rec::default()
        .text("method", "non-inferiority z-test")
        .flag("passed", p < alpha)
        .num("p_value", p)
        .num("diff", diff)
        .num(if higher { "ci_lower" } else { "ci_upper" }, bound)
        .num("margin", margin)
        .num("mean_a", ma)
        .num("mean_b", mb)
        .value())
}
