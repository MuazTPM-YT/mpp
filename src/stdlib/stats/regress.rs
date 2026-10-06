// linear regression: simple and multiple (OLS)
use super::desc::{mean, sum};
use super::dist::{f_sf, p_from_t, t_ppf};
use super::tests::Rec;
use crate::vm::Value;

pub fn linregress(x: &[f64], y: &[f64]) -> Result<Rec, String> {
    if x.len() != y.len() {
        return Err(format!("lengths differ: {} and {}", x.len(), y.len()));
    }
    if x.len() < 3 {
        return Err("linregress needs at least 3 points".into());
    }
    let n = x.len() as f64;
    let (mx, my) = (mean(x), mean(y));
    let sxx = sum(&x.iter().map(|v| (v - mx).powi(2)).collect::<Vec<_>>());
    let sxy = sum(&x.iter().zip(y).map(|(a, b)| (a - mx) * (b - my)).collect::<Vec<_>>());
    let syy = sum(&y.iter().map(|v| (v - my).powi(2)).collect::<Vec<_>>());
    if sxx == 0.0 {
        return Err("all x values are the same".into());
    }
    let slope = sxy / sxx;
    let intercept = my - slope * mx;
    let r = if syy == 0.0 { 0.0 } else { (sxy / (sxx * syy).sqrt()).clamp(-1.0, 1.0) };
    let df = n - 2.0;
    let sse = (syy - slope * sxy).max(0.0);
    let se_slope = (sse / df / sxx).sqrt();
    let se_int = se_slope * (sum(&x.iter().map(|v| v * v).collect::<Vec<_>>()) / n).sqrt();
    let p = if se_slope == 0.0 { 0.0 } else { p_from_t(slope / se_slope, df, super::dist::Alt::Two) };
    Ok(Rec::default()
        .text("method", "simple linear regression")
        .num("slope", slope)
        .num("intercept", intercept)
        .num("r", r)
        .num("r2", r * r)
        .num("p_value", p)
        .num("stderr", se_slope)
        .num("intercept_stderr", se_int)
        .int("n", n as i64))
}

// Cholesky of a symmetric positive definite matrix
fn cholesky(a: &[Vec<f64>]) -> Option<Vec<Vec<f64>>> {
    let k = a.len();
    let mut l = vec![vec![0.0; k]; k];
    for i in 0..k {
        for j in 0..=i {
            let s: f64 = (0..j).map(|m| l[i][m] * l[j][m]).sum();
            if i == j {
                let d = a[i][i] - s;
                if d <= 1e-12 * a[i][i].abs().max(1e-300) {
                    return None;
                }
                l[i][j] = d.sqrt();
            } else {
                l[i][j] = (a[i][j] - s) / l[j][j];
            }
        }
    }
    Some(l)
}

// solve L L^T x = b
fn chol_solve(l: &[Vec<f64>], b: &[f64]) -> Vec<f64> {
    let k = l.len();
    let mut y = vec![0.0; k];
    for i in 0..k {
        y[i] = (b[i] - (0..i).map(|m| l[i][m] * y[m]).sum::<f64>()) / l[i][i];
    }
    let mut x = vec![0.0; k];
    for i in (0..k).rev() {
        x[i] = (y[i] - (i + 1..k).map(|m| l[m][i] * x[m]).sum::<f64>()) / l[i][i];
    }
    x
}

// y ~ columns; intercept added first when asked
pub fn ols(y: &[f64], cols: &[Vec<f64>], names: &[String], intercept: bool, level: f64) -> Result<Rec, String> {
    let n = y.len();
    if cols.iter().any(|c| c.len() != n) {
        return Err("every x column must have the same length as y".into());
    }
    let mut xs: Vec<Vec<f64>> = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    if intercept {
        xs.push(vec![1.0; n]);
        labels.push("intercept".into());
    }
    xs.extend(cols.iter().cloned());
    labels.extend(names.iter().cloned());
    let k = xs.len();
    if n <= k {
        return Err(format!("need more rows ({n}) than coefficients ({k})"));
    }
    let xtx: Vec<Vec<f64>> = (0..k).map(|i| (0..k).map(|j| (0..n).map(|r| xs[i][r] * xs[j][r]).sum()).collect()).collect();
    let xty: Vec<f64> = (0..k).map(|i| (0..n).map(|r| xs[i][r] * y[r]).sum()).collect();
    let l = cholesky(&xtx).ok_or("x columns are collinear (one is a mix of the others); drop one")?;
    let beta = chol_solve(&l, &xty);
    let fitted: Vec<f64> = (0..n).map(|r| (0..k).map(|i| xs[i][r] * beta[i]).sum()).collect();
    let resid: Vec<f64> = y.iter().zip(&fitted).map(|(a, b)| a - b).collect();
    let sse: f64 = resid.iter().map(|e| e * e).sum();
    let my = mean(y);
    let sst: f64 = if intercept { y.iter().map(|v| (v - my).powi(2)).sum() } else { y.iter().map(|v| v * v).sum() };
    let df = (n - k) as f64;
    let sigma2 = sse / df;
    let q = t_ppf(0.5 + level / 2.0, df);
    let mut coefs = indexmap::IndexMap::new();
    for i in 0..k {
        let mut e = vec![0.0; k];
        e[i] = 1.0;
        let inv_ii = chol_solve(&l, &e)[i];
        let se = (sigma2 * inv_ii).sqrt();
        let t = beta[i] / se;
        let rec = Rec::default()
            .num("estimate", beta[i])
            .num("stderr", se)
            .num("t", t)
            .num("p_value", p_from_t(t, df, super::dist::Alt::Two))
            .pair("ci", beta[i] - q * se, beta[i] + q * se);
        coefs.insert(crate::vm::Key::Str(labels[i].as_str().into()), rec.value());
    }
    let r2 = 1.0 - sse / sst;
    let p_model = (k - intercept as usize) as f64;
    let adj = 1.0 - (1.0 - r2) * (n as f64 - intercept as usize as f64) / df;
    let f = ((sst - sse) / p_model) / (sse / df);
    Ok(Rec::default()
        .text("method", "ordinary least squares")
        .val("coef", Value::map(coefs))
        .num("r2", r2)
        .num("adj_r2", adj)
        .num("f", f)
        .num("f_p_value", if p_model > 0.0 { f_sf(f, p_model, df) } else { f64::NAN })
        .num("residual_se", sigma2.sqrt())
        .int("n", n as i64)
        .num("df_resid", df))
}
