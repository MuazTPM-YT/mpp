pub mod connect;
pub mod text;
pub mod train;

use super::json::{from_json, to_json};
use super::stats::desc;
use super::stats::tests::Rec;
use super::table::{Table, table_from_rows, table_value};
use crate::vm::*;
use connect::{GenOut, HttpCfg, Mode, Proc, Req};
use serde_json::{Map, Value as J};
use std::cell::RefCell;
use std::rc::Rc;
use train::TrainLog;

pub enum Backend {
    Http(HttpCfg),
    Proc(RefCell<Proc>, f64),
    // a Muaz++ function prompt -> text, for tests without a real model
    Func(Value),
}

pub struct Model {
    pub name: String,
    pub backend: Backend,
    pub defaults: Map<String, J>,
}

const MODEL_METHODS: &[&str] = &["generate", "ask", "chat", "stream", "batch", "logprobs", "perplexity"];

fn gen_value(o: &GenOut) -> Value {
    let tps = match (o.tokens_out, o.latency_ms) {
        (Some(t), l) if l > 0.0 => Value::Float(t as f64 / (l / 1000.0)),
        _ => Value::Nil,
    };
    let lp = o
        .logprobs
        .as_ref()
        .map_or(Value::Nil, |l| Value::list(l.iter().map(|(t, p)| Rec::default().text("token", t).num("logprob", *p).value()).collect()));
    Rec::default()
        .text("text", &o.text)
        .val("tokens_in", o.tokens_in.map_or(Value::Nil, Value::Int))
        .val("tokens_out", o.tokens_out.map_or(Value::Nil, Value::Int))
        .num("latency_ms", o.latency_ms)
        .val("ttft_ms", o.ttft_ms.map_or(Value::Nil, Value::Float))
        .val("tokens_per_sec", tps)
        .val("logprobs", lp)
        .val("finish_reason", o.finish.as_deref().map_or(Value::Nil, Value::str))
        .value()
}

fn llm_err(msg: String) -> Flow {
    err("ModelError", msg)
}

impl Model {
    // one request, any backend
    pub fn run(&self, vm: &mut Vm, req: &Req) -> Result<GenOut, Flow> {
        match &self.backend {
            Backend::Http(cfg) => connect::http_generate(cfg, req).map_err(llm_err),
            Backend::Proc(p, t) => p.borrow_mut().generate(req, *t).map_err(llm_err),
            Backend::Func(f) => {
                let t0 = std::time::Instant::now();
                let prompt = Value::str(
                    req.prompt
                        .clone()
                        .unwrap_or_else(|| req.messages.as_ref().map(|m| J::Array(m.clone()).to_string()).unwrap_or_default()),
                );
                let params = Value::map(req.params.iter().map(|(k, v)| (Key::Str(k.as_str().into()), from_json(v.clone()))).collect());
                let r = vm.call(f, &[prompt, params])?;
                let text = match r {
                    Value::Str(s) => s.to_string(),
                    other => vm.display(&other, false)?,
                };
                Ok(GenOut {
                    tokens_out: Some(text.split_whitespace().count() as i64),
                    text,
                    latency_ms: t0.elapsed().as_secs_f64() * 1000.0,
                    ..Default::default()
                })
            }
        }
    }

    // many requests; HTTP runs `conc` at a time on threads, others in order
    pub fn run_many(&self, vm: &mut Vm, reqs: &[Req], conc: usize) -> Vec<Result<GenOut, String>> {
        match &self.backend {
            Backend::Http(cfg) if conc > 1 => {
                let next = std::sync::atomic::AtomicUsize::new(0);
                let slots: Vec<std::sync::Mutex<Option<Result<GenOut, String>>>> =
                    reqs.iter().map(|_| std::sync::Mutex::new(None)).collect();
                std::thread::scope(|s| {
                    for _ in 0..conc.min(reqs.len()) {
                        s.spawn(|| {
                            loop {
                                let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                if i >= reqs.len() {
                                    break;
                                }
                                *slots[i].lock().unwrap() = Some(connect::http_generate(cfg, &reqs[i]));
                            }
                        });
                    }
                });
                slots.into_iter().map(|m| m.into_inner().unwrap().unwrap_or_else(|| Err("not run".into()))).collect()
            }
            _ => reqs.iter().map(|r| self.run(vm, r).map_err(|e| flow_text(&e))).collect(),
        }
    }
}

fn flow_text(f: &Flow) -> String {
    match f {
        Flow::Throw(Value::Error(e)) => e.message.to_string(),
        Flow::Throw(v) => format!("{v:?}"),
        Flow::Exit(c) => format!("exit({c})"),
    }
}

// split call args: special words vs model params
struct GenArgs {
    system: Option<String>,
    logprobs: bool,
    stream: bool,
    concurrency: Option<usize>,
    params: Map<String, J>,
    rest: Vec<(Rc<str>, Value)>,
}

