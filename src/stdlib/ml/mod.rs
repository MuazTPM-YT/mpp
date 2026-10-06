pub mod compare;
pub mod drift;
pub mod metrics;

use super::stats::tests::{self as st, Rec};
use super::stats::{desc, num_or};
use super::table::{Col, Table, table_from_rows, table_value};
use crate::vm::*;
use indexmap::IndexMap;
use std::collections::HashSet;

macro_rules! natives {
    ($($name:literal => $f:expr),* $(,)?) => {
        &[$(Native { name: $name, f: $f }),*]
    };
}

pub static FNS: &[Native] = natives![
    "accuracy" => |vm, a| class_metric(vm, a, "accuracy"),
    "balanced_accuracy" => |vm, a| class_metric(vm, a, "balanced_accuracy"),
    "mcc" => |vm, a| class_metric(vm, a, "mcc"),
    "cohen_kappa" => |vm, a| class_metric(vm, a, "kappa"),
    "precision" => |_, a| prf(a, "precision"),
    "recall" => |_, a| prf(a, "recall"),
    "f1" => |_, a| prf(a, "f1"),
    "fbeta" => |_, a| prf(a, "fbeta"),
    "confusion_matrix" => confusion_matrix,
    "classification_report" => classification_report,
    "roc_auc" => roc_auc,
    "roc_curve" => |_, a| curve(a, true),
    "pr_curve" => |_, a| curve(a, false),
    "average_precision" => average_precision,
    "pr_auc" => average_precision,
    "log_loss" => log_loss,
    "brier" => brier,
    "top_k_accuracy" => top_k_accuracy,
    "regression" => regression,
    "mae" => |_, a| reg_one(a, |m| m.mae),
    "mse" => |_, a| reg_one(a, |m| m.mse),
    "rmse" => |_, a| reg_one(a, |m| m.rmse),
    "r2" => |_, a| reg_one(a, |m| m.r2),
    "mape" => |_, a| reg_one(a, |m| m.mape),
    "ndcg" => |_, a| rank_metric(a, "ndcg"),
    "precision_at_k" => |_, a| rank_metric(a, "precision"),
    "recall_at_k" => |_, a| rank_metric(a, "recall"),
    "hit_rate" => |_, a| rank_metric(a, "hit"),
    "mrr" => |_, a| rank_metric(a, "mrr"),
    "map_at_k" => |_, a| rank_metric(a, "map"),
    "calibration" => calibration,
    "psi" => psi,
    "wasserstein" => |_, a| {
        let [x, y] = a.bind(["a", "b"])?;
        Ok(Value::Float(drift::wasserstein(&floats(x, "a")?, &floats(y, "b")?)))
    },
    "kl" => |_, a| {
        let [p, q] = a.bind(["p", "q"])?;
        Ok(Value::Float(drift::kl(&floats(p, "p")?, &floats(q, "q")?)))
    },
    "js" => |_, a| {
        let [p, q] = a.bind(["p", "q"])?;
        Ok(Value::Float(drift::js(&floats(p, "p")?, &floats(q, "q")?)))
    },
    "drift" => drift_tables,
    "fairness" => fairness,
    "mcnemar" => mcnemar,
    "delong" => delong,
    "paired_bootstrap" => paired_bootstrap,
    "cv5x2" => |_, a| {
        let [d] = a.bind(["diffs"])?;
        let d = super::stats::matrix(d, "diffs")?;
        let (t, p) = compare::cv5x2(&d).map_err(value_err)?;
        Ok(Rec::default().text("method", "5x2cv paired t-test").num("statistic", t).num("p_value", p).num("df", 5.0).value())
    },
    "threshold_sweep" => threshold_sweep,
    "slices" => slices,
    "check" => check,
    "leakage" => leakage,
    "label_balance" => label_balance,
    "schema" => schema,
];

// ---- input helpers ----

fn values(v: Option<Value>, name: &str) -> Result<Vec<Value>, Flow> {
    super::to_vec(&need(v, name)?, name)
}

fn floats(v: Option<Value>, name: &str) -> Result<Vec<f64>, Flow> {
    let x = super::to_f64s(&need(v, name)?, name)?;
    if x.iter().any(|v| v.is_nan()) {
        return Err(value_err(format!("{name} has missing values; drop them first")));
    }
    Ok(x)
}

fn same_len(a: usize, b: usize, what: &str) -> Result<(), Flow> {
    if a != b {
        return Err(value_err(format!("{what}: lengths differ ({a} and {b})")));
    }
    if a == 0 {
        return Err(value_err(format!("{what}: no data")));
    }
    Ok(())
}

// labels: numbers, strings, bools; classes sorted
struct Enc {
    classes: Vec<Value>,
    y: Vec<usize>,
    p: Vec<usize>,
}

// hashable key for any cell, floats included
fn cell_key(v: &Value) -> Key {
    match v {
        Value::Float(x) if x.fract() != 0.0 => Key::Str(format!("{x:e}").into()),
        other => Key::from(other).unwrap_or(Key::Nil),
    }
}

fn norm_label(v: &Value) -> Value {
    match v {
        Value::Bool(b) => Value::Int(*b as i64),
        Value::Float(x) if x.fract() == 0.0 => Value::Int(*x as i64),
        other => other.clone(),
    }
}

