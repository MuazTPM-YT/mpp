// training-log doctor: NaN, spikes, divergence, plateaus, overfitting, grad norms, LR shape
use crate::stdlib::chart::{chart_value, line_svg};
use crate::stdlib::stats::desc;
use crate::stdlib::stats::tests::Rec;
use crate::stdlib::table::{Col, Table, table_from_rows, table_value};
use crate::vm::*;
use std::rc::Rc;

// columns of numbers; None = not logged on that row, Some(NaN) = logged as NaN
pub const METHODS: &[&str] = &["check", "plot", "compare", "col"];

pub struct TrainLog {
    pub path: String,
    pub names: Vec<String>,
    pub cols: Vec<Vec<Option<f64>>>,
}

const STEP: &[&str] = &["step", "global_step", "iteration", "iter", "steps"];
const LOSS: &[&str] = &["loss", "train_loss", "training_loss", "train/loss"];
const VAL: &[&str] = &["val_loss", "eval_loss", "valid_loss", "validation_loss", "eval/loss", "val/loss"];
const GRAD: &[&str] = &["grad_norm", "gradient_norm", "train/grad_norm"];
const LR: &[&str] = &["lr", "learning_rate", "train/learning_rate"];
const SPEED: &[&str] =
    &["tokens_per_sec", "tokens_per_second", "throughput", "train_samples_per_second", "samples_per_second", "tok_per_sec"];

fn special(s: &str) -> Option<f64> {
    match s.trim() {
        "NaN" | "nan" | "__mpp_NaN" => Some(f64::NAN),
        "Infinity" | "inf" | "__mpp_Infinity" => Some(f64::INFINITY),
        "-Infinity" | "-inf" | "__mpp_-Infinity" => Some(f64::NEG_INFINITY),
        _ => None,
    }
}

impl TrainLog {
    pub fn load(path: &str) -> Result<TrainLog, Flow> {
        let text = std::fs::read_to_string(path).map_err(|e| err("IOError", format!("{path}: {e}")))?;
        let lower = path.to_lowercase();
        if lower.ends_with(".csv") || lower.ends_with(".tsv") {
            return Self::from_csv(path, &text, if lower.ends_with(".tsv") { b'\t' } else { b',' });
        }
        Self::from_jsonl(path, &text)
    }

    fn from_csv(path: &str, text: &str, sep: u8) -> Result<TrainLog, Flow> {
        let mut rd = csv::ReaderBuilder::new().delimiter(sep).flexible(true).from_reader(text.as_bytes());
        let names: Vec<String> =
            rd.headers().map_err(|e| err("CSVError", format!("{path}: {e}")))?.iter().map(|s| s.trim().to_string()).collect();
        let mut cols = vec![Vec::new(); names.len()];
        for rec in rd.records() {
            let rec = rec.map_err(|e| err("CSVError", format!("{path}: {e}")))?;
            for (i, col) in cols.iter_mut().enumerate() {
                let cell = rec.get(i).unwrap_or("").trim();
                col.push(if cell.is_empty() { None } else { special(cell).or_else(|| cell.parse::<f64>().ok()) });
            }
        }
        Ok(TrainLog { path: path.into(), names, cols }.numeric_only())
    }

    fn from_jsonl(path: &str, text: &str) -> Result<TrainLog, Flow> {
        // python's json writes bare NaN/Infinity; make them parseable
        let re = regex::Regex::new(r"([:\[,]\s*)(-?Infinity|NaN)(\s*[,\]\}])").unwrap();
        let mut rows: Vec<serde_json::Map<String, serde_json::Value>> = Vec::new();
        let body = text.trim_start();
        let lines: Vec<String> = if body.starts_with('[') {
            // whole-file JSON array
            let fixed = fix_all(&re, body);
            let v: serde_json::Value = serde_json::from_str(&fixed).map_err(|e| err("JSONError", format!("{path}: {e}")))?;
            v.as_array().map(|a| a.iter().map(|x| x.to_string()).collect()).unwrap_or_default()
        } else {
            text.lines().map(String::from).collect()
        };
        for (i, line) in lines.iter().enumerate() {
            let l = line.trim();
            if l.is_empty() {
                continue;
            }
            let fixed = fix_all(&re, l);
            let v: serde_json::Value =
                serde_json::from_str(&fixed).map_err(|_| err("JSONError", format!("{path}:{}: bad JSON line", i + 1)))?;
            if let serde_json::Value::Object(m) = v {
                rows.push(m);
            }
        }
        let mut names: Vec<String> = Vec::new();
        for r in &rows {
            for (k, v) in r {
                let numeric = v.is_number() || v.as_str().and_then(special).is_some() || v.is_boolean();
                if numeric && !names.contains(k) {
                    names.push(k.clone());
                }
            }
        }
        let cols = names
            .iter()
            .map(|n| {
                rows.iter()
                    .map(|r| {
                        r.get(n).and_then(|v| {
                            v.as_f64().or_else(|| v.as_str().and_then(special)).or_else(|| v.as_bool().map(|b| b as i64 as f64))
                        })
                    })
                    .collect()
            })
            .collect();
        Ok(TrainLog { path: path.into(), names, cols })
    }

