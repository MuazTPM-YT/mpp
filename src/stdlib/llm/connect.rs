// talk to a local model: OpenAI-style HTTP, raw JSON HTTP, or a script over JSON lines
use serde_json::{Map, Value as J, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    Chat,
    Completions,
    Raw,
}

#[derive(Clone, Debug)]
pub struct HttpCfg {
    pub url: String,
    pub mode: Mode,
    pub model: Option<String>,
    pub headers: Vec<(String, String)>,
    pub timeout: f64,
    // raw mode: dotted path to the text in the reply, like "choices.0.text"
    pub field: Option<String>,
}

#[derive(Clone, Default, Debug)]
pub struct Req {
    pub prompt: Option<String>,
    pub messages: Option<Vec<J>>,
    pub system: Option<String>,
    pub params: Map<String, J>,
    pub stream: bool,
    pub logprobs: bool,
    // score the prompt itself (for perplexity)
    pub echo: bool,
}

#[derive(Clone, Default, Debug)]
pub struct GenOut {
    pub text: String,
    pub tokens_in: Option<i64>,
    pub tokens_out: Option<i64>,
    pub latency_ms: f64,
    pub ttft_ms: Option<f64>,
    pub logprobs: Option<Vec<(String, f64)>>,
    pub finish: Option<String>,
}

fn endpoint(cfg: &HttpCfg) -> String {
    let base = cfg.url.trim_end_matches('/');
    match cfg.mode {
        Mode::Raw => cfg.url.clone(),
        _ if base.ends_with("/completions") => base.to_string(),
        Mode::Chat if base.ends_with("/v1") => format!("{base}/chat/completions"),
        Mode::Completions if base.ends_with("/v1") => format!("{base}/completions"),
        Mode::Chat => format!("{base}/v1/chat/completions"),
        Mode::Completions => format!("{base}/v1/completions"),
    }
}

fn messages(req: &Req) -> Vec<J> {
    if let Some(m) = &req.messages {
        let mut out = Vec::new();
        if let Some(s) = &req.system
            && !m.iter().any(|x| x["role"] == "system")
        {
            out.push(json!({"role": "system", "content": s}));
        }
        out.extend(m.iter().cloned());
        return out;
    }
    let mut out = Vec::new();
    if let Some(s) = &req.system {
        out.push(json!({"role": "system", "content": s}));
    }
    out.push(json!({"role": "user", "content": req.prompt.clone().unwrap_or_default()}));
    out
}

// flat prompt text for completion-style backends
fn prompt_text(req: &Req) -> String {
    if let Some(p) = &req.prompt {
        return match &req.system {
            Some(s) => format!("{s}\n\n{p}"),
            None => p.clone(),
        };
    }
    messages(req)
        .iter()
        .map(|m| format!("{}: {}", m["role"].as_str().unwrap_or(""), m["content"].as_str().unwrap_or("")))
        .collect::<Vec<_>>()
        .join("\n")
}

fn body(cfg: &HttpCfg, req: &Req) -> J {
    let mut b = Map::new();
    match cfg.mode {
        Mode::Chat => {
            b.insert("model".into(), json!(cfg.model.clone().unwrap_or_else(|| "default".into())));
            b.insert("messages".into(), J::Array(messages(req)));
            if req.logprobs {
                b.insert("logprobs".into(), json!(true));
            }
        }
        Mode::Completions => {
            b.insert("model".into(), json!(cfg.model.clone().unwrap_or_else(|| "default".into())));
            b.insert("prompt".into(), json!(prompt_text(req)));
            if req.logprobs || req.echo {
                b.insert("logprobs".into(), json!(1));
            }
            if req.echo {
                b.insert("echo".into(), json!(true));
                b.insert("max_tokens".into(), json!(1));
            }
        }
        Mode::Raw => {
            b.insert("prompt".into(), json!(prompt_text(req)));
            if let Some(m) = &req.messages {
                b.insert("messages".into(), J::Array(m.clone()));
            }
            if req.logprobs {
                b.insert("logprobs".into(), json!(true));
            }
            if req.echo {
                b.insert("echo".into(), json!(true));
            }
        }
    }
    for (k, v) in &req.params {
        if !(req.echo && k == "max_tokens") {
            b.insert(k.clone(), v.clone());
        }
    }
    if req.stream {
        b.insert("stream".into(), json!(true));
    }
    J::Object(b)
}

// "choices.0.text" style lookup
pub fn dig<'a>(v: &'a J, path: &str) -> Option<&'a J> {
    let mut cur = v;
    for part in path.split('.').filter(|p| !p.is_empty()) {
        cur = match part.parse::<usize>() {
            Ok(i) => cur.get(i)?,
            Err(_) => cur.get(part)?,
        };
    }
    Some(cur)
}

