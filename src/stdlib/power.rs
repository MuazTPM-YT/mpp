// sample size, minimum detectable effect, achieved power
use super::stats::dist::{norm_cdf, norm_ppf};
use super::stats::tests::Rec;
use super::stats::{alt_arg, num_or};
use crate::vm::*;

macro_rules! natives {
    ($($name:literal => $f:expr),* $(,)?) => {
        &[$(Native { name: $name, f: $f }),*]
    };
}

pub static FNS: &[Native] = natives![
    "proportions" => proportions,
    "means" => means,
    "mde_proportions" => mde_proportions,
    "mde_means" => mde_means,
    "achieved_proportions" => achieved_proportions,
    "achieved_means" => achieved_means,
    "duration" => duration,
];

struct Design {
    alpha: f64,
    power: f64,
    ratio: f64,
    two_sided: bool,
}

fn design(alpha: Option<Value>, power: Option<Value>, ratio: Option<Value>, alt: Option<Value>) -> Result<Design, Flow> {
    let alpha = num_or(alpha, "alpha", 0.05)?;
    let power = num_or(power, "power", 0.8)?;
    let ratio = num_or(ratio, "ratio", 1.0)?;
    if !(0.0 < alpha && alpha < 1.0 && 0.0 < power && power < 1.0) || ratio <= 0.0 {
        return Err(value_err("alpha and power must be in (0, 1), ratio > 0"));
    }
    Ok(Design { alpha, power, ratio, two_sided: alt_arg(alt)? == super::stats::dist::Alt::Two })
}

impl Design {
    fn za(&self) -> f64 {
        if self.two_sided { norm_ppf(1.0 - self.alpha / 2.0) } else { norm_ppf(1.0 - self.alpha) }
    }
}

// power of a z-test: shift = effect / sd of estimate under H1, s0/s1 = null/alt sd ratio
fn z_power(effect: f64, se0: f64, se1: f64, d: &Design) -> f64 {
    let za = d.za();
    let up = norm_cdf((effect.abs() - za * se0) / se1);
    if d.two_sided { up + norm_cdf((-effect.abs() - za * se0) / se1) } else { up }
}

// smallest n_a with power >= target (n_b = ratio * n_a)
fn solve_n(f: impl Fn(f64) -> f64, target: f64) -> f64 {
    let (mut lo, mut hi) = (2.0f64, 4.0f64);
    while f(hi) < target {
        hi *= 2.0;
        if hi > 1e12 {
            return f64::INFINITY;
        }
    }
    for _ in 0..200 {
        let mid = (lo + hi) / 2.0;
        if f(mid) >= target { hi = mid } else { lo = mid }
    }
    hi
}

fn prop_power(pa: f64, pb: f64, na: f64, d: &Design) -> f64 {
    let nb = na * d.ratio;
    let pbar = (pa * na + pb * nb) / (na + nb);
    let se0 = (pbar * (1.0 - pbar) * (1.0 / na + 1.0 / nb)).sqrt();
    let se1 = (pa * (1.0 - pa) / na + pb * (1.0 - pb) / nb).sqrt();
    z_power(pb - pa, se0, se1, d)
}

fn mean_power(sd: f64, diff: f64, na: f64, d: &Design) -> f64 {
    let se = sd * (1.0 / na + 1.0 / (na * d.ratio)).sqrt();
    z_power(diff, se, se, d)
}

fn sizes(na: f64, d: &Design) -> Rec {
    let na = na.ceil();
    let nb = (na * d.ratio).ceil();
    Rec::default().int("n_a", na as i64).int("n_b", nb as i64).int("total", (na + nb) as i64).num("alpha", d.alpha).num("power", d.power)
}

// mde is relative (0.05 = +5%) unless relative = false
fn proportions(_: &mut Vm, a: Args) -> R {
    let [pa, mde, alpha, power, ratio, relative, alt] =
        a.bind(["baseline", "mde", "alpha", "power", "ratio", "relative", "alternative"])?;
    let d = design(alpha, power, ratio, alt)?;
    let pa = need(pa, "baseline")?.num("baseline")?;
    let mde = need(mde, "mde")?.num("mde")?;
    let pb = if relative.is_none_or(|r| r.truthy()) { pa * (1.0 + mde) } else { pa + mde };
    if !(0.0 < pa && pa < 1.0 && 0.0 < pb && pb < 1.0) || pa == pb {
        return Err(value_err("baseline and baseline + mde must be different rates between 0 and 1"));
    }
    let n = solve_n(|n| prop_power(pa, pb, n, &d), d.power);
    Ok(sizes(n, &d).num("rate_a", pa).num("rate_b", pb).num("mde_abs", pb - pa).num("mde_rel", pb / pa - 1.0).value())
}