fn gen_args(kw: Vec<(Rc<str>, Value)>, defaults: &Map<String, J>, extra: &[&str]) -> Result<GenArgs, Flow> {
    let mut g = GenArgs { system: None, logprobs: false, stream: false, concurrency: None, params: defaults.clone(), rest: Vec::new() };
    for (k, v) in kw {
        match &*k {
            "system" => g.system = opt(Some(v)).map(|s| Ok::<_, Flow>(s.as_str("system")?.to_string())).transpose()?,
            "logprobs" => g.logprobs = v.truthy(),
            "stream" => g.stream = v.truthy(),
            "concurrency" => g.concurrency = Some(v.int("concurrency")?.clamp(1, 512) as usize),
            "params" => {
                if let Value::Map(m) = &v {
                    for (pk, pv) in m.borrow().iter() {
                        g.params.insert(pk.value().as_str("param name")?.to_string(), to_json(pv, 0)?);
                    }
                }
            }
            name if extra.contains(&name) => g.rest.push((k.clone(), v)),
            name => {
                g.params.insert(name.to_string(), to_json(&v, 0)?);
            }
        }
    }
    Ok(g)
}

fn take(rest: &[(Rc<str>, Value)], name: &str) -> Option<Value> {
    rest.iter().find(|(k, _)| &**k == name).map(|(_, v)| v.clone())
}

fn make_req(prompt: Option<String>, messages: Option<Vec<J>>, g: &GenArgs) -> Req {
    Req { prompt, messages, system: g.system.clone(), params: g.params.clone(), stream: g.stream, logprobs: g.logprobs, echo: false }
}

fn prompt_of(v: &Value) -> Result<(Option<String>, Option<Vec<J>>), Flow> {
    match v {
        Value::Str(s) => Ok((Some(s.to_string()), None)),
        Value::List(l) => Ok((None, Some(l.borrow().iter().map(|m| to_json(m, 0)).collect::<Result<_, _>>()?))),
        other => Err(type_err(format!("prompt must be a string (or a list of chat messages), got {}", other.kind_name()))),
    }
}

impl Object for Model {
    fn type_name(&self) -> &'static str {
        "model"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn display(&self) -> String {
        format!("<model {}>", self.name)
    }
    fn get(&self, name: &str) -> Option<Value> {
        match name {
            "name" => Some(Value::str(&self.name)),
            _ => None,
        }
    }
    fn methods(&self) -> &'static [&'static str] {
        MODEL_METHODS
    }
    fn call_method(&self, vm: &mut Vm, _this: &Value, name: &str, a: Args) -> R {
        let Args { pos, kw } = a;
        let mut g = gen_args(kw, &self.defaults, &[])?;
        let first = pos.first().cloned().ok_or_else(|| type_err(format!("{name}() needs a prompt")))?;
        if pos.len() > 1 {
            return Err(type_err(format!("{name}() takes one prompt; pass options by name, like temperature = 0")));
        }
        match name {
            "generate" | "ask" | "chat" | "stream" => {
                if name == "stream" {
                    g.stream = true;
                }
                let (p, m) = prompt_of(&first)?;
                let out = self.run(vm, &make_req(p, m, &g))?;
                Ok(if name == "ask" { Value::str(out.text) } else { gen_value(&out) })
            }
            "batch" => {
                let prompts = super::to_vec(&first, "prompts")?;
                let reqs: Vec<Req> = prompts.iter().map(|p| prompt_of(p).map(|(p, m)| make_req(p, m, &g))).collect::<Result<_, _>>()?;
                let res = self.run_many(vm, &reqs, g.concurrency.unwrap_or(4));
                let mut out = Vec::new();
                for (i, r) in res.into_iter().enumerate() {
                    match r {
                        Ok(o) => out.push(gen_value(&o)),
                        Err(e) => return Err(llm_err(format!("prompt #{i}: {e}"))),
                    }
                }
                Ok(Value::list(out))
            }
            "logprobs" => {
                g.logprobs = true;
                let (p, m) = prompt_of(&first)?;
                let out = self.run(vm, &make_req(p, m, &g))?;
                let lp =
                    out.logprobs.ok_or_else(|| llm_err("the model did not return logprobs (does your server support them?)".into()))?;
                Ok(Value::list(lp.iter().map(|(t, p)| Rec::default().text("token", t).num("logprob", *p).value()).collect()))
            }
            "perplexity" => {
                let text = first.as_str("text")?.to_string();
                let mut req = make_req(Some(text), None, &g);
                req.echo = true;
                let out = self.run(vm, &req)?;
                let lp = out.logprobs.ok_or_else(|| {
                    llm_err("perplexity needs prompt logprobs (completions API with echo); use llm.http(url, api = \"completions\")".into())
                })?;
                // with echo, the prompt tokens come first; drop the generated tail
                let n = out.tokens_in.map_or(lp.len(), |t| (t as usize).min(lp.len()));
                let vals: Vec<f64> = lp[..n].iter().map(|p| p.1).filter(|v| v.is_finite()).collect();
                if vals.is_empty() {
                    return Err(llm_err("no token logprobs to score".into()));
                }
                Ok(Rec::default()
                    .num("perplexity", (-desc::mean(&vals)).exp())
                    .num("mean_logprob", desc::mean(&vals))
                    .int("tokens", vals.len() as i64)
                    .value())
            }
            _ => Err(err("AttributeError", format!("model has no method `{name}`"))),
        }
    }
}

macro_rules! natives {
    ($($name:literal => $f:expr),* $(,)?) => {
        &[$(Native { name: $name, f: $f }),*]
    };
}