fn logprobs_of(choice: &J) -> Option<Vec<(String, f64)>> {
    let lp = choice.get("logprobs")?;
    // chat style: {"content": [{"token", "logprob"}]}
    if let Some(items) = lp.get("content").and_then(|c| c.as_array()) {
        return Some(items.iter().filter_map(|t| Some((t["token"].as_str().unwrap_or("").to_string(), t["logprob"].as_f64()?))).collect());
    }
    // completions style: {"tokens": [...], "token_logprobs": [...]}
    if let (Some(toks), Some(lps)) = (lp.get("tokens").and_then(|t| t.as_array()), lp.get("token_logprobs").and_then(|t| t.as_array())) {
        return Some(toks.iter().zip(lps).filter_map(|(t, l)| Some((t.as_str().unwrap_or("").to_string(), l.as_f64()?))).collect());
    }
    // plain list: [[token, lp], ...] or [{"token", "logprob"}] or [lp, ...]
    if let Some(items) = lp.as_array() {
        return Some(
            items
                .iter()
                .filter_map(|t| match t {
                    J::Array(p) => Some((p.first()?.as_str().unwrap_or("").to_string(), p.get(1)?.as_f64()?)),
                    J::Object(_) => Some((t["token"].as_str().unwrap_or("").to_string(), t["logprob"].as_f64()?)),
                    J::Number(n) => Some((String::new(), n.as_f64()?)),
                    _ => None,
                })
                .collect(),
        );
    }
    None
}

// pull the answer out of a reply JSON
pub fn parse_reply(mode: Mode, field: Option<&str>, v: &J) -> Result<GenOut, String> {
    if let Some(e) = v.get("error")
        && !e.is_null()
    {
        return Err(format!("model returned an error: {}", e.as_str().map(String::from).unwrap_or_else(|| e.to_string())));
    }
    let mut out = GenOut::default();
    let usage = v.get("usage");
    out.tokens_in = usage.and_then(|u| u["prompt_tokens"].as_i64()).or_else(|| v["tokens_in"].as_i64());
    out.tokens_out = usage.and_then(|u| u["completion_tokens"].as_i64()).or_else(|| v["tokens_out"].as_i64());
    match (mode, field) {
        (_, Some(f)) => {
            let t = dig(v, f).ok_or_else(|| format!("reply has no field `{f}`: {}", short(v)))?;
            out.text = t.as_str().map(String::from).unwrap_or_else(|| t.to_string());
            out.logprobs = logprobs_of(v);
        }
        (Mode::Chat, None) | (Mode::Completions, None) if v.get("choices").is_some() => {
            let c = &v["choices"][0];
            out.text = c["message"]["content"].as_str().or_else(|| c["text"].as_str()).unwrap_or("").to_string();
            out.finish = c["finish_reason"].as_str().map(String::from);
            out.logprobs = logprobs_of(c);
        }
        _ => {
            let keys = ["text", "output", "response", "generated_text", "completion", "content", "answer"];
            let found =
                keys.iter().find_map(|k| v.get(*k)).or_else(|| v.as_array().and_then(|a| a.first()).and_then(|x| x.get("generated_text")));
            out.text = match (found, v) {
                (Some(t), _) => t.as_str().map(String::from).unwrap_or_else(|| t.to_string()),
                (None, J::String(s)) => s.clone(),
                _ => return Err(format!("cannot find the text in the reply; set field = \"path.to.text\". Reply: {}", short(v))),
            };
            out.logprobs = logprobs_of(v);
        }
    }
    Ok(out)
}

fn short(v: &J) -> String {
    let s = v.to_string();
    if s.chars().count() > 300 { format!("{}…", s.chars().take(300).collect::<String>()) } else { s }
}

fn agent(timeout: f64) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs_f64(timeout.max(0.001))))
        .http_status_as_error(false)
        .build()
        .new_agent()
}

pub fn http_generate(cfg: &HttpCfg, req: &Req) -> Result<GenOut, String> {
    let url = endpoint(cfg);
    let mut rb = agent(cfg.timeout).post(&url).header("Content-Type", "application/json");
    for (k, v) in &cfg.headers {
        rb = rb.header(k.as_str(), v.as_str());
    }
    let t0 = Instant::now();
    let mut resp = rb.send_json(body(cfg, req)).map_err(|e| format!("cannot reach model at {url}: {e}"))?;
    let status = resp.status().as_u16();
    if status >= 400 {
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        return Err(format!("model at {url} answered HTTP {status}: {}", text.chars().take(300).collect::<String>()));
    }
    if req.stream {
        return read_stream(cfg, resp.into_body().into_reader(), t0);
    }
    let text = resp.body_mut().read_to_string().map_err(|e| format!("reading reply from {url}: {e}"))?;
    let latency_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let v: J = serde_json::from_str(&text).unwrap_or(J::String(text));
    let mut out = parse_reply(cfg.mode, cfg.field.as_deref(), &v)?;
    out.latency_ms = latency_ms;
    Ok(out)
}