    fn numeric_only(self) -> TrainLog {
        let keep: Vec<usize> = (0..self.names.len()).filter(|&i| self.cols[i].iter().any(|v| v.is_some())).collect();
        TrainLog {
            path: self.path,
            names: keep.iter().map(|&i| self.names[i].clone()).collect(),
            cols: keep.iter().map(|&i| self.cols[i].clone()).collect(),
        }
    }

    pub fn from_table(t: &Table) -> TrainLog {
        let mut names = Vec::new();
        let mut cols = Vec::new();
        for (n, c) in t.names.iter().zip(&t.cols) {
            if let Col::Num(v, _) = c {
                names.push(n.to_string());
                cols.push(v.iter().map(|x| if x.is_nan() { None } else { Some(*x) }).collect());
            }
        }
        TrainLog { path: "<table>".into(), names, cols }
    }

    fn find(&self, want: Option<&str>, aliases: &[&str]) -> Option<usize> {
        match want {
            Some(w) => self.names.iter().position(|n| n == w),
            None => aliases.iter().find_map(|a| self.names.iter().position(|n| n.eq_ignore_ascii_case(a))),
        }
    }

    // (step, value) pairs where the column was logged
    fn series(&self, col: usize, step: Option<usize>) -> Vec<(f64, f64)> {
        self.cols[col]
            .iter()
            .enumerate()
            .filter_map(|(i, v)| v.map(|v| (step.and_then(|s| self.cols[s][i]).unwrap_or(i as f64), v)))
            .collect()
    }

    pub fn table(&self) -> Result<Table, Flow> {
        let names = self.names.iter().map(|n| Rc::from(n.as_str())).collect();
        let cols = self
            .cols
            .iter()
            .map(|c| Col::Num(Rc::new(c.iter().map(|v| v.unwrap_or(f64::NAN)).collect()), c.iter().flatten().all(|x| x.fract() == 0.0)))
            .collect();
        Table::new(names, cols)
    }

    pub fn plot_value(&self, x: Option<Value>, ys: Option<Value>, title: Option<Value>, log: Option<Value>) -> R {
        let step = match opt(x) {
            Some(x) => Some(self.find(Some(x.as_str("x")?), &[]).ok_or_else(|| value_err("x column not found"))?),
            None => self.find(None, STEP),
        };
        let ynames: Vec<String> = match opt(ys) {
            Some(Value::Str(s)) => vec![s.to_string()],
            Some(other) => {
                crate::stdlib::to_vec(&other, "y")?.iter().map(|v| Ok(v.as_str("y")?.to_string())).collect::<Result<_, Flow>>()?
            }
            None => [self.find(None, LOSS), self.find(None, VAL)].iter().flatten().map(|&i| self.names[i].clone()).collect(),
        };
        let mut series = Vec::new();
        for y in &ynames {
            let i = self.find(Some(y), &[]).ok_or_else(|| value_err(format!("no column `{y}` in log (have {})", self.names.join(", "))))?;
            series.push((y.clone(), self.series(i, step)));
        }
        let title = opt(title).map_or(Ok(ynames.join(", ")), |v| Ok::<_, Flow>(v.as_str("title")?.to_string()))?;
        let xl = step.map_or("row".to_string(), |s| self.names[s].clone());
        Ok(chart_value(&title, line_svg(&title, &xl, &series, log.is_some_and(|l| l.truthy()))))
    }
}