pub static FNS: &[Native] = natives![
    "http" => http,
    "process" => process,
    "mock" => mock,
    "eval_set" => eval_set,
    "compare_checkpoints" => compare_checkpoints,
    "determinism" => determinism,
    "consistency" => consistency,
    "judge" => judge,
    "load_test" => load_test,
    "latency_stats" => |_, a| {
        let [ms] = a.bind(["ms"])?;
        Ok(latency(&super::stats::nums(ms, "ms")?))
    },
    "exact_match" => |_, a| pairwise(a, |p, r, norm| if norm { (text::normalize(p) == text::normalize(r)) as i64 as f64 } else { (p == r) as i64 as f64 }),
    "token_f1" => |_, a| pairwise(a, |p, r, _| text::token_f1(p, r)),
    "similarity" => |_, a| pairwise(a, |p, r, _| text::similarity(p, r)),
    "edit_distance" => |_, a| {
        let [x, y] = a.bind(["a", "b"])?;
        Ok(Value::Int(text::edit_distance(need(x, "a")?.as_str("a")?, need(y, "b")?.as_str("b")?) as i64))
    },
    "contains" => |_, a| {
        let [t, s, case] = a.bind(["text", "sub", "case_sensitive"])?;
        let (t, s) = (need(t, "text")?.as_str("text")?.to_string(), need(s, "sub")?.as_str("sub")?.to_string());
        Ok(Value::Bool(if case.is_some_and(|c| c.truthy()) { t.contains(&s) } else { t.to_lowercase().contains(&s.to_lowercase()) }))
    },
    "regex_match" => |_, a| {
        let [t, p] = a.bind(["text", "pattern"])?;
        Ok(Value::Bool(regex_of(need(p, "pattern")?.as_str("pattern")?)?.is_match(need(t, "text")?.as_str("text")?)))
    },
    "extract" => |_, a| {
        let [t, p, g] = a.bind(["text", "pattern", "group"])?;
        let re = regex_of(need(p, "pattern")?.as_str("pattern")?)?;
        let t = need(t, "text")?;
        let gi = opt(g).map_or(Ok(if re.captures_len() > 1 { 1 } else { 0 }), |v| v.int("group"))? as usize;
        Ok(re.captures(t.as_str("text")?).and_then(|c| c.get(gi)).map_or(Value::Nil, |m| Value::str(m.as_str())))
    },
    "extract_all" => |_, a| {
        let [t, p] = a.bind(["text", "pattern"])?;
        let re = regex_of(need(p, "pattern")?.as_str("pattern")?)?;
        let t = need(t, "text")?;
        Ok(Value::list(re.find_iter(t.as_str("text")?).map(|m| Value::str(m.as_str())).collect()))
    },
    "json_valid" => |_, a| {
        let [t] = a.bind(["text"])?;
        Ok(Value::Bool(serde_json::from_str::<J>(need(t, "text")?.as_str("text")?.trim()).is_ok()))
    },
    "parse_json" => |_, a| {
        let [t] = a.bind(["text"])?;
        Ok(text::extract_json(need(t, "text")?.as_str("text")?).map_or(Value::Nil, from_json))
    },
    "json_schema" => |_, a| {
        let [v, s] = a.bind(["value", "schema"])?;
        let v = need(v, "value")?;
        let doc = match &v {
            Value::Str(s) => match text::extract_json(s) {
                Some(j) => j,
                None => return Ok(Rec::default().flag("valid", false).val("errors", Value::list(vec![Value::str("not valid JSON")])).value()),
            },
            other => to_json(other, 0)?,
        };
        let schema = to_json(&need(s, "schema")?, 0)?;
        let mut errs = Vec::new();
        text::schema_errors(&doc, &schema, "", &mut errs);
        Ok(Rec::default().flag("valid", errs.is_empty()).val("errors", Value::list(errs.into_iter().map(Value::str).collect())).value())
    },
    "bleu" => |_, a| {
        let [c, r, n, smooth] = a.bind(["candidate", "references", "max_n", "smooth"])?;
        let refs = refs_of(need(r, "references")?)?;
        let n = opt(n).map_or(Ok(4), |v| v.int("max_n"))?.clamp(1, 10) as usize;
        Ok(Value::Float(text::bleu(need(c, "candidate")?.as_str("candidate")?, &refs, n, smooth.is_some_and(|s| s.truthy()))))
    },
    "rouge" => |_, a| {
        let [c, r] = a.bind(["candidate", "reference"])?;
        let (c, r) = (need(c, "candidate")?.as_str("candidate")?.to_string(), need(r, "reference")?.as_str("reference")?.to_string());
        Ok(Rec::default().num("rouge1", text::rouge_n(&c, &r, 1).2).num("rouge2", text::rouge_n(&c, &r, 2).2).num("rougeL", text::rouge_l(&c, &r).2).value())
    },
    "rouge_l" => |_, a| pairwise(a, |p, r, _| text::rouge_l(p, r).2),
    "rouge_n" => |_, a| {
        let [c, r, n] = a.bind(["candidate", "reference", "n"])?;
        let n = opt(n).map_or(Ok(1), |v| v.int("n"))?.max(1) as usize;
        let (p, rc, f) = text::rouge_n(need(c, "candidate")?.as_str("candidate")?, need(r, "reference")?.as_str("reference")?, n);
        Ok(Rec::default().num("precision", p).num("recall", rc).num("f1", f).value())
    },
    "chrf" => |_, a| {
        let [c, r] = a.bind(["candidate", "references"])?;
        Ok(Value::Float(text::chrf(need(c, "candidate")?.as_str("candidate")?, &refs_of(need(r, "references")?)?, 6, 2.0)))
    },
    "refusal" => |_, a| {
        let [t] = a.bind(["text"])?;
        Ok(Value::Bool(text::is_refusal(need(t, "text")?.as_str("text")?)))
    },
    "pass_at_k" => |_, a| {
        let [n, c, k] = a.bind(["n", "correct", "k"])?;
        let (n, c, k) = (need(n, "n")?.int("n")?, need(c, "correct")?.int("correct")?, need(k, "k")?.int("k")?);
        if n < 1 || c < 0 || c > n || k < 1 || k > n {
            return Err(value_err("need 0 <= correct <= n and 1 <= k <= n"));
        }
        Ok(Value::Float(text::pass_at_k(n as u64, c as u64, k as u64)))
    },
    "perplexity_of" => |_, a| {
        let [lp] = a.bind(["logprobs"])?;
        let v = need(lp, "logprobs")?;
        let nums: Vec<f64> = super::to_vec(&v, "logprobs")?.iter().map(|x| match x {
            Value::Map(m) => m.borrow().get(&Key::Str("logprob".into())).map_or(Ok(f64::NAN), |l| l.num("logprob")),
            other => other.num("logprob"),
        }).collect::<Result<_, _>>()?;
        Ok(Value::Float((-desc::mean(&nums)).exp()))
    },
    "train_log" => |_, a| {
        let [p] = a.bind(["path"])?;
        Ok(Value::Object(Rc::new(log_of(need(p, "path")?)?)))
    },
    "check_training" => |vm, a| {
        let Args { mut pos, kw } = a;
        if pos.is_empty() {
            return Err(type_err("check_training() needs a log path, table or train_log"));
        }
        let src = pos.remove(0);
        let log = Value::Object(Rc::new(log_of(src)?));
        let Value::Object(o) = &log else { unreachable!() };
        o.call_method(vm, &log, "check", Args { pos, kw })
    },
    "compare_runs" => |vm, a| {
        let [x, y, metric] = a.bind(["a", "b", "metric"])?;
        let la = Value::Object(Rc::new(log_of(need(x, "a")?)?));
        let lb = Value::Object(Rc::new(log_of(need(y, "b")?)?));
        let Value::Object(o) = &la else { unreachable!() };
        let mut args = Args::new(vec![lb]);
        if let Some(m) = metric {
            args.kw.push(("metric".into(), m));
        }
        o.call_method(vm, &la, "compare", args)
    },
    "plot" => super::chart::plot,
];