// server-sent events: "data: {...}" lines until [DONE]
fn read_stream(cfg: &HttpCfg, r: impl std::io::Read, t0: Instant) -> Result<GenOut, String> {
    let mut out = GenOut::default();
    let mut chunks = 0;
    for line in BufReader::new(r).lines() {
        let line = line.map_err(|e| format!("stream broke: {e}"))?;
        let Some(data) = line.strip_prefix("data:") else { continue };
        let data = data.trim();
        if data == "[DONE]" {
            break;
        }
        let Ok(v) = serde_json::from_str::<J>(data) else { continue };
        let c = &v["choices"][0];
        let piece = c["delta"]["content"]
            .as_str()
            .or_else(|| c["text"].as_str())
            .or_else(|| cfg.field.as_deref().and_then(|f| dig(&v, f)).and_then(|x| x.as_str()))
            .unwrap_or("");
        if !piece.is_empty() {
            if out.ttft_ms.is_none() {
                out.ttft_ms = Some(t0.elapsed().as_secs_f64() * 1000.0);
            }
            out.text.push_str(piece);
            chunks += 1;
        }
        if let Some(f) = c["finish_reason"].as_str() {
            out.finish = Some(f.to_string());
        }
        if let Some(u) = v.get("usage").filter(|u| !u.is_null()) {
            out.tokens_in = u["prompt_tokens"].as_i64();
            out.tokens_out = u["completion_tokens"].as_i64();
        }
    }
    out.latency_ms = t0.elapsed().as_secs_f64() * 1000.0;
    if out.tokens_out.is_none() {
        out.tokens_out = Some(chunks);
    }
    Ok(out)
}

// a long-running script: one JSON request line in, one JSON reply line out
pub struct Proc {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<std::io::Result<String>>,
    pub cmd: String,
}

impl Proc {
    pub fn start(cmd: &str, cwd: Option<&str>) -> Result<Proc, String> {
        let (sh, flag) = if cfg!(windows) { ("cmd", "/C") } else { ("sh", "-c") };
        let mut c = Command::new(sh);
        c.arg(flag).arg(cmd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit());
        if let Some(d) = cwd {
            c.current_dir(d);
        }
        let mut child = c.spawn().map_err(|e| format!("cannot start `{cmd}`: {e}"))?;
        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Proc { child, stdin, lines: rx, cmd: cmd.to_string() })
    }

    pub fn generate(&mut self, req: &Req, timeout: f64) -> Result<GenOut, String> {
        let mut msg = Map::new();
        msg.insert("prompt".into(), json!(prompt_text(req)));
        if let Some(m) = &req.messages {
            msg.insert("messages".into(), J::Array(m.clone()));
        }
        if let Some(s) = &req.system {
            msg.insert("system".into(), json!(s));
        }
        msg.insert("logprobs".into(), json!(req.logprobs));
        msg.insert("echo".into(), json!(req.echo));
        for (k, v) in &req.params {
            msg.insert(k.clone(), v.clone());
        }
        let t0 = Instant::now();
        writeln!(self.stdin, "{}", J::Object(msg))
            .and_then(|_| self.stdin.flush())
            .map_err(|e| format!("`{}` stopped reading input: {e}", self.cmd))?;
        loop {
            let left = Duration::from_secs_f64(timeout).saturating_sub(t0.elapsed());
            let line = match self.lines.recv_timeout(left) {
                Ok(Ok(l)) => l,
                Ok(Err(e)) => return Err(format!("reading from `{}`: {e}", self.cmd)),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => return Err(format!("`{}` did not answer within {timeout}s", self.cmd)),
                Err(_) => {
                    let code = self.child.try_wait().ok().flatten().map(|s| s.to_string()).unwrap_or_else(|| "closed".into());
                    return Err(format!("`{}` exited ({code}) before answering", self.cmd));
                }
            };
            let line = line.trim();
            // scripts may print logs; only JSON object lines are replies
            if !line.starts_with('{') {
                continue;
            }
            let v: J = serde_json::from_str(line).map_err(|e| format!("`{}` sent bad JSON: {e}", self.cmd))?;
            let mut out = parse_reply(Mode::Raw, None, &v)?;
            out.latency_ms = t0.elapsed().as_secs_f64() * 1000.0;
            return Ok(out);
        }
    }
}

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_and_parsing() {
        let mut c = HttpCfg { url: "http://h:8080".into(), mode: Mode::Chat, model: None, headers: vec![], timeout: 1.0, field: None };
        assert_eq!(endpoint(&c), "http://h:8080/v1/chat/completions");
        c.url = "http://h/v1".into();
        c.mode = Mode::Completions;
        assert_eq!(endpoint(&c), "http://h/v1/completions");
        let v = json!({"choices": [{"message": {"content": "hi"}, "finish_reason": "stop", "logprobs": {"content": [{"token": "hi", "logprob": -0.5}]}}], "usage": {"prompt_tokens": 3, "completion_tokens": 1}});
        let o = parse_reply(Mode::Chat, None, &v).unwrap();
        assert_eq!((o.text.as_str(), o.tokens_in, o.logprobs.unwrap()[0].1), ("hi", Some(3), -0.5));
        let o = parse_reply(Mode::Raw, Some("data.out.0"), &json!({"data": {"out": ["yo"]}})).unwrap();
        assert_eq!(o.text, "yo");
        assert_eq!(parse_reply(Mode::Raw, None, &json!([{"generated_text": "hf"}])).unwrap().text, "hf");
    }
}