fn classes_of(lists: &[&[Value]]) -> Result<Vec<Value>, Flow> {
    let mut keys = HashSet::new();
    let mut out = Vec::new();
    for l in lists {
        for v in l.iter() {
            let v = norm_label(v);
            if keys.insert(Key::from(&v)?) {
                out.push(v);
            }
        }
    }
    let mut bad = None;
    out.sort_by(|a, b| {
        compare(a, b).unwrap_or_else(|e| {
            bad = Some(e);
            std::cmp::Ordering::Equal
        })
    });
    if let Some(e) = bad {
        return Err(e);
    }
    Ok(out)
}

fn index_of(classes: &[Value], v: &Value) -> usize {
    let v = norm_label(v);
    classes.iter().position(|c| equal(c, &v)).unwrap_or(0)
}

fn encode(y: &[Value], p: &[Value]) -> Result<Enc, Flow> {
    same_len(y.len(), p.len(), "labels and predictions")?;
    let classes = classes_of(&[y, p])?;
    Ok(Enc { y: y.iter().map(|v| index_of(&classes, v)).collect(), p: p.iter().map(|v| index_of(&classes, v)).collect(), classes })
}

fn is_01(classes: &[Value]) -> bool {
    classes.iter().all(|c| matches!(c, Value::Int(0) | Value::Int(1)))
}

// which label counts as "positive"
fn pos_label(v: Option<Value>) -> Value {
    opt(v).map(|v| norm_label(&v)).unwrap_or(Value::Int(1))
}

fn truth(y: &[Value], pos: &Value) -> Vec<bool> {
    y.iter().map(|v| equal(&norm_label(v), pos)).collect()
}

fn require_both(t: &[bool]) -> Result<(), Flow> {
    if t.iter().all(|b| *b) || t.iter().all(|b| !*b) {
        return Err(value_err("need both positive and negative labels"));
    }
    Ok(())
}

fn list_f(v: &[f64]) -> Value {
    Value::list(v.iter().map(|x| Value::Float(*x)).collect())
}

// ---- classification ----

fn class_value(c: &[Vec<f64>], which: &str) -> f64 {
    match which {
        "accuracy" => metrics::accuracy(c),
        "balanced_accuracy" => metrics::balanced_accuracy(c),
        "mcc" => metrics::mcc(c),
        _ => metrics::kappa(c),
    }
}

fn class_metric(_: &mut Vm, a: Args, which: &str) -> R {
    let [y, p] = a.bind(["y_true", "y_pred"])?;
    let e = encode(&values(y, "y_true")?, &values(p, "y_pred")?)?;
    let c = metrics::confusion(&e.y, &e.p, e.classes.len());
    Ok(Value::Float(class_value(&c, which)))
}

// precision / recall / f by averaging mode
fn prf_value(e: &Enc, which: &str, beta: f64, average: &str, pos: &Value) -> Result<Value, Flow> {
    let c = metrics::confusion(&e.y, &e.p, e.classes.len());
    let s = metrics::per_class(&c, beta);
    let pick = |i: usize| match which {
        "precision" => s.precision[i],
        "recall" => s.recall[i],
        _ => s.f[i],
    };
    let k = e.classes.len();
    Ok(Value::Float(match average {
        "binary" => {
            let i = e
                .classes
                .iter()
                .position(|c| equal(c, pos))
                .ok_or_else(|| value_err(format!("positive label {pos:?} not found in the labels")))?;
            pick(i)
        }
        "micro" => metrics::accuracy(&c),
        "macro" => desc::mean(&(0..k).map(pick).collect::<Vec<_>>()),
        "weighted" => {
            let tot: f64 = s.support.iter().sum();
            (0..k).map(|i| pick(i) * s.support[i]).sum::<f64>() / tot
        }
        "none" => {
            return Ok(Value::map(
                e.classes
                    .iter()
                    .enumerate()
                    .map(|(i, c)| Ok((Key::from(c)?, Value::Float(pick(i)))))
                    .collect::<Result<IndexMap<_, _>, Flow>>()?,
            ));
        }
        other => return Err(value_err(format!("average must be binary, macro, micro, weighted or none, not {other:?}"))),
    }))
}

fn prf(a: Args, which: &str) -> R {
    let [y, p, average, pos, beta] = a.bind(["y_true", "y_pred", "average", "pos_label", "beta"])?;
    let e = encode(&values(y, "y_true")?, &values(p, "y_pred")?)?;
    let default = if is_01(&e.classes) { "binary" } else { "macro" };
    let average = opt(average).map_or(Ok(default.to_string()), |v| Ok::<_, Flow>(v.as_str("average")?.to_string()))?;
    let beta = if which == "fbeta" { need(beta, "beta")?.num("beta")? } else { 1.0 };
    prf_value(&e, which, beta, &average, &pos_label(pos))
}

fn confusion_matrix(_: &mut Vm, a: Args) -> R {
    let [y, p] = a.bind(["y_true", "y_pred"])?;
    let e = encode(&values(y, "y_true")?, &values(p, "y_pred")?)?;
    let c = metrics::confusion(&e.y, &e.p, e.classes.len());
    let rows = c.iter().map(|r| Value::list(r.iter().map(|x| Value::Int(*x as i64)).collect())).collect();
    Ok(Rec::default().val("labels", Value::list(e.classes)).val("matrix", Value::list(rows)).value())
}