fn fix_all(re: &regex::Regex, s: &str) -> String {
    let mut cur = s.to_string();
    loop {
        let next = re.replace_all(&cur, "$1\"__mpp_$2\"$3").to_string();
        if next == cur {
            return cur;
        }
        cur = next;
    }
}

struct Issue {
    kind: &'static str,
    severity: &'static str,
    step: f64,
    message: String,
}

pub struct CheckCols {
    pub step: Option<String>,
    pub loss: Option<String>,
    pub val: Option<String>,
    pub grad: Option<String>,
    pub lr: Option<String>,
    pub speed: Option<String>,
    pub window: usize,
}

pub fn check(log: &TrainLog, c: &CheckCols) -> Result<Value, Flow> {
    let step = log.find(c.step.as_deref(), STEP);
    let loss_i = log
        .find(c.loss.as_deref(), LOSS)
        .ok_or_else(|| value_err(format!("no loss column found (have {}); pass loss = \"name\"", log.names.join(", "))))?;
    let mut issues: Vec<Issue> = Vec::new();
    let loss = log.series(loss_i, step);
    if loss.len() < 2 {
        return Err(value_err("need at least 2 logged loss values"));
    }
    // steps must go up
    let steps: Vec<f64> = loss.iter().map(|p| p.0).collect();
    if let Some(w) = steps.windows(2).find(|w| w[1] <= w[0]) {
        issues.push(Issue {
            kind: "steps",
            severity: "warn",
            step: w[1],
            message: format!("step goes from {} back to {} (restart or resumed run?)", w[0], w[1]),
        });
    }
    // 1. non-finite loss
    if let Some(p) = loss.iter().find(|p| !p.1.is_finite()) {
        issues.push(Issue {
            kind: "nan_loss",
            severity: "error",
            step: p.0,
            message: format!("loss became {} at step {}", fmt_float(p.1), p.0),
        });
    }
    let fin: Vec<(f64, f64)> = loss.iter().copied().filter(|p| p.1.is_finite()).collect();
    let vals: Vec<f64> = fin.iter().map(|p| p.1).collect();
    let n = vals.len();
    // 2. spikes: a jump far bigger than normal step-to-step noise, to a new local high
    let w = c.window.max(5);
    let mut spikes = Vec::new();
    for i in w.min(n)..n {
        let prev = &vals[i - w..i];
        let diffs: Vec<f64> = prev.windows(2).map(|p| p[1] - p[0]).collect();
        let md = desc::median(&diffs);
        let mad = desc::median(&diffs.iter().map(|d| (d - md).abs()).collect::<Vec<_>>());
        let scale = (1.4826 * mad).max(1e-6 * desc::median(prev).abs()).max(1e-12);
        let jump = vals[i] - vals[i - 1];
        if (jump - md) / scale > 6.0 && vals[i] > desc::max(prev) {
            spikes.push(fin[i].0);
        }
    }
    if !spikes.is_empty() {
        let shown: Vec<String> = spikes.iter().take(5).map(|s| fmt_float(*s)).collect();
        issues.push(Issue {
            kind: "loss_spike",
            severity: "warn",
            step: spikes[0],
            message: format!("{} loss spike(s), first at step(s) {}", spikes.len(), shown.join(", ")),
        });
    }
    // 3. divergence
    let (min_loss, min_at) = fin.iter().fold((f64::INFINITY, 0.0), |b, p| if p.1 < b.0 { (p.1, p.0) } else { b });
    if n >= 10 {
        let tail = &vals[n - (n / 10).max(2)..];
        let head = &vals[..(n / 10).max(2)];
        let (mt, mh) = (desc::mean(tail), desc::mean(head));
        if mt > mh {
            issues.push(Issue {
                kind: "divergence",
                severity: "error",
                step: fin[n - 1].0,
                message: format!("loss at the end ({}) is above the start ({}): training diverged", short(mt), short(mh)),
            });
        } else if mt > min_loss * 1.5 && mt - min_loss > 1e-3 * min_loss.abs().max(1.0) {
            issues.push(Issue {
                kind: "divergence",
                severity: "warn",
                step: fin[n - 1].0,
                message: format!("loss rose back up: best {} at step {}, now {}", short(min_loss), min_at, short(mt)),
            });
        }
    }
    // 4. plateau over the last quarter
    if n >= 40 {
        let q = &vals[n - n / 4..];
        let half = q.len() / 2;
        let (a, b) = (desc::mean(&q[..half]), desc::mean(&q[half..]));
        if a.abs() > 0.0 && (a - b) / a.abs() < 0.001 && (b - a).abs() / a.abs() < 0.01 {
            issues.push(Issue {
                kind: "plateau",
                severity: "info",
                step: fin[n - n / 4].0,
                message: format!("loss flat for the last {} logs (change under 0.1%)", q.len()),
            });
        }
    }
    let mut summary = Rec::default()
        .int("logged", n as i64)
        .num("first_loss", vals[0])
        .num("final_loss", vals[n - 1])
        .num("min_loss", min_loss)
        .num("min_loss_step", min_at);
    // 5. overfitting from validation loss
    if let Some(vi) = log.find(c.val.as_deref(), VAL) {
        let val = log.series(vi, step);
        let vf: Vec<(f64, f64)> = val.iter().copied().filter(|p| p.1.is_finite()).collect();
        if val.iter().any(|p| !p.1.is_finite()) {
            issues.push(Issue {
                kind: "nan_val_loss",
                severity: "error",
                step: val.iter().find(|p| !p.1.is_finite()).unwrap().0,
                message: "validation loss is NaN/inf".into(),
            });
        }
        if let Some(&(best_step, best)) = vf.iter().min_by(|a, b| a.1.total_cmp(&b.1)) {
            let last = vf.last().unwrap();
            summary = summary.num("final_val_loss", last.1).num("best_val_loss", best).num("best_val_step", best_step);
            let train_at = |s: f64| fin.iter().rfind(|p| p.0 <= s).map(|p| p.1);
            if vf.len() >= 3 && last.1 > best * 1.05 && last.0 > best_step {
                let falling = matches!((train_at(best_step), train_at(last.0)), (Some(a), Some(b)) if b < a);
                if falling {
                    issues.push(Issue {
                        kind: "overfitting",
                        severity: "warn",
                        step: best_step,
                        message: format!(
                            "validation loss rose from {} (step {}) to {} while train loss kept falling; best checkpoint is step {}",
                            short(best),
                            best_step,
                            short(last.1),
                            best_step
                        ),
                    });
                }
            }
            if let Some(t) = train_at(last.0) {
                summary = summary.num("generalization_gap", last.1 - t);
            }
        }
    }
    // 6. gradient norm
    if let Some(gi) = log.find(c.grad.as_deref(), GRAD) {
        let g = log.series(gi, step);
        if let Some(p) = g.iter().find(|p| !p.1.is_finite()) {
            issues.push(Issue {
                kind: "nan_grad",
                severity: "error",
                step: p.0,
                message: format!("grad norm became {} at step {}", fmt_float(p.1), p.0),
            });
        }
        let gv: Vec<f64> = g.iter().map(|p| p.1).filter(|v| v.is_finite()).collect();
        if !gv.is_empty() {
            let med = desc::median(&gv);
            let big: Vec<&(f64, f64)> = g.iter().filter(|p| p.1.is_finite() && p.1 > 10.0 * med && med > 0.0).collect();
            if !big.is_empty() {
                issues.push(Issue {
                    kind: "grad_explosion",
                    severity: "warn",
                    step: big[0].0,
                    message: format!("{} grad-norm value(s) above 10x the median ({}), first at step {}", big.len(), short(med), big[0].0),
                });
            }
            let tail = &gv[gv.len() - (gv.len() / 5).max(1)..];
            if desc::median(tail) < 1e-7 {
                issues.push(Issue {
                    kind: "grad_vanishing",
                    severity: "warn",
                    step: g[g.len() - tail.len()].0,
                    message: "grad norm is ~0 at the end (vanishing gradients or frozen weights)".into(),
                });
            }
            summary = summary.num("median_grad_norm", med);
        }
    }
    // 7. learning-rate schedule
    if let Some(li) = log.find(c.lr.as_deref(), LR) {
        let lr = log.series(li, step);
        if let Some(p) = lr.iter().find(|p| p.1 < 0.0 || !p.1.is_finite()) {
            issues.push(Issue {
                kind: "lr_invalid",
                severity: "error",
                step: p.0,
                message: format!("learning rate is {} at step {}", fmt_float(p.1), p.0),
            });
        }
        let lv: Vec<f64> = lr.iter().map(|p| p.1).collect();
        if !lv.is_empty() {
            let (pi, peak) = lv.iter().enumerate().fold((0, f64::MIN), |b, (i, v)| if *v > b.1 { (i, *v) } else { b });
            let warmup = pi > 0 && lv[0] < peak;
            let after = &lv[pi..];
            let rises = after.windows(2).filter(|w| w[1] > w[0] * 1.1 && w[1] - w[0] > 1e-12).count();
            let decays = after.last().is_some_and(|l| *l < peak * 0.99);
            if rises > 0 {
                issues.push(Issue {
                    kind: "lr_schedule",
                    severity: "info",
                    step: lr[pi].0,
                    message: format!("learning rate goes up {rises} time(s) after its peak (restarts or a bug)"),
                });
            }
            if lv.iter().take(lv.len().min(3)).all(|v| *v == 0.0) && peak > 0.0 && !warmup {
                issues.push(Issue {
                    kind: "lr_schedule",
                    severity: "warn",
                    step: lr[0].0,
                    message: "learning rate is 0 at the start".into(),
                });
            }
            let shape = match (warmup, decays) {
                (true, true) => "warmup then decay",
                (true, false) => "warmup then constant",
                (false, true) => "decay",
                (false, false) => "constant",
            };
            summary =
                summary.num("peak_lr", peak).num("peak_lr_step", lr[pi].0).num("final_lr", *lv.last().unwrap()).text("lr_shape", shape);
        }
    }
    // 8. throughput drops
    if let Some(si) = log.find(c.speed.as_deref(), SPEED) {
        let sp = log.series(si, step);
        let sv: Vec<f64> = sp.iter().map(|p| p.1).filter(|v| v.is_finite()).collect();
        if !sv.is_empty() {
            let med = desc::median(&sv);
            let slow: Vec<&(f64, f64)> = sp.iter().filter(|p| p.1 < 0.5 * med).collect();
            if !slow.is_empty() {
                issues.push(Issue {
                    kind: "throughput_drop",
                    severity: "warn",
                    step: slow[0].0,
                    message: format!("{} log(s) below half the median speed ({}), first at step {}", slow.len(), short(med), slow[0].0),
                });
            }
            summary = summary.num("median_throughput", med);
        }
    }
    let healthy = !issues.iter().any(|i| i.severity == "error");
    let rows: Vec<Value> = issues
        .iter()
        .map(|i| Rec::default().text("severity", i.severity).text("kind", i.kind).num("step", i.step).text("message", &i.message).value())
        .collect();
    let kinds: Vec<Value> = issues.iter().map(|i| Value::str(i.kind)).collect();
    Ok(Rec::default()
        .flag("healthy", healthy)
        .val("problems", Value::list(kinds))
        .val("issues", table_value(table_from_rows(&rows).unwrap_or(Table { names: vec![], cols: vec![], nrows: 0 })))
        .val("summary", summary.value())
        .value())
}

