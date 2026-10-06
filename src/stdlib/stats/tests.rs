// hypothesis tests; numbers match scipy.stats defaults (see tests/fixtures)
use super::desc::{self, mean, var};
use super::dist::*;
use crate::vm::{Key, Value};
use indexmap::IndexMap;

pub type Res = Result<Rec, String>;

// result record built in order; becomes a map value
#[derive(Default, Clone, Debug)]
pub struct Rec(pub Vec<(&'static str, Value)>);

impl Rec {
    pub fn num(mut self, k: &'static str, v: f64) -> Rec {
        self.0.push((k, Value::Float(v)));
        self
    }
    pub fn int(mut self, k: &'static str, v: i64) -> Rec {
        self.0.push((k, Value::Int(v)));
        self
    }
    pub fn text(mut self, k: &'static str, v: &str) -> Rec {
        self.0.push((k, Value::str(v)));
        self
    }
    pub fn flag(mut self, k: &'static str, v: bool) -> Rec {
        self.0.push((k, Value::Bool(v)));
        self
    }
    pub fn pair(mut self, k: &'static str, a: f64, b: f64) -> Rec {
        self.0.push((k, Value::list(vec![Value::Float(a), Value::Float(b)])));
        self
    }
    pub fn val(mut self, k: &'static str, v: Value) -> Rec {
        self.0.push((k, v));
        self
    }
    pub fn get(&self, k: &str) -> f64 {
        self.0.iter().find(|(n, _)| *n == k).and_then(|(_, v)| v.num("").ok()).unwrap_or(f64::NAN)
    }
    pub fn value(self) -> Value {
        Value::map(self.0.into_iter().map(|(k, v)| (Key::Str(k.into()), v)).collect::<IndexMap<_, _>>())
    }
}

fn need_n(x: &[f64], n: usize, what: &str) -> Result<(), String> {
    if x.len() < n { Err(format!("{what} needs at least {n} values, got {}", x.len())) } else { Ok(()) }
}

// confidence interval bounds for an estimate with a t (or z if df inf) distribution
fn ci_t(est: f64, se: f64, df: f64, level: f64, alt: Alt) -> (f64, f64) {
    match alt {
        Alt::Two => {
            let q = t_ppf(0.5 + level / 2.0, df);
            (est - q * se, est + q * se)
        }
        Alt::Less => (f64::NEG_INFINITY, est + t_ppf(level, df) * se),
        Alt::Greater => (est - t_ppf(level, df) * se, f64::INFINITY),
    }
}

pub fn ttest_1samp(x: &[f64], mu: f64, alt: Alt, level: f64) -> Res {
    need_n(x, 2, "t-test")?;
    let n = x.len() as f64;
    let m = mean(x);
    let se = (var(x, 1.0) / n).sqrt();
    let t = (m - mu) / se;
    let df = n - 1.0;
    let (lo, hi) = ci_t(m, se, df, level, alt);
    Ok(Rec::default()
        .text("method", "one-sample t-test")
        .num("statistic", t)
        .num("p_value", p_from_t(t, df, alt))
        .num("df", df)
        .num("mean", m)
        .pair("ci", lo, hi)
        .num("cohens_d", (m - mu) / var(x, 1.0).sqrt())
        .text("alternative", alt.name()))
}

pub fn ttest_ind(a: &[f64], b: &[f64], equal_var: bool, alt: Alt, level: f64) -> Res {
    need_n(a, 2, "t-test sample a")?;
    need_n(b, 2, "t-test sample b")?;
    let (na, nb) = (a.len() as f64, b.len() as f64);
    let (ma, mb) = (mean(a), mean(b));
    let (va, vb) = (var(a, 1.0), var(b, 1.0));
    let (se, df) = if equal_var {
        let sp = ((na - 1.0) * va + (nb - 1.0) * vb) / (na + nb - 2.0);
        ((sp * (1.0 / na + 1.0 / nb)).sqrt(), na + nb - 2.0)
    } else {
        let (qa, qb) = (va / na, vb / nb);
        ((qa + qb).sqrt(), (qa + qb).powi(2) / (qa * qa / (na - 1.0) + qb * qb / (nb - 1.0)))
    };
    let diff = ma - mb;
    let t = diff / se;
    let (lo, hi) = ci_t(diff, se, df, level, alt);
    Ok(Rec::default()
        .text("method", if equal_var { "Student two-sample t-test" } else { "Welch two-sample t-test" })
        .num("statistic", t)
        .num("p_value", p_from_t(t, df, alt))
        .num("df", df)
        .num("mean_a", ma)
        .num("mean_b", mb)
        .num("diff", diff)
        .pair("ci", lo, hi)
        .num("cohens_d", cohens_d(a, b))
        .text("alternative", alt.name()))
}

pub fn ttest_rel(a: &[f64], b: &[f64], alt: Alt, level: f64) -> Res {
    if a.len() != b.len() {
        return Err(format!("paired t-test needs equal lengths, got {} and {}", a.len(), b.len()));
    }
    let d: Vec<f64> = a.iter().zip(b).map(|(x, y)| x - y).collect();
    let mut r = ttest_1samp(&d, 0.0, alt, level)?;
    r.0[0].1 = Value::str("paired t-test");
    Ok(r)
}

pub fn ztest(x: &[f64], mu: f64, sigma: Option<f64>, alt: Alt, level: f64) -> Res {
    need_n(x, 2, "z-test")?;
    let n = x.len() as f64;
    let m = mean(x);
    let s = sigma.unwrap_or_else(|| var(x, 1.0).sqrt());
    let se = s / n.sqrt();
    let z = (m - mu) / se;
    let (lo, hi) = ci_t(m, se, f64::INFINITY, level, alt);
    Ok(Rec::default()
        .text("method", "z-test")
        .num("statistic", z)
        .num("p_value", p_from_z(z, alt))
        .num("mean", m)
        .pair("ci", lo, hi)
        .text("alternative", alt.name()))
}

pub fn cohens_d(a: &[f64], b: &[f64]) -> f64 {
    let (na, nb) = (a.len() as f64, b.len() as f64);
    let sp = (((na - 1.0) * var(a, 1.0) + (nb - 1.0) * var(b, 1.0)) / (na + nb - 2.0)).sqrt();
    (mean(a) - mean(b)) / sp
}

pub fn hedges_g(a: &[f64], b: &[f64]) -> f64 {
    let n = (a.len() + b.len()) as f64;
    cohens_d(a, b) * (1.0 - 3.0 / (4.0 * n - 9.0))
}

pub fn cliffs_delta(a: &[f64], b: &[f64]) -> f64 {
    let mut s = 0i64;
    for x in a {
        for y in b {
            s += (x > y) as i64 - (x < y) as i64;
        }
    }
    s as f64 / (a.len() * b.len()) as f64
}

pub fn chi2_contingency(t: &[Vec<f64>], correction: bool) -> Res {
    let r = t.len();
    let c = t.first().map_or(0, Vec::len);
    if r < 2 || c < 2 || t.iter().any(|row| row.len() != c) {
        return Err("contingency table needs at least 2x2, all rows the same length".into());
    }
    let rows: Vec<f64> = t.iter().map(|row| row.iter().sum()).collect();
    let cols: Vec<f64> = (0..c).map(|j| t.iter().map(|row| row[j]).sum()).collect();
    let n: f64 = rows.iter().sum();
    let dof = ((r - 1) * (c - 1)) as f64;
    let mut stat = 0.0;
    let mut expected = Vec::new();
    for i in 0..r {
        let mut erow = Vec::new();
        for j in 0..c {
            let e = rows[i] * cols[j] / n;
            if e == 0.0 {
                return Err("an expected count is zero; drop empty rows/columns".into());
            }
            let mut d = (t[i][j] - e).abs();
            if correction && dof == 1.0 {
                d = (d - 0.5).max(0.0);
            }
            stat += d * d / e;
            erow.push(Value::Float(e));
        }
        expected.push(Value::list(erow));
    }
    let k = (r.min(c) - 1) as f64;
    Ok(Rec::default()
        .text("method", "chi-square test of independence")
        .num("statistic", stat)
        .num("p_value", chi2_sf(stat, dof))
        .num("df", dof)
        .num("cramers_v", (stat / (n * k)).sqrt())
        .val("expected", Value::list(expected)))
}

pub fn chi2_gof(obs: &[f64], exp: Option<&[f64]>) -> Res {
    if obs.len() < 2 {
        return Err("goodness-of-fit needs at least 2 categories".into());
    }
    let n: f64 = obs.iter().sum();
    let e: Vec<f64> = match exp {
        Some(e) => {
            if e.len() != obs.len() {
                return Err("observed and expected lengths differ".into());
            }
            let s: f64 = e.iter().sum();
            e.iter().map(|x| x / s * n).collect()
        }
        None => vec![n / obs.len() as f64; obs.len()],
    };
    let stat: f64 = obs.iter().zip(&e).map(|(o, e)| (o - e) * (o - e) / e).sum();
    let df = (obs.len() - 1) as f64;
    Ok(Rec::default().text("method", "chi-square goodness of fit").num("statistic", stat).num("p_value", chi2_sf(stat, df)).num("df", df))
}

fn hypergeom_pmf(k: f64, r1: f64, c1: f64, n: f64) -> f64 {
    (ln_choose(c1, k) + ln_choose(n - c1, r1 - k) - ln_choose(n, r1)).exp()
}

pub fn fisher_exact(t: [[f64; 2]; 2], alt: Alt) -> Res {
    let [[a, b], [c, d]] = t;
    let (r1, c1, n) = (a + b, a + c, a + b + c + d);
    let lo = (r1 + c1 - n).max(0.0);
    let hi = r1.min(c1);
    let p_obs = hypergeom_pmf(a, r1, c1, n);
    let mut p = 0.0;
    let mut k = lo;
    while k <= hi {
        let pk = hypergeom_pmf(k, r1, c1, n);
        p += match alt {
            Alt::Two if pk <= p_obs * (1.0 + 1e-7) => pk,
            Alt::Less if k <= a => pk,
            Alt::Greater if k >= a => pk,
            _ => 0.0,
        };
        k += 1.0;
    }
    let or = if b * c == 0.0 { if a * d == 0.0 { f64::NAN } else { f64::INFINITY } } else { a * d / (b * c) };
    Ok(Rec::default().text("method", "Fisher exact test").num("odds_ratio", or).num("p_value", p.min(1.0)).text("alternative", alt.name()))
}

// count of ways to get each U (no ties), by recursion on sizes
fn mwu_exact_cdf(u: f64, n1: usize, n2: usize) -> f64 {
    let max = n1 * n2;
    // f[i][j][u] built row by row: f(i,j,u) = f(i-1,j,u-j) + f(i,j-1,u)
    let mut prev: Vec<Vec<f64>> = vec![vec![0.0; max + 1]; n2 + 1];
    for row in prev.iter_mut() {
        row[0] = 1.0;
    }
    for _ in 1..=n1 {
        let mut cur: Vec<Vec<f64>> = vec![vec![0.0; max + 1]; n2 + 1];
        cur[0][0] = 1.0;
        for j in 1..=n2 {
            for uu in 0..=max {
                let mut v = cur[j - 1][uu];
                if uu >= j {
                    v += prev[j][uu - j];
                }
                cur[j][uu] = v;
            }
        }
        prev = cur;
    }
    let counts = &prev[n2];
    let total: f64 = counts.iter().sum();
    let k = u.floor().max(-1.0);
    if k < 0.0 {
        return 0.0;
    }
    counts.iter().take(k as usize + 1).sum::<f64>() / total
}

pub fn mannwhitneyu(a: &[f64], b: &[f64], alt: Alt, method: Option<&str>) -> Res {
    need_n(a, 1, "Mann-Whitney sample a")?;
    need_n(b, 1, "Mann-Whitney sample b")?;
    let (n1, n2) = (a.len(), b.len());
    let all: Vec<f64> = a.iter().chain(b).copied().collect();
    let r = desc::rank(&all);
    let r1: f64 = r[..n1].iter().sum();
    let u1 = r1 - (n1 * (n1 + 1)) as f64 / 2.0;
    let u2 = (n1 * n2) as f64 - u1;
    let ties = desc::tie_counts(&all);
    let method = method.unwrap_or(if (n1 > 8 && n2 > 8) || !ties.is_empty() { "asymptotic" } else { "exact" });
    let p = if method == "exact" {
        let total = (n1 * n2) as f64;
        match alt {
            Alt::Greater => 1.0 - mwu_exact_cdf(u1 - 1.0, n1, n2),
            Alt::Less => mwu_exact_cdf(u1, n1, n2),
            Alt::Two => {
                let u = u1.max(u2);
                (2.0 * (1.0 - mwu_exact_cdf(u - 1.0, n1, n2))).min(1.0).max(if total == 0.0 { 1.0 } else { 0.0 })
            }
        }
    } else {
        let n = (n1 + n2) as f64;
        let tie: f64 = ties.iter().map(|t| t * t * t - t).sum();
        let s = ((n1 * n2) as f64 / 12.0 * ((n + 1.0) - tie / (n * (n - 1.0)))).sqrt();
        let mu = (n1 * n2) as f64 / 2.0;
        let u = if alt == Alt::Two { u1.max(u2) } else { u1 };
        let mut num = u - mu;
        let sign = match alt {
            Alt::Greater => 1.0,
            Alt::Less => -1.0,
            Alt::Two => num.signum(),
        };
        num -= 0.5 * sign;
        let z = num / s;
        match alt {
            Alt::Two => (2.0 * norm_sf(z)).min(1.0),
            Alt::Greater => norm_sf(z),
            Alt::Less => norm_cdf(z),
        }
    };
    Ok(Rec::default()
        .text("method", if method == "exact" { "Mann-Whitney U (exact)" } else { "Mann-Whitney U (normal approx)" })
        .num("statistic", u1)
        .num("p_value", p)
        .num("cliffs_delta", cliffs_delta(a, b))
        .num("rank_biserial", 2.0 * u1 / (n1 * n2) as f64 - 1.0)
        .text("alternative", alt.name()))
}

// counts of subsets of 1..n with each rank sum
fn signed_rank_counts(n: usize) -> Vec<f64> {
    let max = n * (n + 1) / 2;
    let mut c = vec![0.0; max + 1];
    c[0] = 1.0;
    for k in 1..=n {
        for s in (k..=max).rev() {
            c[s] += c[s - k];
        }
    }
    c
}

pub fn wilcoxon(x: &[f64], y: Option<&[f64]>, alt: Alt, method: Option<&str>) -> Res {
    let d: Vec<f64> = match y {
        Some(y) => {
            if x.len() != y.len() {
                return Err(format!("wilcoxon needs equal lengths, got {} and {}", x.len(), y.len()));
            }
            x.iter().zip(y).map(|(a, b)| a - b).collect()
        }
        None => x.to_vec(),
    };
    let zeros = d.iter().filter(|v| **v == 0.0).count();
    let d: Vec<f64> = d.into_iter().filter(|v| *v != 0.0).collect();
    need_n(&d, 1, "wilcoxon (non-zero differences)")?;
    let n = d.len();
    let absd: Vec<f64> = d.iter().map(|v| v.abs()).collect();
    let r = desc::rank(&absd);
    let r_plus: f64 = d.iter().zip(&r).filter(|(v, _)| **v > 0.0).map(|(_, r)| r).sum();
    let r_minus: f64 = d.iter().zip(&r).filter(|(v, _)| **v < 0.0).map(|(_, r)| r).sum();
    let ties = desc::tie_counts(&absd);
    let method = method.unwrap_or(if n <= 50 && ties.is_empty() && zeros == 0 { "exact" } else { "asymptotic" });
    let stat = if alt == Alt::Two { r_plus.min(r_minus) } else { r_plus };
    let p = if method == "exact" {
        let c = signed_rank_counts(n);
        let total: f64 = c.iter().sum();
        let cdf = |w: f64| if w < 0.0 { 0.0 } else { c.iter().take(w as usize + 1).sum::<f64>() / total };
        match alt {
            Alt::Less => cdf(r_plus.ceil()),
            Alt::Greater => 1.0 - cdf(r_plus.floor() - 1.0),
            Alt::Two => (2.0 * cdf(r_plus.ceil()).min(1.0 - cdf(r_plus.floor() - 1.0))).min(1.0),
        }
    } else {
        let nf = n as f64;
        let mn = nf * (nf + 1.0) / 4.0;
        let tie: f64 = ties.iter().map(|t| t * t * t - t).sum();
        let se = ((nf * (nf + 1.0) * (2.0 * nf + 1.0) - tie / 2.0) / 24.0).sqrt();
        p_from_z((r_plus - mn) / se, alt)
    };
    Ok(Rec::default()
        .text("method", if method == "exact" { "Wilcoxon signed-rank (exact)" } else { "Wilcoxon signed-rank (normal approx)" })
        .num("statistic", stat)
        .num("p_value", p)
        .int("n", n as i64)
        .int("zeros_dropped", zeros as i64)
        .text("alternative", alt.name()))
}

// two-sided exact p for D (lattice path count, Hodges)
fn ks_exact_two_sided(d: f64, n1: usize, n2: usize) -> f64 {
    let (n1f, n2f) = (n1 as f64, n2 as f64);
    let tol = 1e-12;
    // count paths with |i/n1 - j/n2| < d (inside), as probabilities to avoid overflow
    let mut u = vec![0.0f64; n2 + 1];
    for (j, uj) in u.iter_mut().enumerate() {
        *uj = if (j as f64 / n2f) < d - tol { 1.0 } else { 0.0 };
    }
    for i in 1..=n1 {
        let w = i as f64 / (i + n2) as f64;
        let inside = |j: usize| ((i as f64 / n1f) - (j as f64 / n2f)).abs() < d - tol;
        u[0] = if inside(0) { w * u[0] } else { 0.0 };
        for j in 1..=n2 {
            u[j] = if inside(j) { w * u[j] + u[j - 1] } else { 0.0 };
        }
    }
    (1.0 - u[n2]).clamp(0.0, 1.0)
}

pub fn ks_2samp(a: &[f64], b: &[f64], method: Option<&str>) -> Res {
    need_n(a, 1, "KS sample a")?;
    need_n(b, 1, "KS sample b")?;
    let (sa, sb) = (desc::sorted(a), desc::sorted(b));
    let (n1, n2) = (sa.len(), sb.len());
    let mut d: f64 = 0.0;
    let all: Vec<f64> = desc::sorted(&sa.iter().chain(&sb).copied().collect::<Vec<_>>());
    for x in &all {
        let fa = sa.partition_point(|v| v <= x) as f64 / n1 as f64;
        let fb = sb.partition_point(|v| v <= x) as f64 / n2 as f64;
        d = d.max((fa - fb).abs());
    }
    let method = method.unwrap_or(if n1 * n2 <= 10_000 { "exact" } else { "asymptotic" });
    let p = if method == "exact" {
        ks_exact_two_sided(d, n1, n2)
    } else {
        let en = (n1 * n2) as f64 / (n1 + n2) as f64;
        kstwo_sf(d, en.round() as usize)
    };
    Ok(Rec::default()
        .text("method", if method == "exact" { "two-sample KS (exact)" } else { "two-sample KS (asymptotic)" })
        .num("statistic", d)
        .num("p_value", p))
}

pub fn anova(groups: &[Vec<f64>]) -> Res {
    if groups.len() < 2 {
        return Err("ANOVA needs at least 2 groups".into());
    }
    let all: Vec<f64> = groups.iter().flatten().copied().collect();
    let grand = mean(&all);
    let n = all.len() as f64;
    let k = groups.len() as f64;
    let ssb: f64 = groups.iter().map(|g| g.len() as f64 * (mean(g) - grand).powi(2)).sum();
    let ssw: f64 = groups
        .iter()
        .map(|g| {
            let m = mean(g);
            g.iter().map(|x| (x - m).powi(2)).sum::<f64>()
        })
        .sum();
    let (df1, df2) = (k - 1.0, n - k);
    let f = (ssb / df1) / (ssw / df2);
    Ok(Rec::default()
        .text("method", "one-way ANOVA")
        .num("statistic", f)
        .num("p_value", f_sf(f, df1, df2))
        .num("df_between", df1)
        .num("df_within", df2)
        .num("eta_squared", ssb / (ssb + ssw)))
}

pub fn kruskal(groups: &[Vec<f64>]) -> Res {
    if groups.len() < 2 {
        return Err("Kruskal-Wallis needs at least 2 groups".into());
    }
    let all: Vec<f64> = groups.iter().flatten().copied().collect();
    let n = all.len() as f64;
    let r = desc::rank(&all);
    let mut h = 0.0;
    let mut at = 0;
    for g in groups {
        let rs: f64 = r[at..at + g.len()].iter().sum();
        h += rs * rs / g.len() as f64;
        at += g.len();
    }
    h = 12.0 / (n * (n + 1.0)) * h - 3.0 * (n + 1.0);
    let tie: f64 = desc::tie_counts(&all).iter().map(|t| t * t * t - t).sum();
    h /= 1.0 - tie / (n * n * n - n);
    let k = groups.len() as f64;
    Ok(Rec::default()
        .text("method", "Kruskal-Wallis H")
        .num("statistic", h)
        .num("p_value", chi2_sf(h, k - 1.0))
        .num("df", k - 1.0)
        .num("epsilon_squared", h / ((n * n - 1.0) / (n + 1.0))))
}

pub fn levene(groups: &[Vec<f64>], center: &str) -> Res {
    if groups.len() < 2 {
        return Err("Levene needs at least 2 groups".into());
    }
    let z: Vec<Vec<f64>> = groups
        .iter()
        .map(|g| {
            let c = if center == "mean" { mean(g) } else { desc::median(g) };
            g.iter().map(|x| (x - c).abs()).collect()
        })
        .collect();
    let mut r = anova(&z)?;
    r.0[0].1 = Value::str(if center == "mean" { "Levene (mean)" } else { "Brown-Forsythe Levene (median)" });
    r.0.truncate(5);
    Ok(r)
}

fn poly(c: &[f64], x: f64) -> f64 {
    c.iter().rev().fold(0.0, |acc, v| acc * x + v)
}

// Shapiro-Wilk W and p, Royston's AS R94 algorithm
pub fn shapiro(x: &[f64]) -> Res {
    let n = x.len();
    if n < 3 {
        return Err("Shapiro-Wilk needs at least 3 values".into());
    }
    if n > 5000 {
        return Err("Shapiro-Wilk p-values are not reliable above 5000 values".into());
    }
    let s = desc::sorted(x);
    let range = s[n - 1] - s[0];
    if range < 1e-12 {
        return Err("all values are the same".into());
    }
    let an = n as f64;
    let nn2 = n / 2;
    let mut a = vec![0.0; nn2 + 1];
    if n == 3 {
        a[1] = 0.5f64.sqrt();
    } else {
        let an25 = an + 0.25;
        let m: Vec<f64> = (0..=nn2).map(|i| if i == 0 { 0.0 } else { norm_ppf((i as f64 - 0.375) / an25) }).collect();
        let summ2: f64 = 2.0 * m[1..].iter().map(|v| v * v).sum::<f64>();
        let ssumm2 = summ2.sqrt();
        let rsn = 1.0 / an.sqrt();
        let c1 = [0.0, 0.221157, -0.147981, -2.071190, 4.434685, -2.706056];
        let c2 = [0.0, 0.042981, -0.293762, -1.752461, 5.682633, -3.582633];
        let a1 = poly(&c1, rsn) - m[1] / ssumm2;
        let (i1, fac) = if n > 5 {
            let a2 = -m[2] / ssumm2 + poly(&c2, rsn);
            a[2] = a2;
            (3, ((summ2 - 2.0 * m[1] * m[1] - 2.0 * m[2] * m[2]) / (1.0 - 2.0 * a1 * a1 - 2.0 * a2 * a2)).sqrt())
        } else {
            (2, ((summ2 - 2.0 * m[1] * m[1]) / (1.0 - 2.0 * a1 * a1)).sqrt())
        };
        a[1] = a1;
        for i in i1..=nn2 {
            a[i] = -m[i] / fac;
        }
    }
    let mean_x = mean(&s);
    let ssq: f64 = s.iter().map(|v| (v - mean_x).powi(2)).sum();
    let num: f64 = (1..=nn2).map(|i| a[i] * (s[n - i] - s[i - 1])).sum();
    let w = (num * num / ssq).min(1.0);
    let p = if n == 3 {
        let pi6 = 1.909_859_317_102_744;
        let stqr = std::f64::consts::FRAC_PI_3;
        (pi6 * (w.sqrt().asin() - stqr)).max(0.0)
    } else {
        let w1 = (1.0 - w).ln();
        let xx = an.ln();
        if n <= 11 {
            let gamma = poly(&[-2.273, 0.459], an);
            if w1 >= gamma {
                1e-99
            } else {
                let w1 = -(gamma - w1).ln();
                let m = poly(&[0.5440, -0.39978, 0.025054, -6.714e-4], an);
                let sd = poly(&[1.3822, -0.77857, 0.062767, -0.0020322], an).exp();
                norm_sf((w1 - m) / sd)
            }
        } else {
            let m = poly(&[-1.5861, -0.31082, -0.083751, 0.0038915], xx);
            let sd = poly(&[-0.4803, -0.082676, 0.0030302], xx).exp();
            norm_sf((w1 - m) / sd)
        }
    };
    Ok(Rec::default().text("method", "Shapiro-Wilk").num("statistic", w).num("p_value", p).flag("normal", p > 0.05))
}

pub fn pearson(x: &[f64], y: &[f64], alt: Alt, level: f64) -> Res {
    if x.len() != y.len() {
        return Err(format!("lengths differ: {} and {}", x.len(), y.len()));
    }
    need_n(x, 3, "correlation")?;
    let n = x.len() as f64;
    let r = desc::pearson_r(x, y);
    let df = n - 2.0;
    let t = r * (df / (1.0 - r * r)).sqrt();
    let p = if r.abs() >= 1.0 {
        match alt {
            Alt::Two => 0.0,
            Alt::Greater => (r < 0.0) as i64 as f64,
            Alt::Less => (r > 0.0) as i64 as f64,
        }
    } else {
        p_from_t(t, df, alt)
    };
    let z = r.atanh();
    let se = 1.0 / (n - 3.0).sqrt();
    let q = norm_ppf(0.5 + level / 2.0);
    Ok(Rec::default()
        .text("method", "Pearson correlation")
        .num("r", r)
        .num("statistic", r)
        .num("p_value", p)
        .pair("ci", (z - q * se).tanh(), (z + q * se).tanh())
        .int("n", n as i64))
}

pub fn spearman(x: &[f64], y: &[f64], alt: Alt) -> Res {
    if x.len() != y.len() {
        return Err(format!("lengths differ: {} and {}", x.len(), y.len()));
    }
    need_n(x, 3, "correlation")?;
    let (rx, ry) = (desc::rank(x), desc::rank(y));
    let rho = desc::pearson_r(&rx, &ry);
    let df = x.len() as f64 - 2.0;
    let t = rho * (df / ((1.0 - rho) * (1.0 + rho))).sqrt();
    let p = if rho.abs() >= 1.0 { 0.0 } else { p_from_t(t, df, alt) };
    Ok(Rec::default()
        .text("method", "Spearman rank correlation")
        .num("rho", rho)
        .num("statistic", rho)
        .num("p_value", p)
        .int("n", x.len() as i64))
}

pub fn kendall(x: &[f64], y: &[f64]) -> Res {
    if x.len() != y.len() {
        return Err(format!("lengths differ: {} and {}", x.len(), y.len()));
    }
    need_n(x, 2, "Kendall tau")?;
    let n = x.len();
    let (mut con, mut dis) = (0f64, 0f64);
    for i in 0..n {
        for j in i + 1..n {
            let dx = (x[i] - x[j]).signum() * ((x[i] != x[j]) as i64 as f64);
            let dy = (y[i] - y[j]).signum() * ((y[i] != y[j]) as i64 as f64);
            if dx * dy > 0.0 {
                con += 1.0;
            } else if dx * dy < 0.0 {
                dis += 1.0;
            }
        }
    }
    let tie_stats = |v: &[f64]| {
        let t = desc::tie_counts(v);
        (
            t.iter().map(|c| c * (c - 1.0) / 2.0).sum::<f64>(),
            t.iter().map(|c| c * (c - 1.0) * (c - 2.0)).sum::<f64>(),
            t.iter().map(|c| c * (c - 1.0) * (2.0 * c + 5.0)).sum::<f64>(),
        )
    };
    let (xtie, x0, x1) = tie_stats(x);
    let (ytie, y0, y1) = tie_stats(y);
    let tot = (n * (n - 1) / 2) as f64;
    let cmd = con - dis;
    let tau = cmd / ((tot - xtie).sqrt() * (tot - ytie).sqrt());
    let p = if xtie == 0.0 && ytie == 0.0 && (n <= 33 || dis.min(tot - dis) <= 1.0) {
        kendall_exact_p(n, dis.min(tot - dis), tot)
    } else {
        let nf = n as f64;
        let m = nf * (nf - 1.0);
        let var = (m * (2.0 * nf + 5.0) - x1 - y1) / 18.0 + (2.0 * xtie * ytie) / m + x0 * y0 / (9.0 * m * (nf - 2.0));
        (2.0 * norm_sf((cmd / var.sqrt()).abs())).min(1.0)
    };
    Ok(Rec::default().text("method", "Kendall tau-b").num("tau", tau).num("statistic", tau).num("p_value", p).int("n", n as i64))
}

fn kendall_exact_p(n: usize, c: f64, tot: f64) -> f64 {
    let fact = |k: usize| (1..=k).map(|i| i as f64).product::<f64>();
    if n <= 2 || 2.0 * c == tot {
        return 1.0;
    }
    if c == 0.0 {
        return (2.0 / fact(n)).min(1.0);
    }
    if c == 1.0 {
        return (2.0 / fact(n - 1)).min(1.0);
    }
    let c = c as usize;
    let mut new = vec![0.0; c + 1];
    new[0] = 1.0;
    new[1] = 1.0;
    for j in 3..=n {
        let old = new.clone();
        for k in 1..(j.min(c + 1)) {
            new[k] += new[k - 1];
        }
        for k in j..=c {
            new[k] += new[k - 1] - old[k - j];
        }
    }
    (2.0 * new.iter().sum::<f64>() / fact(n)).min(1.0)
}

// p-value corrections for many tests
pub fn adjust(p: &[f64], method: &str) -> Result<Vec<f64>, String> {
    let n = p.len();
    let nf = n as f64;
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_by(|&a, &b| p[a].total_cmp(&p[b]));
    let mut out = vec![0.0; n];
    match method {
        "bonferroni" => {
            for i in 0..n {
                out[i] = (p[i] * nf).min(1.0);
            }
        }
        "holm" => {
            let mut run: f64 = 0.0;
            for (rank, &i) in idx.iter().enumerate() {
                run = run.max((nf - rank as f64) * p[i]);
                out[i] = run.min(1.0);
            }
        }
        "hochberg" => {
            let mut run: f64 = 1.0;
            for (rank, &i) in idx.iter().enumerate().rev() {
                run = run.min((nf - rank as f64) * p[i]);
                out[i] = run.min(1.0);
            }
        }
        "bh" | "fdr_bh" | "benjamini-hochberg" | "by" | "fdr_by" => {
            let cm = if method.ends_with("by") { (1..=n).map(|k| 1.0 / k as f64).sum::<f64>() } else { 1.0 };
            let mut run: f64 = 1.0;
            for (rank, &i) in idx.iter().enumerate().rev() {
                run = run.min(p[i] * nf * cm / (rank as f64 + 1.0));
                out[i] = run.min(1.0);
            }
        }
        other => return Err(format!("unknown correction {other:?} (use bonferroni, holm, hochberg, bh, by)")),
    }
    Ok(out)
}
