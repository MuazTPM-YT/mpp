// text quality metrics for model outputs (match nltk, rouge-score, sacrebleu)
use serde_json::Value as J;
use std::collections::HashMap;

// SQuAD-style normalise: lowercase, drop punctuation and articles, squeeze spaces
pub fn normalize(s: &str) -> String {
    let lower = s.to_lowercase();
    let no_punct: String = lower.chars().map(|c| if c.is_alphanumeric() || c.is_whitespace() { c } else { ' ' }).collect();
    no_punct.split_whitespace().filter(|w| !matches!(*w, "a" | "an" | "the")).collect::<Vec<_>>().join(" ")
}

// SQuAD token F1
pub fn token_f1(pred: &str, gold: &str) -> f64 {
    let p: Vec<String> = normalize(pred).split_whitespace().map(String::from).collect();
    let g: Vec<String> = normalize(gold).split_whitespace().map(String::from).collect();
    if p.is_empty() || g.is_empty() {
        return (p == g) as i64 as f64;
    }
    let mut counts: HashMap<&str, i64> = HashMap::new();
    for w in &g {
        *counts.entry(w).or_default() += 1;
    }
    let mut same = 0;
    for w in &p {
        if let Some(c) = counts.get_mut(w.as_str())
            && *c > 0
        {
            *c -= 1;
            same += 1;
        }
    }
    if same == 0 {
        return 0.0;
    }
    let (pr, rc) = (same as f64 / p.len() as f64, same as f64 / g.len() as f64);
    2.0 * pr * rc / (pr + rc)
}

fn ngrams<T: std::hash::Hash + Eq + Clone>(toks: &[T], n: usize) -> HashMap<Vec<T>, usize> {
    let mut m = HashMap::new();
    if toks.len() >= n {
        for w in toks.windows(n) {
            *m.entry(w.to_vec()).or_default() += 1;
        }
    }
    m
}

// sentence BLEU like nltk (whitespace tokens, uniform weights, closest ref length)
pub fn bleu(cand: &str, refs: &[String], max_n: usize, smooth: bool) -> f64 {
    let c: Vec<&str> = cand.split_whitespace().collect();
    let rs: Vec<Vec<&str>> = refs.iter().map(|r| r.split_whitespace().collect()).collect();
    if c.is_empty() || rs.is_empty() {
        return 0.0;
    }
    let mut logs = 0.0;
    for n in 1..=max_n {
        let cn = ngrams(&c, n);
        let mut maxref: HashMap<Vec<&str>, usize> = HashMap::new();
        for r in &rs {
            for (g, k) in ngrams(r, n) {
                let e = maxref.entry(g).or_default();
                *e = (*e).max(k);
            }
        }
        let clipped: usize = cn.iter().map(|(g, k)| (*k).min(*maxref.get(g).unwrap_or(&0))).sum();
        let total: usize = cn.values().sum();
        // like nltk: no unigram match at all = 0; other empty orders get a tiny value (or 0.1/n smoothed)
        if n == 1 && clipped == 0 {
            return 0.0;
        }
        let p =
            if clipped == 0 { if smooth { 0.1 / total.max(1) as f64 } else { f64::MIN_POSITIVE } } else { clipped as f64 / total as f64 };
        logs += p.ln() / max_n as f64;
    }
    let cl = c.len() as f64;
    let rl = rs.iter().map(|r| r.len() as f64).min_by(|a, b| ((a - cl).abs(), *a).partial_cmp(&((b - cl).abs(), *b)).unwrap()).unwrap();
    let bp = if cl > rl { 1.0 } else { (1.0 - rl / cl).exp() };
    bp * logs.exp()
}

// rouge-score tokenizer: lowercase, non-alphanumeric to space
pub fn rouge_tokens(s: &str) -> Vec<String> {
    let lower = s.to_lowercase();
    let cleaned: String = lower.chars().map(|c| if c.is_ascii_lowercase() || c.is_ascii_digit() { c } else { ' ' }).collect();
    cleaned.split_whitespace().map(String::from).collect()
}

fn prf(overlap: f64, nc: f64, nr: f64) -> (f64, f64, f64) {
    let p = if nc > 0.0 { overlap / nc } else { 0.0 };
    let r = if nr > 0.0 { overlap / nr } else { 0.0 };
    let f = if p + r > 0.0 { 2.0 * p * r / (p + r) } else { 0.0 };
    (p, r, f)
}