fn means(_: &mut Vm, a: Args) -> R {
    let [sd, mde, alpha, power, ratio, alt] = a.bind(["sd", "mde", "alpha", "power", "ratio", "alternative"])?;
    let d = design(alpha, power, ratio, alt)?;
    let sd = need(sd, "sd")?.num("sd")?;
    let mde = need(mde, "mde")?.num("mde")?;
    if sd <= 0.0 || mde == 0.0 {
        return Err(value_err("sd must be > 0 and mde not 0"));
    }
    let n = solve_n(|n| mean_power(sd, mde, n, &d), d.power);
    Ok(sizes(n, &d).num("effect_size", mde / sd).value())
}

fn mde_proportions(_: &mut Vm, a: Args) -> R {
    let [pa, na, alpha, power, ratio, alt] = a.bind(["baseline", "n_a", "alpha", "power", "ratio", "alternative"])?;
    let d = design(alpha, power, ratio, alt)?;
    let pa = need(pa, "baseline")?.num("baseline")?;
    let na = need(na, "n_a")?.num("n_a")?;
    let (mut lo, mut hi) = (1e-9, 1.0 - pa - 1e-9);
    if prop_power(pa, pa + hi, na, &d) < d.power {
        return Err(value_err("no effect size reaches that power with this n"));
    }
    for _ in 0..200 {
        let mid = (lo + hi) / 2.0;
        if prop_power(pa, pa + mid, na, &d) >= d.power { hi = mid } else { lo = mid }
    }
    Ok(Rec::default().num("mde_abs", hi).num("mde_rel", hi / pa).num("rate_b", pa + hi).value())
}

fn mde_means(_: &mut Vm, a: Args) -> R {
    let [sd, na, alpha, power, ratio, alt] = a.bind(["sd", "n_a", "alpha", "power", "ratio", "alternative"])?;
    let d = design(alpha, power, ratio, alt)?;
    let sd = need(sd, "sd")?.num("sd")?;
    let na = need(na, "n_a")?.num("n_a")?;
    let (mut lo, mut hi) = (0.0, sd * 100.0);
    for _ in 0..200 {
        let mid = (lo + hi) / 2.0;
        if mean_power(sd, mid, na, &d) >= d.power { hi = mid } else { lo = mid }
    }
    Ok(Rec::default().num("mde", hi).num("effect_size", hi / sd).value())
}

fn achieved_proportions(_: &mut Vm, a: Args) -> R {
    let [pa, pb, na, alpha, ratio, alt] = a.bind(["rate_a", "rate_b", "n_a", "alpha", "ratio", "alternative"])?;
    let d = design(alpha, None, ratio, alt)?;
    Ok(Value::Float(prop_power(need(pa, "rate_a")?.num("rate_a")?, need(pb, "rate_b")?.num("rate_b")?, need(na, "n_a")?.num("n_a")?, &d)))
}

fn achieved_means(_: &mut Vm, a: Args) -> R {
    let [sd, diff, na, alpha, ratio, alt] = a.bind(["sd", "diff", "n_a", "alpha", "ratio", "alternative"])?;
    let d = design(alpha, None, ratio, alt)?;
    Ok(Value::Float(mean_power(need(sd, "sd")?.num("sd")?, need(diff, "diff")?.num("diff")?, need(na, "n_a")?.num("n_a")?, &d)))
}

// days needed for `total` users at `daily` users/day (share = part of traffic in test)
fn duration(_: &mut Vm, a: Args) -> R {
    let [total, daily, share] = a.bind(["total", "daily", "share"])?;
    let total = need(total, "total")?.num("total")?;
    let daily = need(daily, "daily")?.num("daily")? * num_or(share, "share", 1.0)?;
    if daily <= 0.0 {
        return Err(value_err("daily traffic must be > 0"));
    }
    let days = (total / daily).ceil();
    Ok(Rec::default().int("days", days as i64).num("weeks", days / 7.0).value())
}