fn log_of(v: Value) -> Result<TrainLog, Flow> {
    match &v {
        Value::Str(p) => TrainLog::load(p),
        Value::Object(_) if v.object::<TrainLog>().is_some() => {
            let l = v.object::<TrainLog>().unwrap();
            Ok(TrainLog { path: l.path.clone(), names: l.names.clone(), cols: l.cols.clone() })
        }
        Value::Object(_) if v.object::<Table>().is_some() => Ok(TrainLog::from_table(v.object::<Table>().unwrap())),
        other => Err(type_err(format!("expected a log file path, table or train_log, got {}", other.kind_name()))),
    }
}

fn regex_of(p: &str) -> Result<regex::Regex, Flow> {
    regex::Regex::new(p).map_err(|e| value_err(format!("bad regex: {e}")))
}

fn refs_of(v: Value) -> Result<Vec<String>, Flow> {
    match &v {
        Value::Str(s) => Ok(vec![s.to_string()]),
        other => super::to_vec(other, "references")?.iter().map(|r| Ok(r.as_str("reference")?.to_string())).collect(),
    }
}

// f(pred, ref) for strings, or mean over two lists
fn pairwise(a: Args, f: fn(&str, &str, bool) -> f64) -> R {
    let [p, r, norm] = a.bind(["pred", "ref", "normalize"])?;
    let norm = norm.is_none_or(|n| n.truthy());
    let (p, r) = (need(p, "pred")?, need(r, "ref")?);
    match (&p, &r) {
        (Value::Str(x), Value::Str(y)) => Ok(Value::Float(f(x, y, norm))),
        _ => {
            let (ps, rs) = (super::to_vec(&p, "pred")?, super::to_vec(&r, "ref")?);
            if ps.len() != rs.len() || ps.is_empty() {
                return Err(value_err("pred and ref lists need the same, non-zero length"));
            }
            let mut s = 0.0;
            for (x, y) in ps.iter().zip(&rs) {
                s += f(x.as_str("pred")?, y.as_str("ref")?, norm);
            }
            Ok(Value::Float(s / ps.len() as f64))
        }
    }
}

fn latency(ms: &[f64]) -> Value {
    let s = desc::sorted(ms);
    let q = |p: f64| desc::quantile_sorted(&s, p);
    Rec::default()
        .num("p50", q(0.5))
        .num("p90", q(0.9))
        .num("p95", q(0.95))
        .num("p99", q(0.99))
        .num("mean", desc::mean(ms))
        .num("max", desc::max(ms))
        .int("n", ms.len() as i64)
        .value()
}

// ---- connectors ----

fn defaults_from(rest: &[(Rc<str>, Value)], skip: &[&str]) -> Result<Map<String, J>, Flow> {
    let mut d = Map::new();
    for (k, v) in rest {
        if !skip.contains(&&**k) {
            d.insert(k.to_string(), to_json(v, 0)?);
        }
    }
    Ok(d)
}