fn classification_report(_: &mut Vm, a: Args) -> R {
    let [y, p] = a.bind(["y_true", "y_pred"])?;
    let e = encode(&values(y, "y_true")?, &values(p, "y_pred")?)?;
    let c = metrics::confusion(&e.y, &e.p, e.classes.len());
    let s = metrics::per_class(&c, 1.0);
    let mut rows = Vec::new();
    for (i, cl) in e.classes.iter().enumerate() {
        rows.push(
            Rec::default()
                .val("label", cl.clone())
                .num("precision", s.precision[i])
                .num("recall", s.recall[i])
                .num("f1", s.f[i])
                .int("support", s.support[i] as i64)
                .value(),
        );
    }
    let tot: f64 = s.support.iter().sum();
    for (name, w) in [("macro avg", None), ("weighted avg", Some(&s.support))] {
        let avg = |v: &[f64]| match w {
            None => desc::mean(v),
            Some(w) => v.iter().zip(w.iter()).map(|(a, b)| a * b).sum::<f64>() / tot,
        };
        rows.push(
            Rec::default()
                .val("label", Value::str(name))
                .num("precision", avg(&s.precision))
                .num("recall", avg(&s.recall))
                .num("f1", avg(&s.f))
                .int("support", tot as i64)
                .value(),
        );
    }
    let t = table_from_rows(&rows)?;
    Ok(Rec::default().num("accuracy", metrics::accuracy(&c)).val("table", table_value(t)).value())
}

// scores: list of floats (binary) or list of rows (one column per class)
fn score_rows(v: &Value) -> Option<Result<Vec<Vec<f64>>, Flow>> {
    match v {
        Value::List(l) if l.borrow().first().is_some_and(|x| matches!(x, Value::List(_) | Value::Object(_))) => {
            Some(l.borrow().iter().map(|r| super::to_f64s(r, "score row")).collect())
        }
        _ => None,
    }
}

fn roc_auc(_: &mut Vm, a: Args) -> R {
    let [y, s, pos] = a.bind(["y_true", "scores", "pos_label"])?;
    let yv = values(y, "y_true")?;
    let sv = need(s, "scores")?;
    if let Some(rows) = score_rows(&sv) {
        let rows = rows?;
        same_len(yv.len(), rows.len(), "labels and scores")?;
        let classes = classes_of(&[&yv])?;
        if rows.iter().any(|r| r.len() != classes.len()) {
            return Err(value_err(format!("each score row needs {} columns (one per class, sorted)", classes.len())));
        }
        let mut aucs = Vec::new();
        for (k, c) in classes.iter().enumerate() {
            let t = truth(&yv, c);
            aucs.push(metrics::roc_auc(&t, &rows.iter().map(|r| r[k]).collect::<Vec<_>>()));
        }
        return Ok(Value::Float(desc::mean(&aucs)));
    }
    let s = super::to_f64s(&sv, "scores")?;
    same_len(yv.len(), s.len(), "labels and scores")?;
    let t = truth(&yv, &pos_label(pos));
    require_both(&t)?;
    Ok(Value::Float(metrics::roc_auc(&t, &s)))
}

fn binary_inputs(y: Option<Value>, s: Option<Value>, pos: Option<Value>) -> Result<(Vec<bool>, Vec<f64>), Flow> {
    let yv = values(y, "y_true")?;
    let s = floats(s, "scores")?;
    same_len(yv.len(), s.len(), "labels and scores")?;
    let t = truth(&yv, &pos_label(pos));
    require_both(&t)?;
    Ok((t, s))
}

fn curve(a: Args, roc: bool) -> R {
    let [y, s, pos] = a.bind(["y_true", "scores", "pos_label"])?;
    let (t, s) = binary_inputs(y, s, pos)?;
    if roc {
        let (f, tp, th) = metrics::roc_curve(&t, &s);
        Ok(Rec::default()
            .val("fpr", list_f(&f))
            .val("tpr", list_f(&tp))
            .val("thresholds", list_f(&th))
            .num("auc", metrics::roc_auc(&t, &s))
            .value())
    } else {
        let (p, r, th) = metrics::pr_curve(&t, &s);
        Ok(Rec::default()
            .val("precision", list_f(&p))
            .val("recall", list_f(&r))
            .val("thresholds", list_f(&th))
            .num("average_precision", metrics::average_precision(&t, &s))
            .value())
    }
}

fn average_precision(_: &mut Vm, a: Args) -> R {
    let [y, s, pos] = a.bind(["y_true", "scores", "pos_label"])?;
    let (t, s) = binary_inputs(y, s, pos)?;
    Ok(Value::Float(metrics::average_precision(&t, &s)))
}

fn log_loss(_: &mut Vm, a: Args) -> R {
    let [y, p, pos] = a.bind(["y_true", "probs", "pos_label"])?;
    let yv = values(y, "y_true")?;
    let pv = need(p, "probs")?;
    if let Some(rows) = score_rows(&pv) {
        let rows = rows?;
        same_len(yv.len(), rows.len(), "labels and probs")?;
        let classes = classes_of(&[&yv])?;
        let idx: Vec<usize> = yv.iter().map(|v| index_of(&classes, v)).collect();
        if rows.iter().any(|r| r.len() != classes.len()) {
            return Err(value_err("each prob row needs one column per class (sorted)"));
        }
        return Ok(Value::Float(metrics::log_loss_multi(&idx, &rows)));
    }
    let p = super::to_f64s(&pv, "probs")?;
    same_len(yv.len(), p.len(), "labels and probs")?;
    Ok(Value::Float(metrics::log_loss_binary(&truth(&yv, &pos_label(pos)), &p)))
}

fn brier(_: &mut Vm, a: Args) -> R {
    let [y, p, pos] = a.bind(["y_true", "probs", "pos_label"])?;
    let yv = values(y, "y_true")?;
    let p = floats(p, "probs")?;
    same_len(yv.len(), p.len(), "labels and probs")?;
    Ok(Value::Float(metrics::brier(&truth(&yv, &pos_label(pos)), &p)))
}