pub fn rouge_n(cand: &str, reference: &str, n: usize) -> (f64, f64, f64) {
    let (c, r) = (rouge_tokens(cand), rouge_tokens(reference));
    let (cn, rn) = (ngrams(&c, n), ngrams(&r, n));
    let overlap: usize = cn.iter().map(|(g, k)| (*k).min(*rn.get(g).unwrap_or(&0))).sum();
    prf(overlap as f64, cn.values().sum::<usize>() as f64, rn.values().sum::<usize>() as f64)
}

pub fn rouge_l(cand: &str, reference: &str) -> (f64, f64, f64) {
    let (c, r) = (rouge_tokens(cand), rouge_tokens(reference));
    let mut dp = vec![vec![0usize; r.len() + 1]; c.len() + 1];
    for i in 1..=c.len() {
        for j in 1..=r.len() {
            dp[i][j] = if c[i - 1] == r[j - 1] { dp[i - 1][j - 1] + 1 } else { dp[i - 1][j].max(dp[i][j - 1]) };
        }
    }
    prf(dp[c.len()][r.len()] as f64, c.len() as f64, r.len() as f64)
}

// chrF (sacrebleu defaults: 6 char orders, beta 2, spaces removed); best over refs
pub fn chrf(cand: &str, refs: &[String], order: usize, beta: f64) -> f64 {
    let strip = |s: &str| -> Vec<char> { s.chars().filter(|c| !c.is_whitespace()).collect() };
    let h = strip(cand);
    let mut best = 0.0f64;
    for r in refs {
        let rr = strip(r);
        let (mut ap, mut ar, mut eff) = (0.0, 0.0, 0);
        for n in 1..=order {
            let (hn, rn) = (ngrams(&h, n), ngrams(&rr, n));
            let (th, tr) = (hn.values().sum::<usize>(), rn.values().sum::<usize>());
            let m: usize = hn.iter().map(|(g, k)| (*k).min(*rn.get(g).unwrap_or(&0))).sum();
            if th > 0 && tr > 0 {
                ap += m as f64 / th as f64;
                ar += m as f64 / tr as f64;
                eff += 1;
            }
        }
        if eff == 0 {
            continue;
        }
        let (p, rc) = (ap / eff as f64, ar / eff as f64);
        let b2 = beta * beta;
        if p + rc > 0.0 {
            best = best.max(100.0 * (1.0 + b2) * p * rc / (b2 * p + rc));
        }
    }
    best
}

pub fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + (a[i - 1] != b[j - 1]) as usize);
        }
        prev = cur;
    }
    prev[b.len()]
}

pub fn similarity(a: &str, b: &str) -> f64 {
    let n = a.chars().count().max(b.chars().count());
    if n == 0 { 1.0 } else { 1.0 - edit_distance(a, b) as f64 / n as f64 }
}

// unbiased pass@k (Chen et al. 2021)
pub fn pass_at_k(n: u64, c: u64, k: u64) -> f64 {
    if n < k || n - c < k {
        return 1.0;
    }
    let mut prod = 1.0;
    for i in (n - c + 1)..=n {
        prod *= 1.0 - k as f64 / i as f64;
    }
    1.0 - prod
}

const REFUSALS: &[&str] = &[
    "i can't",
    "i cannot",
    "i can not",
    "i won't",
    "i will not",
    "i'm sorry",
    "i am sorry",
    "i apologize",
    "as an ai",
    "i'm not able to",
    "i am not able to",
    "i'm unable to",
    "i am unable to",
    "cannot help with",
    "can't help with",
    "not able to help",
    "i must decline",
];

pub fn is_refusal(text: &str) -> bool {
    let t = text.to_lowercase().replace('’', "'");
    REFUSALS.iter().any(|p| t.contains(p))
}

