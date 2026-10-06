// model metrics on plain Rust data; numbers follow scikit-learn
use crate::stdlib::stats::desc;

// confusion matrix: c[true][pred]
pub fn confusion(y: &[usize], p: &[usize], k: usize) -> Vec<Vec<f64>> {
    let mut c = vec![vec![0.0; k]; k];
    for (&t, &q) in y.iter().zip(p) {
        c[t][q] += 1.0;
    }
    c
}

pub struct ClassStats {
    pub precision: Vec<f64>,
    pub recall: Vec<f64>,
    pub f: Vec<f64>,
    pub support: Vec<f64>,
    pub predicted: Vec<f64>,
}

fn div0(a: f64, b: f64) -> f64 {
    if b == 0.0 { 0.0 } else { a / b }
}

pub fn per_class(c: &[Vec<f64>], beta: f64) -> ClassStats {
    let k = c.len();
    let b2 = beta * beta;
    let mut s =
        ClassStats { precision: vec![0.0; k], recall: vec![0.0; k], f: vec![0.0; k], support: vec![0.0; k], predicted: vec![0.0; k] };
    for (i, ci) in c.iter().enumerate() {
        let tp = ci[i];
        let row: f64 = ci.iter().sum();
        let col: f64 = c.iter().map(|r| r[i]).sum();
        s.precision[i] = div0(tp, col);
        s.recall[i] = div0(tp, row);
        s.f[i] = div0((1.0 + b2) * s.precision[i] * s.recall[i], b2 * s.precision[i] + s.recall[i]);
        s.support[i] = row;
        s.predicted[i] = col;
    }
    s
}

pub fn accuracy(c: &[Vec<f64>]) -> f64 {
    let n: f64 = c.iter().flatten().sum();
    div0((0..c.len()).map(|i| c[i][i]).sum(), n)
}

pub fn balanced_accuracy(c: &[Vec<f64>]) -> f64 {
    let s = per_class(c, 1.0);
    let r: Vec<f64> = (0..c.len()).filter(|&i| s.support[i] > 0.0).map(|i| s.recall[i]).collect();
    desc::mean(&r)
}

pub fn mcc(c: &[Vec<f64>]) -> f64 {
    let k = c.len();
    let s: f64 = c.iter().flatten().sum();
    let correct: f64 = (0..k).map(|i| c[i][i]).sum();
    let t: Vec<f64> = (0..k).map(|i| c[i].iter().sum()).collect();
    let p: Vec<f64> = (0..k).map(|j| (0..k).map(|i| c[i][j]).sum()).collect();
    let num = correct * s - t.iter().zip(&p).map(|(a, b)| a * b).sum::<f64>();
    let den = ((s * s - p.iter().map(|x| x * x).sum::<f64>()) * (s * s - t.iter().map(|x| x * x).sum::<f64>())).sqrt();
    div0(num, den)
}

pub fn kappa(c: &[Vec<f64>]) -> f64 {
    let k = c.len();
    let n: f64 = c.iter().flatten().sum();
    let po = accuracy(c);
    let pe: f64 = (0..k).map(|i| c[i].iter().sum::<f64>() * (0..k).map(|r| c[r][i]).sum::<f64>()).sum::<f64>() / (n * n);
    div0(po - pe, 1.0 - pe)
}

// AUC by ranks (ties averaged) = Mann-Whitney U / (n1 n0)
pub fn roc_auc(y: &[bool], s: &[f64]) -> f64 {
    let r = desc::rank(s);
    let n1 = y.iter().filter(|b| **b).count() as f64;
    let n0 = y.len() as f64 - n1;
    if n1 == 0.0 || n0 == 0.0 {
        return f64::NAN;
    }
    let sum: f64 = y.iter().zip(&r).filter(|(b, _)| **b).map(|(_, r)| r).sum();
    (sum - n1 * (n1 + 1.0) / 2.0) / (n1 * n0)
}

// distinct thresholds, high to low, with running tp/fp
fn sweep(y: &[bool], s: &[f64]) -> Vec<(f64, f64, f64)> {
    let mut idx: Vec<usize> = (0..s.len()).collect();
    idx.sort_by(|&a, &b| s[b].total_cmp(&s[a]));
    let mut out = Vec::new();
    let (mut tp, mut fp) = (0.0, 0.0);
    for (n, &i) in idx.iter().enumerate() {
        if y[i] {
            tp += 1.0
        } else {
            fp += 1.0
        }
        let last = n + 1 == idx.len() || s[idx[n + 1]] != s[i];
        if last {
            out.push((s[i], tp, fp));
        }
    }
    out
}