fn top_k_accuracy(_: &mut Vm, a: Args) -> R {
    let [y, s, k] = a.bind(["y_true", "scores", "k"])?;
    let yv = values(y, "y_true")?;
    let rows = score_rows(&need(s, "scores")?).ok_or_else(|| type_err("scores must be rows, one column per class"))??;
    same_len(yv.len(), rows.len(), "labels and scores")?;
    let classes = classes_of(&[&yv])?;
    let idx: Vec<usize> = yv.iter().map(|v| index_of(&classes, v)).collect();
    let k = opt(k).map_or(Ok(2), |v| v.int("k"))?.max(1) as usize;
    Ok(Value::Float(metrics::top_k(&idx, &rows, k)))
}

// ---- regression ----

fn reg_inputs(a: Args) -> Result<metrics::RegMetrics, Flow> {
    let [y, p] = a.bind(["y_true", "y_pred"])?;
    let (y, p) = (floats(y, "y_true")?, floats(p, "y_pred")?);
    same_len(y.len(), p.len(), "y_true and y_pred")?;
    Ok(metrics::regression(&y, &p))
}

fn reg_one(a: Args, f: fn(&metrics::RegMetrics) -> f64) -> R {
    Ok(Value::Float(f(&reg_inputs(a)?)))
}

fn regression(_: &mut Vm, a: Args) -> R {
    let m = reg_inputs(a)?;
    Ok(Rec::default()
        .num("mae", m.mae)
        .num("mse", m.mse)
        .num("rmse", m.rmse)
        .num("r2", m.r2)
        .num("mape", m.mape)
        .num("smape", m.smape)
        .num("median_ae", m.median_ae)
        .num("max_error", m.max_error)
        .num("explained_variance", m.explained_variance)
        .value())
}

// ---- ranking ----

fn rank_metric(a: Args, which: &str) -> R {
    let [rels, k, gain] = a.bind(["relevance", "k", "gain"])?;
    let rels = need(rels, "relevance")?;
    let queries: Vec<Vec<f64>> = match score_rows(&rels) {
        Some(r) => r?,
        None => vec![super::to_f64s(&rels, "relevance")?],
    };
    let exp = match opt(gain) {
        Some(g) => match &**g.as_str("gain")? {
            "exp" | "exponential" => true,
            "linear" => false,
            _ => return Err(value_err("gain must be \"linear\" or \"exp\"")),
        },
        None => false,
    };
    let k = opt(k).map(|v| v.int("k")).transpose()?;
    let scores: Vec<f64> = queries
        .iter()
        .map(|q| {
            let k = k.map_or(q.len(), |k| k.max(1) as usize);
            match which {
                "ndcg" => metrics::ndcg(q, k, exp),
                "precision" => metrics::precision_at(q, k),
                "recall" => metrics::recall_at(q, k),
                "hit" => metrics::hit_at(q, k),
                "mrr" => metrics::reciprocal_rank(q, k),
                _ => metrics::average_precision_at(q, k),
            }
        })
        .collect();
    Ok(Value::Float(desc::mean(&scores)))
}

fn calibration(_: &mut Vm, a: Args) -> R {
    let [y, p, bins, strategy, pos] = a.bind(["y_true", "probs", "bins", "strategy", "pos_label"])?;
    let yv = values(y, "y_true")?;
    let p = floats(p, "probs")?;
    same_len(yv.len(), p.len(), "labels and probs")?;
    let bins = opt(bins).map_or(Ok(10), |v| v.int("bins"))?.clamp(1, 1000) as usize;
    let quant = match opt(strategy) {
        None => false,
        Some(s) => match &**s.as_str("strategy")? {
            "uniform" => false,
            "quantile" => true,
            _ => return Err(value_err("strategy must be \"uniform\" or \"quantile\"")),
        },
    };
    let (b, ece, mce) = metrics::calibration(&truth(&yv, &pos_label(pos)), &p, bins, quant);
    let rows: Vec<Value> = b
        .iter()
        .map(|b| {
            Rec::default()
                .num("lo", b.lo)
                .num("hi", b.hi)
                .int("count", b.count as i64)
                .num("mean_pred", b.mean_pred)
                .num("frac_pos", b.frac_pos)
                .value()
        })
        .collect();
    Ok(Rec::default().num("ece", ece).num("mce", mce).val("bins", table_value(table_from_rows(&rows)?)).value())
}

// ---- drift ----

fn psi(_: &mut Vm, a: Args) -> R {
    let [r, c, bins] = a.bind(["reference", "current", "bins"])?;
    let bins = opt(bins).map_or(Ok(10), |v| v.int("bins"))?.clamp(2, 1000) as usize;
    let (v, _) = drift::psi(&floats(r, "reference")?, &floats(c, "current")?, bins);
    Ok(Rec::default().num("psi", v).text("level", drift::psi_level(v)).value())
}

// shares of each category in two samples, same category order
fn shares(a: &Col, b: &Col) -> (Vec<f64>, Vec<f64>, Vec<Vec<f64>>) {
    let mut cats: IndexMap<Key, (f64, f64)> = IndexMap::new();
    for i in 0..a.len() {
        cats.entry(cell_key(&a.get(i))).or_default().0 += 1.0;
    }
    for i in 0..b.len() {
        cats.entry(cell_key(&b.get(i))).or_default().1 += 1.0;
    }
    let (na, nb) = (a.len() as f64, b.len() as f64);
    let sa = cats.values().map(|c| c.0 / na).collect();
    let sb = cats.values().map(|c| c.1 / nb).collect();
    let counts = vec![cats.values().map(|c| c.0).collect(), cats.values().map(|c| c.1).collect()];
    (sa, sb, counts)
}