fn model_value(name: String, backend: Backend, defaults: Map<String, J>) -> Value {
    Value::Object(Rc::new(Model { name, backend, defaults }))
}

// llm.http(url, mode = "openai" | "raw", api = "chat" | "completions", model, headers, timeout, field, **defaults)
fn http(_: &mut Vm, a: Args) -> R {
    let Args { pos, kw } = a;
    let url = pos.first().cloned().or_else(|| take(&kw, "url")).ok_or_else(|| type_err("llm.http() needs a url"))?;
    let url = url.as_str("url")?.to_string();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(value_err("url must start with http:// or https://"));
    }
    let s = |k: &str| -> Result<Option<String>, Flow> { opt(take(&kw, k)).map(|v| Ok(v.as_str(k)?.to_string())).transpose() };
    let mode = match (s("mode")?.as_deref(), s("api")?.as_deref()) {
        (None | Some("openai"), None | Some("chat")) => Mode::Chat,
        (None | Some("openai"), Some("completions")) => Mode::Completions,
        (Some("raw"), _) => Mode::Raw,
        (m, api) => {
            return Err(value_err(format!(
                "unknown mode/api {m:?}/{api:?}; use mode \"openai\" (api \"chat\" or \"completions\") or \"raw\""
            )));
        }
    };
    let mut headers = Vec::new();
    if let Some(Value::Map(h)) = opt(take(&kw, "headers")) {
        for (k, v) in h.borrow().iter() {
            headers.push((k.value().as_str("header name")?.to_string(), v.as_str("header value")?.to_string()));
        }
    }
    let timeout = opt(take(&kw, "timeout")).map_or(Ok(120.0), |v| v.num("timeout"))?;
    let cfg = HttpCfg { url: url.clone(), mode, model: s("model")?, headers, timeout, field: s("field")? };
    let defaults = defaults_from(&kw, &["url", "mode", "api", "model", "headers", "timeout", "field"])?;
    Ok(model_value(cfg.model.clone().unwrap_or(url), Backend::Http(cfg), defaults))
}

// llm.process("python infer.py", cwd, timeout, **defaults)
fn process(_: &mut Vm, a: Args) -> R {
    let Args { pos, kw } = a;
    let cmd = pos.first().cloned().ok_or_else(|| type_err("llm.process() needs a command"))?;
    let cmd = cmd.as_str("command")?.to_string();
    let cwd = opt(take(&kw, "cwd")).map(|v| Ok::<_, Flow>(v.as_str("cwd")?.to_string())).transpose()?;
    let timeout = opt(take(&kw, "timeout")).map_or(Ok(120.0), |v| v.num("timeout"))?;
    let p = Proc::start(&cmd, cwd.as_deref()).map_err(llm_err)?;
    let defaults = defaults_from(&kw, &["cwd", "timeout"])?;
    Ok(model_value(cmd, Backend::Proc(RefCell::new(p), timeout), defaults))
}

// llm.mock(fn(prompt, params) -> text): a fake model for wiring up tests
fn mock(_: &mut Vm, a: Args) -> R {
    let [f, name] = a.bind(["fn", "name"])?;
    let f = need(f, "fn")?;
    if !f.is_callable() {
        return Err(type_err("llm.mock() needs a function (prompt, params) => text"));
    }
    let name = opt(name).map_or(Ok("mock".to_string()), |v| Ok::<_, Flow>(v.as_str("name")?.to_string()))?;
    Ok(model_value(name, Backend::Func(f), Map::new()))
}

fn model_arg(v: &Value) -> Result<&Model, Flow> {
    v.object::<Model>().ok_or_else(|| type_err(format!("expected a model (llm.http / llm.process / llm.mock), got {}", v.kind_name())))
}

// ---- evaluation ----

struct Case {
    prompt: String,
    expected: Value,
    system: Option<String>,
}

fn field<'a>(m: &'a indexmap::IndexMap<Key, Value>, names: &[&str]) -> Option<&'a Value> {
    names.iter().find_map(|n| m.get(&Key::Str((*n).into())))
}

fn cases_of(v: &Value) -> Result<Vec<Case>, Flow> {
    let rows: Vec<Value> = match v.object::<Table>() {
        Some(t) => (0..t.nrows).map(|i| t.row(i)).collect(),
        None => super::to_vec(v, "cases")?,
    };
    let mut out = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        let Value::Map(m) = r else { return Err(type_err(format!("case #{i} must be a map like {{\"prompt\": ..., \"expected\": ...}}"))) };
        let m = m.borrow();
        let prompt = field(&m, &["prompt", "input", "question"])
            .ok_or_else(|| value_err(format!("case #{i} has no \"prompt\"")))?
            .as_str("prompt")?
            .to_string();
        let expected = field(&m, &["expected", "answer", "reference", "ref", "target"]).cloned().unwrap_or(Value::Nil);
        let system = field(&m, &["system"]).and_then(|s| s.as_str("system").ok()).map(|s| s.to_string());
        out.push(Case { prompt, expected, system });
    }
    if out.is_empty() {
        return Err(value_err("no cases"));
    }
    Ok(out)
}

