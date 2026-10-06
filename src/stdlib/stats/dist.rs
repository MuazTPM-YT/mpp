// probability distributions (statrs underneath) + tail helpers
use statrs::distribution::{Beta, ChiSquared, Continuous, ContinuousCDF, FisherSnedecor, Normal, StudentsT};
use statrs::function::gamma::ln_gamma;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Alt {
    Two,
    Less,
    Greater,
}

impl Alt {
    pub fn parse(s: &str) -> Option<Alt> {
        match s {
            "two-sided" | "two_sided" | "both" => Some(Alt::Two),
            "less" => Some(Alt::Less),
            "greater" => Some(Alt::Greater),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Alt::Two => "two-sided",
            Alt::Less => "less",
            Alt::Greater => "greater",
        }
    }
}

pub fn norm_cdf(z: f64) -> f64 {
    0.5 * statrs::function::erf::erfc(-z / std::f64::consts::SQRT_2)
}

pub fn norm_sf(z: f64) -> f64 {
    0.5 * statrs::function::erf::erfc(z / std::f64::consts::SQRT_2)
}

pub fn norm_pdf(z: f64) -> f64 {
    (-0.5 * z * z).exp() / (2.0 * std::f64::consts::PI).sqrt()
}

pub fn norm_ppf(p: f64) -> f64 {
    if p <= 0.0 {
        return f64::NEG_INFINITY;
    }
    if p >= 1.0 {
        return f64::INFINITY;
    }
    Normal::new(0.0, 1.0).unwrap().inverse_cdf(p)
}

pub fn t_cdf(t: f64, df: f64) -> f64 {
    if df.is_infinite() {
        return norm_cdf(t);
    }
    StudentsT::new(0.0, 1.0, df).map_or(f64::NAN, |d| d.cdf(t))
}

pub fn t_sf(t: f64, df: f64) -> f64 {
    t_cdf(-t, df)
}

pub fn t_ppf(p: f64, df: f64) -> f64 {
    if df.is_infinite() {
        return norm_ppf(p);
    }
    StudentsT::new(0.0, 1.0, df).map_or(f64::NAN, |d| d.inverse_cdf(p))
}

pub fn chi2_sf(x: f64, df: f64) -> f64 {
    if x <= 0.0 {
        return 1.0;
    }
    ChiSquared::new(df).map_or(f64::NAN, |d| d.sf(x))
}

pub fn chi2_cdf(x: f64, df: f64) -> f64 {
    1.0 - chi2_sf(x, df)
}

pub fn chi2_ppf(p: f64, df: f64) -> f64 {
    ChiSquared::new(df).map_or(f64::NAN, |d| d.inverse_cdf(p))
}

pub fn f_sf(x: f64, d1: f64, d2: f64) -> f64 {
    if x <= 0.0 {
        return 1.0;
    }
    FisherSnedecor::new(d1, d2).map_or(f64::NAN, |d| d.sf(x))
}

pub fn beta_cdf(x: f64, a: f64, b: f64) -> f64 {
    Beta::new(a, b).map_or(f64::NAN, |d| d.cdf(x))
}

pub fn beta_ppf(p: f64, a: f64, b: f64) -> f64 {
    Beta::new(a, b).map_or(f64::NAN, |d| d.inverse_cdf(p))
}

pub fn beta_pdf(x: f64, a: f64, b: f64) -> f64 {
    Beta::new(a, b).map_or(f64::NAN, |d| d.pdf(x))
}

pub fn ln_choose(n: f64, k: f64) -> f64 {
    ln_gamma(n + 1.0) - ln_gamma(k + 1.0) - ln_gamma(n - k + 1.0)
}

pub fn binom_pmf(k: f64, n: f64, p: f64) -> f64 {
    if k < 0.0 || k > n {
        return 0.0;
    }
    if p == 0.0 {
        return if k == 0.0 { 1.0 } else { 0.0 };
    }
    if p == 1.0 {
        return if k == n { 1.0 } else { 0.0 };
    }
    (ln_choose(n, k) + k * p.ln() + (n - k) * (1.0 - p).ln()).exp()
}

pub fn binom_cdf(k: f64, n: f64, p: f64) -> f64 {
    if k < 0.0 {
        return 0.0;
    }
    if k >= n {
        return 1.0;
    }
    // regularized incomplete beta identity
    1.0 - beta_cdf(p, k.floor() + 1.0, n - k.floor())
}

pub fn poisson_pmf(k: f64, lam: f64) -> f64 {
    if k < 0.0 {
        return 0.0;
    }
    (k * lam.ln() - lam - ln_gamma(k + 1.0)).exp()
}