fn drift_tables(_: &mut Vm, a: Args) -> R {
    let [r, c, cols, psi_t, p_t] = a.bind(["reference", "current", "columns", "psi_threshold", "p_threshold"])?;
    let (r, c) = (need(r, "reference")?, need(c, "current")?);
    let rt = r.object::<Table>().ok_or_else(|| type_err("reference must be a table"))?;
    let ct = c.object::<Table>().ok_or_else(|| type_err("current must be a table"))?;
    let psi_t = num_or(psi_t, "psi_threshold", 0.2)?;
    let p_t = num_or(p_t, "p_threshold", 0.01)?;
    let names: Vec<String> = match opt(cols) {
        Some(v) => super::to_vec(&v, "columns")?.iter().map(|x| Ok(x.as_str("column")?.to_string())).collect::<Result<_, Flow>>()?,
        None => rt.names.iter().filter(|n| ct.col_index(n).is_ok()).map(|n| n.to_string()).collect(),
    };
    let mut out = IndexMap::new();
    let mut drifted = Vec::new();
    for n in &names {
        let (ca, cb) = (rt.col(n)?, ct.col(n)?);
        let rec = match (ca, cb) {
            (Col::Num(..), Col::Num(..)) => {
                let (x, y) = (rt.nums(n)?, ct.nums(n)?);
                if x.len() < 2 || y.len() < 2 {
                    continue;
                }
                let (pv, _) = drift::psi(&x, &y, 10);
                let ks = st::ks_2samp(&x, &y, None).map_err(value_err)?.get("p_value");
                let d = pv >= psi_t || ks < p_t;
                Rec::default()
                    .text("kind", "number")
                    .num("psi", pv)
                    .num("ks_p_value", ks)
                    .num("wasserstein", drift::wasserstein(&x, &y))
                    .num("mean_ref", desc::mean(&x))
                    .num("mean_cur", desc::mean(&y))
                    .flag("drifted", d)
            }
            _ => {
                let (sa, sb, counts) = shares(ca, cb);
                let pv = drift::psi_shares(&sa, &sb);
                let chi =
                    if counts[0].len() >= 2 { st::chi2_contingency(&counts, false).map(|r| r.get("p_value")).unwrap_or(1.0) } else { 1.0 };
                let d = pv >= psi_t || chi < p_t;
                Rec::default()
                    .text("kind", "category")
                    .num("psi", pv)
                    .num("chi2_p_value", chi)
                    .num("js", drift::js(&sa, &sb))
                    .flag("drifted", d)
            }
        };
        if rec.0.last().is_some_and(|(_, v)| v.truthy()) {
            drifted.push(Value::str(n));
        }
        out.insert(Key::Str(n.as_str().into()), rec.value());
    }
    Ok(Rec::default().val("columns", Value::map(out)).val("drifted", Value::list(drifted)).value())
}

// ---- fairness ----

fn fairness(_: &mut Vm, a: Args) -> R {
    let [y, p, g, pos] = a.bind(["y_true", "y_pred", "group", "pos_label"])?;
    let (yv, pv, gv) = (values(y, "y_true")?, values(p, "y_pred")?, values(g, "group")?);
    same_len(yv.len(), pv.len(), "labels and predictions")?;
    same_len(yv.len(), gv.len(), "labels and groups")?;
    let pos = pos_label(pos);
    let (ty, tp) = (truth(&yv, &pos), truth(&pv, &pos));
    let mut groups: IndexMap<Key, Vec<usize>> = IndexMap::new();
    for (i, g) in gv.iter().enumerate() {
        groups.entry(Key::from(g)?).or_default().push(i);
    }
    let mut out = IndexMap::new();
    let (mut sel, mut tprs, mut fprs) = (Vec::new(), Vec::new(), Vec::new());
    for (k, idx) in &groups {
        let n = idx.len() as f64;
        let pos_pred = idx.iter().filter(|&&i| tp[i]).count() as f64;
        let actual_pos = idx.iter().filter(|&&i| ty[i]).count() as f64;
        let actual_neg = n - actual_pos;
        let tpc = idx.iter().filter(|&&i| tp[i] && ty[i]).count() as f64;
        let fpc = idx.iter().filter(|&&i| tp[i] && !ty[i]).count() as f64;
        let correct = idx.iter().filter(|&&i| tp[i] == ty[i]).count() as f64;
        let div = |a: f64, b: f64| if b == 0.0 { f64::NAN } else { a / b };
        let (s, tpr, fpr) = (pos_pred / n, div(tpc, actual_pos), div(fpc, actual_neg));
        sel.push(s);
        if !tpr.is_nan() {
            tprs.push(tpr);
        }
        if !fpr.is_nan() {
            fprs.push(fpr);
        }
        out.insert(
            k.clone(),
            Rec::default()
                .int("n", n as i64)
                .num("selection_rate", s)
                .num("tpr", tpr)
                .num("fpr", fpr)
                .num("precision", div(tpc, pos_pred))
                .num("accuracy", correct / n)
                .value(),
        );
    }
    let spread = |v: &[f64]| if v.is_empty() { f64::NAN } else { desc::max(v) - desc::min(v) };
    let di = if desc::max(&sel) == 0.0 { f64::NAN } else { desc::min(&sel) / desc::max(&sel) };
    let eo = spread(&tprs);
    Ok(Rec::default()
        .val("groups", Value::map(out))
        .num("demographic_parity_diff", spread(&sel))
        .num("disparate_impact", di)
        .num("equal_opportunity_diff", eo)
        .num("equalized_odds_diff", eo.max(spread(&fprs)))
        .flag("passes_80_rule", di >= 0.8)
        .value())
}