fn short(x: f64) -> String {
    format!("{}", (x * 1e4).round() / 1e4)
}

// compare two runs on a metric over their shared step range
pub fn compare(a: &TrainLog, b: &TrainLog, metric: Option<&str>) -> Result<Value, Flow> {
    let pick = |l: &TrainLog| -> Result<Vec<(f64, f64)>, Flow> {
        let i = l.find(metric, LOSS).ok_or_else(|| value_err(format!("{}: metric column not found", l.path)))?;
        Ok(l.series(i, l.find(None, STEP)).into_iter().filter(|p| p.1.is_finite()).collect())
    };
    let (sa, sb) = (pick(a)?, pick(b)?);
    if sa.len() < 2 || sb.len() < 2 {
        return Err(value_err("each run needs at least 2 logged values"));
    }
    let lo = sa[0].0.max(sb[0].0);
    let hi = sa.last().unwrap().0.min(sb.last().unwrap().0);
    if hi <= lo {
        return Err(value_err("runs do not share any step range"));
    }
    // b interpolated at a's steps
    let interp = |s: &[(f64, f64)], x: f64| -> f64 {
        let j = s.partition_point(|p| p.0 < x).min(s.len() - 1);
        if j == 0 || s[j].0 == x {
            return s[j].1;
        }
        let (p0, p1) = (s[j - 1], s[j]);
        p0.1 + (p1.1 - p0.1) * (x - p0.0) / (p1.0 - p0.0)
    };
    let common: Vec<(f64, f64, f64)> = sa.iter().filter(|p| p.0 >= lo && p.0 <= hi).map(|p| (p.0, p.1, interp(&sb, p.0))).collect();
    let mean_a = desc::mean(&common.iter().map(|c| c.1).collect::<Vec<_>>());
    let mean_b = desc::mean(&common.iter().map(|c| c.2).collect::<Vec<_>>());
    let last = common.last().unwrap();
    let rel = (last.2 - last.1) / last.1.abs().max(1e-12);
    let better = if rel.abs() < 0.01 {
        "same"
    } else if rel < 0.0 {
        "b"
    } else {
        "a"
    };
    let split = common.iter().find(|c| (c.2 - c.1).abs() > 0.1 * c.1.abs().max(1e-12)).map_or(Value::Nil, |c| Value::Float(c.0));
    Ok(Rec::default()
        .num("from_step", lo)
        .num("to_step", hi)
        .num("final_a", last.1)
        .num("final_b", last.2)
        .num("final_rel_diff", rel)
        .num("mean_a", mean_a)
        .num("mean_b", mean_b)
        .text("lower_is_better_winner", better)
        .val("first_10pct_gap_step", split)
        .value())
}