// small JSON Schema subset: type, required, properties, items, enum, minimum, maximum,
// minLength, maxLength, additionalProperties=false
pub fn schema_errors(v: &J, s: &J, path: &str, out: &mut Vec<String>) {
    let at = if path.is_empty() { "value".to_string() } else { path.to_string() };
    if let Some(t) = s.get("type") {
        let types: Vec<&str> = match t {
            J::String(x) => vec![x.as_str()],
            J::Array(a) => a.iter().filter_map(|x| x.as_str()).collect(),
            _ => vec![],
        };
        let ok = types.iter().any(|t| match *t {
            "object" => v.is_object(),
            "array" => v.is_array(),
            "string" => v.is_string(),
            "number" => v.is_number(),
            "integer" => v.as_f64().is_some_and(|x| x.fract() == 0.0),
            "boolean" => v.is_boolean(),
            "null" => v.is_null(),
            _ => true,
        });
        if !ok {
            out.push(format!("{at}: expected {}, got {}", types.join(" or "), kind(v)));
            return;
        }
    }
    if let Some(e) = s.get("enum").and_then(|e| e.as_array())
        && !e.contains(v)
    {
        out.push(format!("{at}: {} is not one of {}", v, J::Array(e.clone())));
    }
    if let Some(x) = v.as_f64() {
        if let Some(m) = s.get("minimum").and_then(|m| m.as_f64()).filter(|m| x < *m) {
            out.push(format!("{at}: {x} is below minimum {m}"));
        }
        if let Some(m) = s.get("maximum").and_then(|m| m.as_f64()).filter(|m| x > *m) {
            out.push(format!("{at}: {x} is above maximum {m}"));
        }
    }
    if let Some(st) = v.as_str() {
        let n = st.chars().count() as u64;
        if let Some(m) = s.get("minLength").and_then(|m| m.as_u64()).filter(|m| n < *m) {
            out.push(format!("{at}: shorter than {m} characters"));
        }
        if let Some(m) = s.get("maxLength").and_then(|m| m.as_u64()).filter(|m| n > *m) {
            out.push(format!("{at}: longer than {m} characters"));
        }
    }
    if let Some(obj) = v.as_object() {
        if let Some(req) = s.get("required").and_then(|r| r.as_array()) {
            for k in req.iter().filter_map(|k| k.as_str()) {
                if !obj.contains_key(k) {
                    out.push(format!("{at}: missing key \"{k}\""));
                }
            }
        }
        let props = s.get("properties").and_then(|p| p.as_object());
        if let Some(props) = props {
            for (k, sub) in props {
                if let Some(x) = obj.get(k) {
                    schema_errors(x, sub, &format!("{}{k}", if path.is_empty() { String::new() } else { format!("{path}.") }), out);
                }
            }
        }
        if s.get("additionalProperties") == Some(&J::Bool(false)) {
            for k in obj.keys() {
                if !props.is_some_and(|p| p.contains_key(k)) {
                    out.push(format!("{at}: unexpected key \"{k}\""));
                }
            }
        }
    }
    if let (Some(arr), Some(items)) = (v.as_array(), s.get("items")) {
        for (i, x) in arr.iter().enumerate() {
            schema_errors(x, items, &format!("{at}[{i}]"), out);
        }
    }
}

fn kind(v: &J) -> &'static str {
    match v {
        J::Null => "null",
        J::Bool(_) => "boolean",
        J::Number(_) => "number",
        J::String(_) => "string",
        J::Array(_) => "array",
        J::Object(_) => "object",
    }
}

// JSON inside a reply: strip ``` fences and surrounding prose
pub fn extract_json(text: &str) -> Option<J> {
    let t = text.trim();
    if let Ok(v) = serde_json::from_str::<J>(t) {
        return Some(v);
    }
    let inner = t.split("```").nth(1).map(|b| b.trim_start_matches("json").trim());
    if let Some(v) = inner.and_then(|b| serde_json::from_str::<J>(b).ok()) {
        return Some(v);
    }
    let (s, e) = (t.find(['{', '['])?, t.rfind(['}', ']'])?);
    if e > s { serde_json::from_str(&t[s..=e]).ok() } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basics() {
        assert_eq!(normalize("The  Cat, sat!"), "cat sat");
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert!((pass_at_k(10, 3, 1) - 0.3).abs() < 1e-12);
        assert!(is_refusal("I'm sorry, but I can't do that."));
        assert!(extract_json("Sure! ```json\n{\"a\": 1}\n```").is_some());
        let mut e = Vec::new();
        schema_errors(
            &serde_json::json!({"a": "x"}),
            &serde_json::json!({"type": "object", "required": ["a", "b"], "properties": {"a": {"type": "integer"}}}),
            "",
            &mut e,
        );
        assert_eq!(e.len(), 2, "{e:?}");
    }
}