// score one output against what we expected
fn score(vm: &mut Vm, metric: &Value, out: &str, expected: &Value) -> Result<f64, Flow> {
    if metric.is_callable() {
        let r = vm.call(metric, &[Value::str(out), expected.clone()])?;
        return match r {
            Value::Bool(b) => Ok(b as i64 as f64),
            other => other.num("metric result"),
        };
    }
    let name = metric.as_str("metric")?;
    let exp_str = || -> Result<String, Flow> {
        match expected {
            Value::Str(s) => Ok(s.to_string()),
            Value::Nil => Err(value_err(format!("metric {name:?} needs an \"expected\" value in each case"))),
            other => vm_free_display(other),
        }
    };
    Ok(match &**name {
        "exact" => (text::normalize(out) == text::normalize(&exp_str()?)) as i64 as f64,
        "exact_raw" => (out.trim() == exp_str()?.trim()) as i64 as f64,
        "contains" => out.to_lowercase().contains(&exp_str()?.to_lowercase()) as i64 as f64,
        "regex" => regex_of(&exp_str()?)?.is_match(out) as i64 as f64,
        "json" => text::extract_json(out).is_some() as i64 as f64,
        "json_schema" => {
            let Some(doc) = text::extract_json(out) else { return Ok(0.0) };
            let mut errs = Vec::new();
            text::schema_errors(&doc, &to_json(expected, 0)?, "", &mut errs);
            errs.is_empty() as i64 as f64
        }
        "token_f1" => text::token_f1(out, &exp_str()?),
        "bleu" => text::bleu(out, &[exp_str()?], 4, true),
        "rouge_l" => text::rouge_l(out, &exp_str()?).2,
        "chrf" => text::chrf(out, &[exp_str()?], 6, 2.0) / 100.0,
        "similarity" => text::similarity(&text::normalize(out), &text::normalize(&exp_str()?)),
        "refusal" => text::is_refusal(out) as i64 as f64,
        "not_refusal" => (!text::is_refusal(out)) as i64 as f64,
        other => {
            return Err(value_err(format!(
                "unknown metric {other:?} (exact, exact_raw, contains, regex, json, json_schema, token_f1, bleu, rouge_l, chrf, similarity, refusal, not_refusal, or a function)"
            )));
        }
    })
}

fn vm_free_display(v: &Value) -> Result<String, Flow> {
    Ok(match v {
        Value::Int(n) => n.to_string(),
        Value::Float(x) => fmt_float(*x),
        Value::Bool(b) => b.to_string(),
        other => to_json(other, 0)?.to_string(),
    })
}

struct EvalOut {
    scores: Vec<f64>,
    outputs: Vec<String>,
    errors: Vec<Option<String>>,
    latency: Vec<f64>,
}

fn run_eval(vm: &mut Vm, model: &Model, cases: &[Case], metric: &Value, g: &GenArgs) -> Result<EvalOut, Flow> {
    let reqs: Vec<Req> =
        cases.iter().map(|c| Req { system: c.system.clone().or(g.system.clone()), ..make_req(Some(c.prompt.clone()), None, g) }).collect();
    let res = model.run_many(vm, &reqs, g.concurrency.unwrap_or(4));
    let mut out = EvalOut { scores: Vec::new(), outputs: Vec::new(), errors: Vec::new(), latency: Vec::new() };
    for (c, r) in cases.iter().zip(res) {
        match r {
            Ok(o) => {
                out.scores.push(score(vm, metric, &o.text, &c.expected)?);
                out.latency.push(o.latency_ms);
                out.outputs.push(o.text);
                out.errors.push(None);
            }
            Err(e) => {
                out.scores.push(0.0);
                out.outputs.push(String::new());
                out.errors.push(Some(e));
            }
        }
    }
    Ok(out)
}

const EVAL_WORDS: &[&str] = &["metric", "threshold", "n_boot", "runs"];

fn metric_arg(rest: &[(Rc<str>, Value)]) -> Value {
    opt(take(rest, "metric")).unwrap_or(Value::str("exact"))
}

// eval_set(model, cases, metric = "exact", threshold = 0.5, concurrency = 4, **params)
fn eval_set(vm: &mut Vm, a: Args) -> R {
    let Args { pos, kw } = a;
    let [m, c] = [pos.first().cloned(), pos.get(1).cloned()];
    let model_v = need(m, "model")?;
    let model = model_arg(&model_v)?;
    let cases = cases_of(&need(c, "cases")?)?;
    let g = gen_args(kw, &model.defaults, EVAL_WORDS)?;
    let metric = metric_arg(&g.rest);
    let threshold = opt(take(&g.rest, "threshold")).map_or(Ok(0.5), |v| v.num("threshold"))?;
    let ev = run_eval(vm, model, &cases, &metric, &g)?;
    let rows: Vec<Value> = cases
        .iter()
        .enumerate()
        .map(|(i, c)| {
            Rec::default()
                .text("prompt", &c.prompt)
                .val("expected", c.expected.clone())
                .text("output", &ev.outputs[i])
                .num("score", ev.scores[i])
                .val("error", ev.errors[i].as_deref().map_or(Value::Nil, Value::str))
                .value()
        })
        .collect();
    let errors = ev.errors.iter().filter(|e| e.is_some()).count();
    let passed = ev.scores.iter().filter(|s| **s >= threshold).count();
    let mut r = Rec::default()
        .num("score", desc::mean(&ev.scores))
        .num("pass_rate", passed as f64 / cases.len() as f64)
        .int("passed", passed as i64)
        .int("n", cases.len() as i64)
        .int("errors", errors as i64);
    if !ev.latency.is_empty() {
        r = r.val("latency_ms", latency(&ev.latency));
    }
    Ok(r.val("results", table_value(table_from_rows(&rows)?)).value())
}

