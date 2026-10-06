// is model 2 really better than model 1?
use crate::stdlib::stats::desc;
use crate::stdlib::stats::dist::{Alt, binom_cdf, chi2_sf, norm_sf, p_from_t};

// McNemar on disagreements; exact binomial when few
pub fn mcnemar(c1: &[bool], c2: &[bool], exact: Option<bool>) -> (f64, f64, f64, f64, bool) {
    let b = c1.iter().zip(c2).filter(|(a, b)| **a && !**b).count() as f64;
    let c = c1.iter().zip(c2).filter(|(a, b)| !**a && **b).count() as f64;
    let exact = exact.unwrap_or(b + c < 25.0);
    if b + c == 0.0 {
        return (0.0, 1.0, b, c, exact);
    }
    if exact {
        let k = b.min(c);
        (k, (2.0 * binom_cdf(k, b + c, 0.5)).min(1.0), b, c, true)
    } else {
        let stat = ((b - c).abs() - 1.0).max(0.0).powi(2) / (b + c);
        (stat, chi2_sf(stat, 1.0), b, c, false)
    }
}

// DeLong test for two correlated AUCs (midrank method)
pub fn delong(y: &[bool], s1: &[f64], s2: &[f64]) -> (f64, f64, f64, f64) {
    let pos: Vec<usize> = (0..y.len()).filter(|&i| y[i]).collect();
    let neg: Vec<usize> = (0..y.len()).filter(|&i| !y[i]).collect();
    let (m, n) = (pos.len() as f64, neg.len() as f64);
    let comps = |s: &[f64]| -> (f64, Vec<f64>, Vec<f64>) {
        let x: Vec<f64> = pos.iter().map(|&i| s[i]).collect();
        let yv: Vec<f64> = neg.iter().map(|&i| s[i]).collect();
        let z: Vec<f64> = x.iter().chain(&yv).copied().collect();
        let (tx, ty, tz) = (desc::rank(&x), desc::rank(&yv), desc::rank(&z));
        let v10: Vec<f64> = (0..x.len()).map(|i| (tz[i] - tx[i]) / n).collect();
        let v01: Vec<f64> = (0..yv.len()).map(|j| 1.0 - (tz[x.len() + j] - ty[j]) / m).collect();
        (desc::mean(&v10), v10, v01)
    };
    let (a1, v10a, v01a) = comps(s1);
    let (a2, v10b, v01b) = comps(s2);
    let cov = |a: &[f64], b: &[f64]| if a.len() < 2 { 0.0 } else { desc::cov(a, b) };
    let var = (cov(&v10a, &v10a) + cov(&v10b, &v10b) - 2.0 * cov(&v10a, &v10b)) / m
        + (cov(&v01a, &v01a) + cov(&v01b, &v01b) - 2.0 * cov(&v01a, &v01b)) / n;
    if var <= 0.0 {
        return (a1, a2, 0.0, 1.0);
    }
    let z = (a2 - a1) / var.sqrt();
    (a1, a2, z, (2.0 * norm_sf(z.abs())).min(1.0))
}

// Dietterich 5x2cv paired t-test; d[i][j] = score diff in repeat i, fold j
pub fn cv5x2(d: &[Vec<f64>]) -> Result<(f64, f64), String> {
    if d.len() != 5 || d.iter().any(|r| r.len() != 2) {
        return Err("5x2cv needs 5 rows of 2 differences".into());
    }
    let s2: f64 = d
        .iter()
        .map(|r| {
            let m = (r[0] + r[1]) / 2.0;
            (r[0] - m).powi(2) + (r[1] - m).powi(2)
        })
        .sum();
    if s2 == 0.0 {
        return Err("all differences are equal; t is undefined".into());
    }
    let t = d[0][0] / (s2 / 5.0).sqrt();
    Ok((t, p_from_t(t, 5.0, Alt::Two)))
}