// ---- model comparison ----

fn correct(y: &[Value], p: &[Value]) -> Vec<bool> {
    y.iter().zip(p).map(|(a, b)| equal(&norm_label(a), &norm_label(b))).collect()
}

fn mcnemar(_: &mut Vm, a: Args) -> R {
    let [y, p1, p2, exact] = a.bind(["y_true", "pred_a", "pred_b", "exact"])?;
    let (yv, a1, a2) = (values(y, "y_true")?, values(p1, "pred_a")?, values(p2, "pred_b")?);
    same_len(yv.len(), a1.len(), "labels and pred_a")?;
    same_len(yv.len(), a2.len(), "labels and pred_b")?;
    let exact = opt(exact).map(|e| e.truthy());
    let (s, p, b, c, ex) = compare::mcnemar(&correct(&yv, &a1), &correct(&yv, &a2), exact);
    Ok(Rec::default()
        .text("method", if ex { "McNemar (exact binomial)" } else { "McNemar (chi-square, corrected)" })
        .num("statistic", s)
        .num("p_value", p)
        .int("only_a_right", b as i64)
        .int("only_b_right", c as i64)
        .value())
}

fn delong(_: &mut Vm, a: Args) -> R {
    let [y, s1, s2, pos] = a.bind(["y_true", "scores_a", "scores_b", "pos_label"])?;
    let yv = values(y, "y_true")?;
    let (s1, s2) = (floats(s1, "scores_a")?, floats(s2, "scores_b")?);
    same_len(yv.len(), s1.len(), "labels and scores_a")?;
    same_len(yv.len(), s2.len(), "labels and scores_b")?;
    let t = truth(&yv, &pos_label(pos));
    require_both(&t)?;
    let (a1, a2, z, p) = compare::delong(&t, &s1, &s2);
    Ok(Rec::default()
        .text("method", "DeLong test for two AUCs")
        .num("auc_a", a1)
        .num("auc_b", a2)
        .num("diff", a2 - a1)
        .num("z", z)
        .num("p_value", p)
        .value())
}

// metric by name, or a fn(y_true, y_pred)
fn metric_of(vm: &mut Vm, m: &Value, y: &[Value], p: &[Value]) -> Result<f64, Flow> {
    if m.is_callable() {
        return vm.call(m, &[Value::list(y.to_vec()), Value::list(p.to_vec())])?.num("metric result");
    }
    let name = m.as_str("metric")?;
    match &**name {
        "accuracy" | "balanced_accuracy" | "mcc" | "kappa" => {
            let e = encode(y, p)?;
            Ok(class_value(&metrics::confusion(&e.y, &e.p, e.classes.len()), name))
        }
        "f1" | "precision" | "recall" => {
            let e = encode(y, p)?;
            let avg = if is_01(&e.classes) { "binary" } else { "macro" };
            prf_value(&e, name, 1.0, avg, &Value::Int(1))?.num("")
        }
        "mae" | "mse" | "rmse" | "r2" => {
            let yf: Vec<f64> = y.iter().map(|v| v.num("y")).collect::<Result<_, _>>()?;
            let pf: Vec<f64> = p.iter().map(|v| v.num("pred")).collect::<Result<_, _>>()?;
            let r = metrics::regression(&yf, &pf);
            Ok(match &**name {
                "mae" => r.mae,
                "mse" => r.mse,
                "rmse" => r.rmse,
                _ => r.r2,
            })
        }
        other => Err(value_err(format!(
            "unknown metric {other:?} (accuracy, balanced_accuracy, f1, precision, recall, mcc, kappa, mae, mse, rmse, r2, or a function)"
        ))),
    }
}

fn paired_bootstrap(vm: &mut Vm, a: Args) -> R {
    let [y, p1, p2, metric, n, level] = a.bind(["y_true", "pred_a", "pred_b", "metric", "n", "level"])?;
    let (yv, a1, a2) = (values(y, "y_true")?, values(p1, "pred_a")?, values(p2, "pred_b")?);
    same_len(yv.len(), a1.len(), "labels and pred_a")?;
    same_len(yv.len(), a2.len(), "labels and pred_b")?;
    let metric = opt(metric).unwrap_or(Value::str("accuracy"));
    let reps = opt(n).map_or(Ok(2000), |v| v.int("n"))?.clamp(10, 1_000_000) as usize;
    let level = super::stats::level_arg(level)?;
    let est = metric_of(vm, &metric, &yv, &a2)? - metric_of(vm, &metric, &yv, &a1)?;
    let nrows = yv.len();
    let mut diffs = Vec::with_capacity(reps);
    let (mut by, mut b1, mut b2) = (Vec::with_capacity(nrows), Vec::with_capacity(nrows), Vec::with_capacity(nrows));
    for _ in 0..reps {
        by.clear();
        b1.clear();
        b2.clear();
        for _ in 0..nrows {
            let i = vm.rng.below(nrows as u64) as usize;
            by.push(yv[i].clone());
            b1.push(a1[i].clone());
            b2.push(a2[i].clone());
        }
        diffs.push(metric_of(vm, &metric, &by, &b2)? - metric_of(vm, &metric, &by, &b1)?);
    }
    let s = desc::sorted(&diffs);
    let le = diffs.iter().filter(|d| **d <= 0.0).count() as f64 / reps as f64;
    let ge = diffs.iter().filter(|d| **d >= 0.0).count() as f64 / reps as f64;
    Ok(Rec::default()
        .text("method", "paired bootstrap")
        .num("diff", est)
        .pair("ci", desc::quantile_sorted(&s, (1.0 - level) / 2.0), desc::quantile_sorted(&s, 1.0 - (1.0 - level) / 2.0))
        .num("p_value", (2.0 * le.min(ge)).min(1.0))
        .int("reps", reps as i64)
        .value())
}