// same golden set on two checkpoints: is B better, worse, or the same?
fn compare_checkpoints(vm: &mut Vm, a: Args) -> R {
    let Args { pos, kw } = a;
    let (ma, mb, c) = (need(pos.first().cloned(), "model_a")?, need(pos.get(1).cloned(), "model_b")?, need(pos.get(2).cloned(), "cases")?);
    let (a_m, b_m) = (model_arg(&ma)?, model_arg(&mb)?);
    let cases = cases_of(&c)?;
    let g = gen_args(kw, &a_m.defaults, EVAL_WORDS)?;
    let metric = metric_arg(&g.rest);
    let reps = opt(take(&g.rest, "n_boot")).map_or(Ok(2000), |v| v.int("n_boot"))?.clamp(100, 1_000_000) as usize;
    let ea = run_eval(vm, a_m, &cases, &metric, &g)?;
    let gb = GenArgs {
        params: {
            let mut p = b_m.defaults.clone();
            p.extend(g.params.clone());
            p
        },
        ..g
    };
    let eb = run_eval(vm, b_m, &cases, &metric, &gb)?;
    let d: Vec<f64> = ea.scores.iter().zip(&eb.scores).map(|(x, y)| y - x).collect();
    let n = d.len();
    let mut boots = Vec::with_capacity(reps);
    for _ in 0..reps {
        let mut s = 0.0;
        for _ in 0..n {
            s += d[vm.rng.below(n as u64) as usize];
        }
        boots.push(s / n as f64);
    }
    let sb = desc::sorted(&boots);
    let (lo, hi) = (desc::quantile_sorted(&sb, 0.025), desc::quantile_sorted(&sb, 0.975));
    let wins = d.iter().filter(|x| **x > 0.0).count() as f64;
    let losses = d.iter().filter(|x| **x < 0.0).count() as f64;
    let sign_p =
        if wins + losses == 0.0 { 1.0 } else { (2.0 * super::stats::dist::binom_cdf(wins.min(losses), wins + losses, 0.5)).min(1.0) };
    let verdict = if lo > 0.0 {
        "better"
    } else if hi < 0.0 {
        "worse"
    } else {
        "no clear difference"
    };
    let regress: Vec<Value> = (0..n)
        .filter(|&i| d[i] < 0.0)
        .map(|i| {
            Rec::default()
                .text("prompt", &cases[i].prompt)
                .val("expected", cases[i].expected.clone())
                .text("output_a", &ea.outputs[i])
                .text("output_b", &eb.outputs[i])
                .num("score_a", ea.scores[i])
                .num("score_b", eb.scores[i])
                .value()
        })
        .collect();
    Ok(Rec::default()
        .num("score_a", desc::mean(&ea.scores))
        .num("score_b", desc::mean(&eb.scores))
        .num("diff", desc::mean(&d))
        .pair("ci", lo, hi)
        .int("wins", wins as i64)
        .int("losses", losses as i64)
        .int("ties", (n as f64 - wins - losses) as i64)
        .num("sign_test_p", sign_p)
        .text("verdict", verdict)
        .val("regressions", table_value(table_from_rows(&regress)?))
        .value())
}

// same prompt, same settings, many times: identical answers?
fn determinism(vm: &mut Vm, a: Args) -> R {
    let Args { pos, kw } = a;
    let model_v = need(pos.first().cloned(), "model")?;
    let model = model_arg(&model_v)?;
    let prompt = need(pos.get(1).cloned(), "prompt")?.as_str("prompt")?.to_string();
    let mut g = gen_args(kw, &model.defaults, &["runs"])?;
    let runs = opt(take(&g.rest, "runs")).map_or(Ok(5), |v| v.int("runs"))?.clamp(2, 1000) as usize;
    g.params.entry("temperature").or_insert(J::from(0));
    g.params.entry("seed").or_insert(J::from(0));
    let req = make_req(Some(prompt), None, &g);
    let reqs = vec![req; runs];
    let mut outs = Vec::new();
    for r in model.run_many(vm, &reqs, 1) {
        outs.push(r.map_err(llm_err)?.text);
    }
    let mut uniq = outs.clone();
    uniq.sort();
    uniq.dedup();
    Ok(Rec::default()
        .flag("deterministic", uniq.len() == 1)
        .int("unique_outputs", uniq.len() as i64)
        .val("outputs", Value::list(outs.iter().map(Value::str).collect()))
        .value())
}

