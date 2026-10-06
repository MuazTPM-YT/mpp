// distribution shift between a reference sample and a current one
use crate::stdlib::stats::desc;

// population stability index; bins cut at reference quantiles
pub fn psi(reference: &[f64], current: &[f64], bins: usize) -> (f64, Vec<f64>) {
    let s = desc::sorted(reference);
    let mut edges: Vec<f64> = (1..bins).map(|i| desc::quantile_sorted(&s, i as f64 / bins as f64)).collect();
    edges.dedup();
    let share = |x: &[f64]| -> Vec<f64> {
        let mut c = vec![0.0; edges.len() + 1];
        for v in x {
            c[edges.partition_point(|e| e < v)] += 1.0;
        }
        c.iter().map(|n| (n / x.len() as f64).max(1e-4)).collect()
    };
    let (e, a) = (share(reference), share(current));
    let v = e.iter().zip(&a).map(|(e, a)| (a - e) * (a / e).ln()).sum();
    (v, edges)
}

// psi over category shares
pub fn psi_shares(e: &[f64], a: &[f64]) -> f64 {
    e.iter()
        .zip(a)
        .map(|(e, a)| {
            let (e, a) = (e.max(1e-4), a.max(1e-4));
            (a - e) * (a / e).ln()
        })
        .sum()
}

pub fn psi_level(p: f64) -> &'static str {
    if p < 0.1 {
        "none"
    } else if p < 0.25 {
        "moderate"
    } else {
        "major"
    }
}

// 1-D earth mover's distance between samples (scipy algorithm)
pub fn wasserstein(a: &[f64], b: &[f64]) -> f64 {
    let (sa, sb) = (desc::sorted(a), desc::sorted(b));
    let all = desc::sorted(&sa.iter().chain(&sb).copied().collect::<Vec<_>>());
    let mut d = 0.0;
    for w in all.windows(2) {
        let x = w[0];
        let fa = sa.partition_point(|v| *v <= x) as f64 / sa.len() as f64;
        let fb = sb.partition_point(|v| *v <= x) as f64 / sb.len() as f64;
        d += (fa - fb).abs() * (w[1] - w[0]);
    }
    d
}

fn normalize(p: &[f64]) -> Vec<f64> {
    let s: f64 = p.iter().sum();
    p.iter().map(|x| x / s).collect()
}

// KL(p || q) in nats (scipy.stats.entropy)
pub fn kl(p: &[f64], q: &[f64]) -> f64 {
    let (p, q) = (normalize(p), normalize(q));
    p.iter()
        .zip(&q)
        .map(|(a, b)| {
            if *a == 0.0 {
                0.0
            } else if *b == 0.0 {
                f64::INFINITY
            } else {
                a * (a / b).ln()
            }
        })
        .sum()
}

// Jensen-Shannon divergence in bits, 0..1
pub fn js(p: &[f64], q: &[f64]) -> f64 {
    let (p, q) = (normalize(p), normalize(q));
    let m: Vec<f64> = p.iter().zip(&q).map(|(a, b)| (a + b) / 2.0).collect();
    let part = |x: &[f64]| x.iter().zip(&m).map(|(a, b)| if *a == 0.0 { 0.0 } else { a * (a / b).log2() }).sum::<f64>();
    ((part(&p) + part(&q)) / 2.0).max(0.0)
}