// precision/recall/f1 at many cut-offs; picks the best
fn threshold_sweep(_: &mut Vm, a: Args) -> R {
    let [y, s, metric, steps, pos] = a.bind(["y_true", "scores", "metric", "steps", "pos_label"])?;
    let (t, s) = binary_inputs(y, s, pos)?;
    let metric = opt(metric).map_or(Ok("f1".to_string()), |m| Ok::<_, Flow>(m.as_str("metric")?.to_string()))?;
    let steps = opt(steps).map_or(Ok(101), |v| v.int("steps"))?.clamp(2, 10_000) as usize;
    let (lo, hi) = (desc::min(&s), desc::max(&s));
    let mut rows = Vec::new();
    let mut best: Option<(f64, Value)> = None;
    for i in 0..steps {
        let th = lo + (hi - lo) * i as f64 / (steps - 1) as f64;
        let pred: Vec<bool> = s.iter().map(|v| *v >= th).collect();
        let tp = t.iter().zip(&pred).filter(|(a, b)| **a && **b).count() as f64;
        let fp = t.iter().zip(&pred).filter(|(a, b)| !**a && **b).count() as f64;
        let fneg = t.iter().zip(&pred).filter(|(a, b)| **a && !**b).count() as f64;
        let tn = t.len() as f64 - tp - fp - fneg;
        let d = |a: f64, b: f64| if b == 0.0 { 0.0 } else { a / b };
        let (pr, rc) = (d(tp, tp + fp), d(tp, tp + fneg));
        let f1 = d(2.0 * pr * rc, pr + rc);
        let acc = (tp + tn) / t.len() as f64;
        let fpr = d(fp, fp + tn);
        let score = match metric.as_str() {
            "f1" => f1,
            "precision" => pr,
            "recall" => rc,
            "accuracy" => acc,
            "youden" => rc - fpr,
            other => return Err(value_err(format!("metric must be f1, precision, recall, accuracy or youden, not {other:?}"))),
        };
        let row = Rec::default()
            .num("threshold", th)
            .num("precision", pr)
            .num("recall", rc)
            .num("f1", f1)
            .num("accuracy", acc)
            .num("fpr", fpr)
            .value();
        if best.as_ref().is_none_or(|(b, _)| score > *b) {
            best = Some((score, row.clone()));
        }
        rows.push(row);
    }
    let (score, row) = best.unwrap();
    let th = match &row {
        Value::Map(m) => m.borrow().get(&Key::Str("threshold".into())).cloned().unwrap_or(Value::Nil),
        _ => Value::Nil,
    };
    Ok(Rec::default()
        .val("best_threshold", th)
        .num("best_score", score)
        .val("best", row)
        .val("table", table_value(table_from_rows(&rows)?))
        .value())
}

// metric per slice of a table, worst first
fn slices(vm: &mut Vm, a: Args) -> R {
    let [data, y, p, by, metric, min_n] = a.bind(["data", "y_true", "y_pred", "by", "metric", "min_n"])?;
    let data = need(data, "data")?;
    let t = data.object::<Table>().ok_or_else(|| type_err("data must be a table"))?;
    let (yc, pc, bc) = (
        need(y, "y_true")?.as_str("y_true")?.to_string(),
        need(p, "y_pred")?.as_str("y_pred")?.to_string(),
        need(by, "by")?.as_str("by")?.to_string(),
    );
    let metric = opt(metric).unwrap_or(Value::str("accuracy"));
    let min_n = opt(min_n).map_or(Ok(1), |v| v.int("min_n"))?.max(1) as usize;
    let (ycol, pcol, bcol) = (t.col(&yc)?, t.col(&pc)?, t.col(&bc)?);
    let all_y: Vec<Value> = (0..t.nrows).map(|i| ycol.get(i)).collect();
    let all_p: Vec<Value> = (0..t.nrows).map(|i| pcol.get(i)).collect();
    let overall = metric_of(vm, &metric, &all_y, &all_p)?;
    let lower_better = matches!(&metric, Value::Str(s) if matches!(&**s, "mae" | "mse" | "rmse"));
    let mut groups: IndexMap<Key, Vec<usize>> = IndexMap::new();
    for i in 0..t.nrows {
        groups.entry(Key::from(&bcol.get(i))?).or_default().push(i);
    }
    let mut rows = Vec::new();
    for (k, idx) in groups {
        if idx.len() < min_n {
            continue;
        }
        let ys: Vec<Value> = idx.iter().map(|&i| all_y[i].clone()).collect();
        let ps: Vec<Value> = idx.iter().map(|&i| all_p[i].clone()).collect();
        let m = metric_of(vm, &metric, &ys, &ps)?;
        rows.push((m, Rec::default().val("slice", k.value()).int("n", idx.len() as i64).num("metric", m).num("gap", m - overall).value()));
    }
    rows.sort_by(|a, b| if lower_better { b.0.total_cmp(&a.0) } else { a.0.total_cmp(&b.0) });
    let tbl = table_from_rows(&rows.into_iter().map(|r| r.1).collect::<Vec<_>>())?;
    Ok(Rec::default().num("overall", overall).val("table", table_value(tbl)).value())
}