pub fn poisson_cdf(k: f64, lam: f64) -> f64 {
    if k < 0.0 {
        return 0.0;
    }
    statrs::function::gamma::gamma_ur(k.floor() + 1.0, lam)
}

// p-value from a z score
pub fn p_from_z(z: f64, alt: Alt) -> f64 {
    match alt {
        Alt::Two => (2.0 * norm_sf(z.abs())).min(1.0),
        Alt::Less => norm_cdf(z),
        Alt::Greater => norm_sf(z),
    }
}

pub fn p_from_t(t: f64, df: f64, alt: Alt) -> f64 {
    match alt {
        Alt::Two => (2.0 * t_sf(t.abs(), df)).min(1.0),
        Alt::Less => t_cdf(t, df),
        Alt::Greater => t_sf(t, df),
    }
}

// Kolmogorov limit distribution, survival function
pub fn kolmogorov_sf(x: f64) -> f64 {
    if x <= 0.0 {
        return 1.0;
    }
    if x < 0.3 {
        // small x: use the theta-function form for accuracy
        let w = (2.0 * std::f64::consts::PI).sqrt() / x;
        let mut s = 0.0;
        for k in 1..=50 {
            let kk = (2 * k - 1) as f64;
            s += (-(kk * kk) * std::f64::consts::PI * std::f64::consts::PI / (8.0 * x * x)).exp();
        }
        return 1.0 - w * s;
    }
    let mut s = 0.0;
    for k in 1..=100 {
        let term = (-2.0 * (k * k) as f64 * x * x).exp();
        s += if k % 2 == 1 { term } else { -term };
        if term < 1e-17 {
            break;
        }
    }
    (2.0 * s).clamp(0.0, 1.0)
}

fn mat_mul(a: &[f64], b: &[f64], m: usize) -> Vec<f64> {
    let mut c = vec![0.0; m * m];
    for i in 0..m {
        for k in 0..m {
            let x = a[i * m + k];
            if x == 0.0 {
                continue;
            }
            for j in 0..m {
                c[i * m + j] += x * b[k * m + j];
            }
        }
    }
    c
}

// matrix power with a power-of-ten exponent kept aside to avoid overflow
fn mat_pow(a: &[f64], m: usize, n: usize) -> (Vec<f64>, i32) {
    if n == 1 {
        return (a.to_vec(), 0);
    }
    let (v, ev) = mat_pow(a, m, n / 2);
    let b = mat_mul(&v, &v, m);
    let (mut out, mut e) = if n.is_multiple_of(2) { (b, 2 * ev) } else { (mat_mul(a, &b, m), 2 * ev) };
    if out[(m / 2) * m + m / 2] > 1e140 {
        out.iter_mut().for_each(|x| *x *= 1e-140);
        e += 140;
    }
    (out, e)
}

// exact one-sample Kolmogorov D_n survival (Marsaglia, Tsang, Wang 2003)
pub fn kstwo_sf(d: f64, n: usize) -> f64 {
    if n == 0 || d >= 1.0 {
        return if d >= 1.0 { 0.0 } else { 1.0 };
    }
    if d <= 0.5 / n as f64 {
        return 1.0;
    }
    let nf = n as f64;
    let k = (nf * d) as usize + 1;
    let m = 2 * k - 1;
    if m > 600 {
        return kolmogorov_sf(nf.sqrt() * d);
    }
    let h = k as f64 - nf * d;
    let mut hm = vec![0.0; m * m];
    for i in 0..m {
        for j in 0..m {
            hm[i * m + j] = if i as i64 - j as i64 + 1 >= 0 { 1.0 } else { 0.0 };
        }
    }
    for i in 0..m {
        hm[i * m] -= h.powi(i as i32 + 1);
        hm[(m - 1) * m + i] -= h.powi((m - i) as i32);
    }
    if 2.0 * h - 1.0 > 0.0 {
        hm[(m - 1) * m] += (2.0 * h - 1.0).powi(m as i32);
    }
    for i in 0..m {
        for j in 0..m {
            let span = i as i64 - j as i64 + 1;
            if span > 0 {
                for g in 1..=span {
                    hm[i * m + j] /= g as f64;
                }
            }
        }
    }
    let (q, mut eq) = mat_pow(&hm, m, n);
    let mut s = q[(k - 1) * m + k - 1];
    for i in 1..=n {
        s = s * i as f64 / nf;
        if s < 1e-140 {
            s *= 1e140;
            eq -= 140;
        }
    }
    (1.0 - s * 10f64.powi(eq)).clamp(0.0, 1.0)
}