pub fn roc_curve(y: &[bool], s: &[f64]) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let pos = y.iter().filter(|b| **b).count() as f64;
    let neg = y.len() as f64 - pos;
    let (mut fpr, mut tpr, mut th) = (vec![0.0], vec![0.0], vec![f64::INFINITY]);
    for (t, tp, fp) in sweep(y, s) {
        fpr.push(div0(fp, neg));
        tpr.push(div0(tp, pos));
        th.push(t);
    }
    (fpr, tpr, th)
}

// precision, recall, threshold at each distinct score
pub fn pr_curve(y: &[bool], s: &[f64]) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let pos = y.iter().filter(|b| **b).count() as f64;
    let (mut p, mut r, mut th) = (Vec::new(), Vec::new(), Vec::new());
    for (t, tp, fp) in sweep(y, s) {
        p.push(div0(tp, tp + fp));
        r.push(div0(tp, pos));
        th.push(t);
    }
    (p, r, th)
}

// sum over thresholds of (R_n - R_{n-1}) * P_n
pub fn average_precision(y: &[bool], s: &[f64]) -> f64 {
    let (p, r, _) = pr_curve(y, s);
    let mut prev = 0.0;
    let mut ap = 0.0;
    for (pi, ri) in p.iter().zip(&r) {
        ap += (ri - prev) * pi;
        prev = *ri;
    }
    ap
}

pub fn log_loss_binary(y: &[bool], p: &[f64]) -> f64 {
    let eps = f64::EPSILON;
    let s: f64 = y
        .iter()
        .zip(p)
        .map(|(t, q)| {
            let q = q.clamp(eps, 1.0 - eps);
            if *t { -q.ln() } else { -(1.0 - q).ln() }
        })
        .sum();
    s / y.len() as f64
}

// rows of class probabilities (clipped, not re-normalised, like sklearn >= 1.5)
pub fn log_loss_multi(y: &[usize], probs: &[Vec<f64>]) -> f64 {
    let eps = f64::EPSILON;
    let s: f64 = y.iter().zip(probs).map(|(&t, row)| -row[t].clamp(eps, 1.0 - eps).ln()).sum();
    s / y.len() as f64
}

pub fn brier(y: &[bool], p: &[f64]) -> f64 {
    y.iter().zip(p).map(|(t, q)| (q - if *t { 1.0 } else { 0.0 }).powi(2)).sum::<f64>() / y.len() as f64
}

pub fn top_k(y: &[usize], scores: &[Vec<f64>], k: usize) -> f64 {
    let hits = y
        .iter()
        .zip(scores)
        .filter(|(t, row)| {
            let mine = row[**t];
            // rank = how many classes score strictly higher
            row.iter().filter(|v| **v > mine).count() < k
        })
        .count();
    hits as f64 / y.len() as f64
}

pub struct RegMetrics {
    pub mae: f64,
    pub mse: f64,
    pub rmse: f64,
    pub r2: f64,
    pub mape: f64,
    pub smape: f64,
    pub median_ae: f64,
    pub max_error: f64,
    pub explained_variance: f64,
}

pub fn regression(y: &[f64], p: &[f64]) -> RegMetrics {
    let n = y.len() as f64;
    let e: Vec<f64> = y.iter().zip(p).map(|(a, b)| a - b).collect();
    let ae: Vec<f64> = e.iter().map(|x| x.abs()).collect();
    let mse = desc::sum(&e.iter().map(|x| x * x).collect::<Vec<_>>()) / n;
    let my = desc::mean(y);
    let sst = desc::sum(&y.iter().map(|v| (v - my).powi(2)).collect::<Vec<_>>());
    let sse = mse * n;
    let r2 = if sst == 0.0 { if sse == 0.0 { 1.0 } else { 0.0 } } else { 1.0 - sse / sst };
    let vy = desc::var(y, 0.0);
    RegMetrics {
        mae: desc::mean(&ae),
        mse,
        rmse: mse.sqrt(),
        r2,
        mape: y.iter().zip(&ae).map(|(t, a)| a / t.abs().max(f64::EPSILON)).sum::<f64>() / n,
        smape: y
            .iter()
            .zip(p)
            .map(|(t, q)| if t.abs() + q.abs() == 0.0 { 0.0 } else { 2.0 * (t - q).abs() / (t.abs() + q.abs()) })
            .sum::<f64>()
            / n,
        median_ae: desc::median(&ae),
        max_error: desc::max(&ae),
        explained_variance: if vy == 0.0 { 1.0 } else { 1.0 - desc::var(&e, 0.0) / vy },
    }
}