// ---- data checks ----

fn check(_: &mut Vm, a: Args) -> R {
    let [data] = a.bind(["data"])?;
    let data = need(data, "data")?;
    let t = data.object::<Table>().ok_or_else(|| type_err("data must be a table"))?;
    let mut missing = IndexMap::new();
    let mut constant = Vec::new();
    let mut ranges = IndexMap::new();
    for (n, c) in t.names.iter().zip(&t.cols) {
        let miss = (0..c.len()).filter(|&i| matches!(c.get(i), Value::Nil)).count();
        missing.insert(Key::Str(n.clone()), Value::Int(miss as i64));
        let distinct: HashSet<Key> = (0..c.len()).map(|i| cell_key(&c.get(i))).collect();
        if distinct.len() <= 1 {
            constant.push(Value::Str(n.clone()));
        }
        if let Col::Num(..) = c {
            let v = t.nums(n)?;
            if !v.is_empty() {
                ranges.insert(Key::Str(n.clone()), Value::list(vec![Value::Float(desc::min(&v)), Value::Float(desc::max(&v))]));
            }
        }
    }
    let mut seen = HashSet::new();
    let mut dups = 0;
    for i in 0..t.nrows {
        let key: Vec<Key> = t.cols.iter().map(|c| cell_key(&c.get(i))).collect();
        if !seen.insert(key) {
            dups += 1;
        }
    }
    let total_missing: i64 = missing.values().map(|v| if let Value::Int(n) = v { *n } else { 0 }).sum();
    Ok(Rec::default()
        .int("rows", t.nrows as i64)
        .int("columns", t.names.len() as i64)
        .val("missing", Value::map(missing))
        .num("missing_share", total_missing as f64 / (t.nrows * t.names.len()).max(1) as f64)
        .int("duplicates", dups)
        .val("constant", Value::list(constant))
        .val("ranges", Value::map(ranges))
        .value())
}

// rows of test that also appear in train (on chosen columns)
fn leakage(_: &mut Vm, a: Args) -> R {
    let [train, test, cols] = a.bind(["train", "test", "columns"])?;
    let (tr, te) = (need(train, "train")?, need(test, "test")?);
    let (tr, te) = (
        tr.object::<Table>().ok_or_else(|| type_err("train must be a table"))?,
        te.object::<Table>().ok_or_else(|| type_err("test must be a table"))?,
    );
    let names: Vec<String> = match opt(cols) {
        Some(v) => super::to_vec(&v, "columns")?.iter().map(|x| Ok(x.as_str("column")?.to_string())).collect::<Result<_, Flow>>()?,
        None => tr.names.iter().filter(|n| te.col_index(n).is_ok()).map(|n| n.to_string()).collect(),
    };
    let rowkey = |t: &Table, i: usize| -> Result<Vec<Key>, Flow> { names.iter().map(|n| Ok(cell_key(&t.col(n)?.get(i)))).collect() };
    let mut seen = HashSet::new();
    for i in 0..tr.nrows {
        seen.insert(rowkey(tr, i)?);
    }
    let mut hits = 0;
    for i in 0..te.nrows {
        if seen.contains(&rowkey(te, i)?) {
            hits += 1;
        }
    }
    Ok(Rec::default().int("overlap_rows", hits).num("overlap_share", hits as f64 / te.nrows.max(1) as f64).flag("leak", hits > 0).value())
}

fn label_balance(_: &mut Vm, a: Args) -> R {
    let [y] = a.bind(["y"])?;
    let yv = values(y, "y")?;
    let mut counts: IndexMap<Key, i64> = IndexMap::new();
    for v in &yv {
        *counts.entry(Key::from(&norm_label(v))?).or_default() += 1;
    }
    counts.sort_by(|_, a, _, b| b.cmp(a));
    let n = yv.len() as f64;
    let (mx, mn) = (*counts.values().max().unwrap_or(&0) as f64, *counts.values().min().unwrap_or(&0) as f64);
    Ok(Rec::default()
        .val("counts", Value::map(counts.iter().map(|(k, c)| (k.clone(), Value::Int(*c))).collect()))
        .val("shares", Value::map(counts.iter().map(|(k, c)| (k.clone(), Value::Float(*c as f64 / n))).collect()))
        .num("imbalance_ratio", if mn == 0.0 { f64::INFINITY } else { mx / mn })
        .value())
}

// schema(table, {"col": "number" | "text", ...}, required = true)
fn schema(_: &mut Vm, a: Args) -> R {
    let [data, spec] = a.bind(["data", "spec"])?;
    let data = need(data, "data")?;
    let t = data.object::<Table>().ok_or_else(|| type_err("data must be a table"))?;
    let spec = need(spec, "spec")?;
    let Value::Map(m) = &spec else { return Err(type_err("spec must be a map like {\"age\": \"number\"}")) };
    let mut problems = Vec::new();
    for (k, v) in m.borrow().iter() {
        let name = k.value();
        let name = name.as_str("column")?;
        let want = v.as_str("type")?;
        match t.col(name) {
            Err(_) => problems.push(Value::str(format!("missing column `{name}`"))),
            Ok(c) => {
                let got = if matches!(c, Col::Num(..)) { "number" } else { "text" };
                if &**want != got {
                    problems.push(Value::str(format!("column `{name}` is {got}, expected {want}")));
                }
            }
        }
    }
    Ok(Rec::default().flag("ok", problems.is_empty()).val("problems", Value::list(problems)).value())
}
