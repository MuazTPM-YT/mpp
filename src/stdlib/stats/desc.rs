// plain number crunching shared by vec, table, stats, ab, ml

// compensated sum (Neumaier): stays exact-ish for long float lists
pub fn sum(xs: &[f64]) -> f64 {
    let mut s = 0.0;
    let mut c = 0.0;
    for &v in xs {
        let t = s + v;
        c += if s.abs() >= v.abs() { (s - t) + v } else { (v - t) + s };
        s = t;
    }
    s + c
}

pub fn mean(xs: &[f64]) -> f64 {
    if xs.is_empty() { f64::NAN } else { sum(xs) / xs.len() as f64 }
}

// variance, two-pass for accuracy; ddof 1 = sample
pub fn var(xs: &[f64], ddof: f64) -> f64 {
    let n = xs.len() as f64;
    if n - ddof <= 0.0 {
        return f64::NAN;
    }
    let m = mean(xs);
    let ss: Vec<f64> = xs.iter().map(|x| (x - m) * (x - m)).collect();
    sum(&ss) / (n - ddof)
}

pub fn std(xs: &[f64], ddof: f64) -> f64 {
    var(xs, ddof).sqrt()
}

pub fn sem(xs: &[f64]) -> f64 {
    std(xs, 1.0) / (xs.len() as f64).sqrt()
}

pub fn sorted(xs: &[f64]) -> Vec<f64> {
    let mut v = xs.to_vec();
    v.sort_by(|a, b| a.total_cmp(b));
    v
}

// linear interpolation (numpy default) on sorted data
pub fn quantile_sorted(s: &[f64], q: f64) -> f64 {
    if s.is_empty() || !(0.0..=1.0).contains(&q) {
        return f64::NAN;
    }
    let h = (s.len() - 1) as f64 * q;
    let lo = h.floor() as usize;
    let hi = h.ceil() as usize;
    s[lo] + (h - lo as f64) * (s[hi] - s[lo])
}

pub fn quantile(xs: &[f64], q: f64) -> f64 {
    quantile_sorted(&sorted(xs), q)
}

pub fn median(xs: &[f64]) -> f64 {
    quantile(xs, 0.5)
}

pub fn min(xs: &[f64]) -> f64 {
    xs.iter().copied().fold(f64::NAN, |a, b| if a.is_nan() || b < a { b } else { a })
}

pub fn max(xs: &[f64]) -> f64 {
    xs.iter().copied().fold(f64::NAN, |a, b| if a.is_nan() || b > a { b } else { a })
}

// population skewness (scipy default, bias=True)
pub fn skew(xs: &[f64]) -> f64 {
    let m = mean(xs);
    let n = xs.len() as f64;
    let m2 = xs.iter().map(|x| (x - m).powi(2)).sum::<f64>() / n;
    let m3 = xs.iter().map(|x| (x - m).powi(3)).sum::<f64>() / n;
    m3 / m2.powf(1.5)
}

// excess kurtosis (scipy default: fisher, bias=True)
pub fn kurtosis(xs: &[f64]) -> f64 {
    let m = mean(xs);
    let n = xs.len() as f64;
    let m2 = xs.iter().map(|x| (x - m).powi(2)).sum::<f64>() / n;
    let m4 = xs.iter().map(|x| (x - m).powi(4)).sum::<f64>() / n;
    m4 / (m2 * m2) - 3.0
}

// average ranks, 1-based; ties share the mean rank
pub fn rank(xs: &[f64]) -> Vec<f64> {
    let mut idx: Vec<usize> = (0..xs.len()).collect();
    idx.sort_by(|&a, &b| xs[a].total_cmp(&xs[b]));
    let mut r = vec![0.0; xs.len()];
    let mut i = 0;
    while i < idx.len() {
        let mut j = i;
        while j + 1 < idx.len() && xs[idx[j + 1]] == xs[idx[i]] {
            j += 1;
        }
        let avg = (i + j) as f64 / 2.0 + 1.0;
        for k in i..=j {
            r[idx[k]] = avg;
        }
        i = j + 1;
    }
    r
}

// sizes of tie groups (for tie corrections)
pub fn tie_counts(xs: &[f64]) -> Vec<f64> {
    let s = sorted(xs);
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        let mut j = i;
        while j + 1 < s.len() && s[j + 1] == s[i] {
            j += 1;
        }
        if j > i {
            out.push((j - i + 1) as f64);
        }
        i = j + 1;
    }
    out
}

pub fn cov(x: &[f64], y: &[f64]) -> f64 {
    let (mx, my) = (mean(x), mean(y));
    let p: Vec<f64> = x.iter().zip(y).map(|(a, b)| (a - mx) * (b - my)).collect();
    sum(&p) / (x.len() as f64 - 1.0)
}

pub fn pearson_r(x: &[f64], y: &[f64]) -> f64 {
    let (mx, my) = (mean(x), mean(y));
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for (a, b) in x.iter().zip(y) {
        sxy += (a - mx) * (b - my);
        sxx += (a - mx) * (a - mx);
        syy += (b - my) * (b - my);
    }
    (sxy / (sxx * syy).sqrt()).clamp(-1.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basics() {
        let x = [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        assert_eq!(mean(&x), 5.0);
        assert_eq!(var(&x, 0.0), 4.0);
        assert_eq!(median(&x), 4.5);
        assert_eq!(quantile(&[1.0, 2.0, 3.0, 4.0], 0.25), 1.75);
        assert_eq!(rank(&[10.0, 20.0, 10.0]), vec![1.5, 3.0, 1.5]);
        assert_eq!(sum(&[0.1; 10]), 1.0);
    }
}