// ---- ranking: one query = relevances in the order the model ranked them ----

pub fn dcg(rel: &[f64], k: usize, exp_gain: bool) -> f64 {
    rel.iter().take(k).enumerate().map(|(i, r)| (if exp_gain { 2f64.powf(*r) - 1.0 } else { *r }) / (i as f64 + 2.0).log2()).sum()
}

pub fn ndcg(rel: &[f64], k: usize, exp_gain: bool) -> f64 {
    let mut ideal = rel.to_vec();
    ideal.sort_by(|a, b| b.total_cmp(a));
    let id = dcg(&ideal, k, exp_gain);
    if id == 0.0 { 0.0 } else { dcg(rel, k, exp_gain) / id }
}

pub fn precision_at(rel: &[f64], k: usize) -> f64 {
    rel.iter().take(k).filter(|r| **r > 0.0).count() as f64 / k as f64
}

pub fn recall_at(rel: &[f64], k: usize) -> f64 {
    let total = rel.iter().filter(|r| **r > 0.0).count() as f64;
    div0(rel.iter().take(k).filter(|r| **r > 0.0).count() as f64, total)
}

pub fn hit_at(rel: &[f64], k: usize) -> f64 {
    rel.iter().take(k).any(|r| *r > 0.0) as i64 as f64
}

pub fn reciprocal_rank(rel: &[f64], k: usize) -> f64 {
    rel.iter().take(k).position(|r| *r > 0.0).map_or(0.0, |i| 1.0 / (i as f64 + 1.0))
}

pub fn average_precision_at(rel: &[f64], k: usize) -> f64 {
    let total = rel.iter().filter(|r| **r > 0.0).count();
    if total == 0 {
        return 0.0;
    }
    let mut hits = 0.0;
    let mut s = 0.0;
    for (i, r) in rel.iter().take(k).enumerate() {
        if *r > 0.0 {
            hits += 1.0;
            s += hits / (i as f64 + 1.0);
        }
    }
    s / total.min(k) as f64
}

pub struct CalBin {
    pub lo: f64,
    pub hi: f64,
    pub count: usize,
    pub mean_pred: f64,
    pub frac_pos: f64,
}

// equal-width (or equal-count) bins of predicted probability
pub fn calibration(y: &[bool], p: &[f64], bins: usize, quantile: bool) -> (Vec<CalBin>, f64, f64) {
    let edges: Vec<f64> = if quantile {
        let s = desc::sorted(p);
        (0..=bins).map(|i| desc::quantile_sorted(&s, i as f64 / bins as f64)).collect()
    } else {
        (0..=bins).map(|i| i as f64 / bins as f64).collect()
    };
    let mut out = Vec::new();
    let n = p.len() as f64;
    let (mut ece, mut mce) = (0.0f64, 0.0f64);
    for b in 0..bins {
        let (lo, hi) = (edges[b], edges[b + 1]);
        let last = b + 1 == bins;
        let idx: Vec<usize> = (0..p.len()).filter(|&i| p[i] >= lo && (p[i] < hi || (last && p[i] <= hi))).collect();
        if idx.is_empty() {
            continue;
        }
        let mp = idx.iter().map(|&i| p[i]).sum::<f64>() / idx.len() as f64;
        let fp = idx.iter().filter(|&&i| y[i]).count() as f64 / idx.len() as f64;
        let gap = (mp - fp).abs();
        ece += idx.len() as f64 / n * gap;
        mce = mce.max(gap);
        out.push(CalBin { lo, hi, count: idx.len(), mean_pred: mp, frac_pos: fp });
    }
    (out, ece, mce)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_cases() {
        let c = confusion(&[0, 0, 1, 1], &[0, 1, 1, 1], 2);
        assert_eq!(accuracy(&c), 0.75);
        assert_eq!(roc_auc(&[false, false, true, true], &[0.1, 0.4, 0.35, 0.8]), 0.75);
        assert!((ndcg(&[3.0, 2.0, 3.0, 0.0], 4, false) - 0.97779).abs() < 1e-4);
        assert_eq!(reciprocal_rank(&[0.0, 0.0, 1.0], 10), 1.0 / 3.0);
    }
}