impl Object for TrainLog {
    fn type_name(&self) -> &'static str {
        "train_log"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn display(&self) -> String {
        format!("<train_log {} rows, columns: {}>", self.cols.first().map_or(0, Vec::len), self.names.join(", "))
    }
    fn len(&self) -> Option<usize> {
        Some(self.cols.first().map_or(0, Vec::len))
    }
    fn get(&self, name: &str) -> Option<Value> {
        match name {
            "columns" => Some(Value::list(self.names.iter().map(Value::str).collect())),
            "path" => Some(Value::str(&self.path)),
            "table" => self.table().ok().map(table_value),
            _ => None,
        }
    }
    fn methods(&self) -> &'static [&'static str] {
        METHODS
    }
    fn call_method(&self, _vm: &mut Vm, _this: &Value, name: &str, a: Args) -> R {
        match name {
            "check" => {
                let [loss, val, step, grad, lr, speed, window] =
                    a.bind(["loss", "val_loss", "step", "grad_norm", "lr", "throughput", "window"])?;
                let s = |v: Option<Value>, n: &str| -> Result<Option<String>, Flow> {
                    opt(v).map(|v| Ok(v.as_str(n)?.to_string())).transpose()
                };
                let cols = CheckCols {
                    loss: s(loss, "loss")?,
                    val: s(val, "val_loss")?,
                    step: s(step, "step")?,
                    grad: s(grad, "grad_norm")?,
                    lr: s(lr, "lr")?,
                    speed: s(speed, "throughput")?,
                    window: opt(window).map_or(Ok(50), |v| v.int("window"))?.max(5) as usize,
                };
                check(self, &cols)
            }
            "plot" => {
                let [ys, x, title, log] = a.bind(["y", "x", "title", "log"])?;
                self.plot_value(x, ys, title, log)
            }
            "compare" => {
                let [other, metric] = a.bind(["other", "metric"])?;
                let other = need(other, "other")?;
                let o = other.object::<TrainLog>().ok_or_else(|| type_err("compare() needs another train_log"))?;
                let m = opt(metric).map(|m| Ok::<_, Flow>(m.as_str("metric")?.to_string())).transpose()?;
                compare(self, o, m.as_deref())
            }
            "col" => {
                let [n] = a.bind(["name"])?;
                let n = need(n, "name")?;
                let i = self.find(Some(n.as_str("name")?), &[]).ok_or_else(|| value_err("no such column"))?;
                Ok(crate::stdlib::vec::vec_value(self.cols[i].iter().map(|v| v.unwrap_or(f64::NAN)).collect()))
            }
            _ => Err(err("AttributeError", format!("train_log has no method `{name}`"))),
        }
    }
}