// paraphrases of one question should get the same answer
fn consistency(vm: &mut Vm, a: Args) -> R {
    let Args { pos, kw } = a;
    let model_v = need(pos.first().cloned(), "model")?;
    let model = model_arg(&model_v)?;
    let prompts = super::to_vec(&need(pos.get(1).cloned(), "prompts")?, "prompts")?;
    if prompts.len() < 2 {
        return Err(value_err("consistency() needs at least 2 prompts"));
    }
    let g = gen_args(kw, &model.defaults, &["metric"])?;
    let metric = opt(take(&g.rest, "metric")).map_or(Ok("similarity".to_string()), |v| Ok::<_, Flow>(v.as_str("metric")?.to_string()))?;
    let reqs: Vec<Req> =
        prompts.iter().map(|p| Ok(make_req(Some(p.as_str("prompt")?.to_string()), None, &g))).collect::<Result<_, Flow>>()?;
    let outs: Vec<String> = model
        .run_many(vm, &reqs, g.concurrency.unwrap_or(4))
        .into_iter()
        .map(|r| r.map(|o| o.text).map_err(llm_err))
        .collect::<Result<_, _>>()?;
    let mut pair = Vec::new();
    for i in 0..outs.len() {
        for j in i + 1..outs.len() {
            let (x, y) = (text::normalize(&outs[i]), text::normalize(&outs[j]));
            pair.push(match metric.as_str() {
                "exact" => (x == y) as i64 as f64,
                "token_f1" => text::token_f1(&x, &y),
                _ => text::similarity(&x, &y),
            });
        }
    }
    Ok(Rec::default()
        .num("agreement", desc::mean(&pair))
        .num("min_pair", desc::min(&pair))
        .val("outputs", Value::list(outs.iter().map(Value::str).collect()))
        .value())
}

// your own model grades an answer with a rubric
fn judge(vm: &mut Vm, a: Args) -> R {
    let Args { pos, kw } = a;
    let model_v = need(pos.first().cloned(), "model")?;
    let model = model_arg(&model_v)?;
    let answer = need(pos.get(1).cloned(), "answer")?.as_str("answer")?.to_string();
    let rubric = need(pos.get(2).cloned(), "rubric")?.as_str("rubric")?.to_string();
    let mut g = gen_args(kw, &model.defaults, &["question", "scale"])?;
    let scale = opt(take(&g.rest, "scale")).map_or(Ok(10), |v| v.int("scale"))?.clamp(2, 100);
    let question = opt(take(&g.rest, "question")).map(|q| Ok::<_, Flow>(q.as_str("question")?.to_string())).transpose()?;
    g.params.entry("temperature").or_insert(J::from(0));
    let mut prompt = format!("You are a strict grader.\nRubric: {rubric}\n");
    if let Some(q) = &question {
        prompt.push_str(&format!("Question: {q}\n"));
    }
    prompt.push_str(&format!("Answer to grade:\n{answer}\n\nReply with only one whole number from 1 to {scale}."));
    let out = model.run(vm, &make_req(Some(prompt), None, &g))?;
    let re = regex::Regex::new(r"-?\d+(\.\d+)?").unwrap();
    let raw = re.find(&out.text).and_then(|m| m.as_str().parse::<f64>().ok());
    let s = raw.map(|x| ((x - 1.0) / (scale as f64 - 1.0)).clamp(0.0, 1.0));
    Ok(Rec::default()
        .val("score", s.map_or(Value::Nil, Value::Float))
        .val("rating", raw.map_or(Value::Nil, Value::Float))
        .int("scale", scale)
        .text("raw", &out.text)
        .value())
}

// hammer the model with N parallel requests; latency, throughput, errors
fn load_test(vm: &mut Vm, a: Args) -> R {
    let Args { pos, kw } = a;
    let model_v = need(pos.first().cloned(), "model")?;
    let model = model_arg(&model_v)?;
    let prompts = super::to_vec(&need(pos.get(1).cloned(), "prompts")?, "prompts")?;
    if prompts.is_empty() {
        return Err(value_err("load_test() needs prompts"));
    }
    let g = gen_args(kw, &model.defaults, &["requests"])?;
    let total = opt(take(&g.rest, "requests")).map_or(Ok(prompts.len() as i64), |v| v.int("requests"))?.clamp(1, 1_000_000) as usize;
    let conc = g.concurrency.unwrap_or(8);
    let reqs: Vec<Req> = (0..total)
        .map(|i| Ok(make_req(Some(prompts[i % prompts.len()].as_str("prompt")?.to_string()), None, &g)))
        .collect::<Result<_, Flow>>()?;
    let t0 = std::time::Instant::now();
    let res = model.run_many(vm, &reqs, conc);
    let secs = t0.elapsed().as_secs_f64();
    let ok: Vec<&GenOut> = res.iter().filter_map(|r| r.as_ref().ok()).collect();
    let errs: Vec<&String> = res.iter().filter_map(|r| r.as_ref().err()).collect();
    let lat: Vec<f64> = ok.iter().map(|o| o.latency_ms).collect();
    let ttft: Vec<f64> = ok.iter().filter_map(|o| o.ttft_ms).collect();
    let toks: i64 = ok.iter().filter_map(|o| o.tokens_out).sum();
    let mut r = Rec::default()
        .int("requests", total as i64)
        .int("ok", ok.len() as i64)
        .int("errors", errs.len() as i64)
        .num("error_rate", errs.len() as f64 / total as f64)
        .int("concurrency", if matches!(model.backend, Backend::Http(_)) { conc as i64 } else { 1 })
        .num("duration_s", secs)
        .num("requests_per_sec", total as f64 / secs.max(1e-9))
        .num("tokens_per_sec", toks as f64 / secs.max(1e-9));
    if !lat.is_empty() {
        r = r.val("latency_ms", latency(&lat));
    }
    if !ttft.is_empty() {
        r = r.val("ttft_ms", latency(&ttft));
    }
    if let Some(e) = errs.first() {
        r = r.text("first_error", e);
    }
    Ok(r.value())
}
