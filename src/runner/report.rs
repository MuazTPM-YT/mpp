use super::{FileResult, RunReport, Status, Summary, TestResult};
use serde_json::Value as J;
use std::fmt::Write as _;

pub struct Term {
    pub color: bool,
    pub verbose: bool,
}

impl Term {
    fn paint(&self, code: &str, s: &str) -> String {
        if self.color { format!("\x1b[{code}m{s}\x1b[0m") } else { s.to_string() }
    }
    fn green(&self, s: &str) -> String {
        self.paint("32", s)
    }
    fn red(&self, s: &str) -> String {
        self.paint("31", s)
    }
    fn yellow(&self, s: &str) -> String {
        self.paint("33", s)
    }
    fn dim(&self, s: &str) -> String {
        self.paint("2", s)
    }
    fn bold(&self, s: &str) -> String {
        self.paint("1", s)
    }

    pub fn file(&self, f: &FileResult) -> String {
        let mut o = String::new();
        let bad = f.results.iter().any(|t| matches!(t.status, Status::Failed | Status::Error));
        let tag = if bad { self.paint("1;41", " FAIL ") } else { self.paint("1;42", " PASS ") };
        let _ = writeln!(o, "{tag} {} {}", self.bold(&f.file), self.dim(&fmt_ms(f.duration_ms)));
        if self.verbose && !f.module_output.is_empty() {
            o.push_str(&indent(&f.module_output, 6));
        }
        for t in &f.results {
            o.push_str(&self.test(t));
        }
        o
    }

    fn test(&self, t: &TestResult) -> String {
        let mut o = String::new();
        let (mark, name) = match t.status {
            Status::Passed => (self.green("✓"), t.name.clone()),
            Status::Failed => (self.red("✗"), self.red(&t.name)),
            Status::Error => (self.red("!"), self.red(&t.name)),
            Status::Skipped => (self.yellow("○"), self.yellow(&t.name)),
        };
        let kind = if t.kind == "test" { String::new() } else { self.dim(&format!("{} ", t.kind)) };
        let mut extra = fmt_ms(t.duration_ms);
        if let Some(n) = t.runs {
            extra = format!("{n} cases, {extra}");
        }
        if t.attempts > 1 {
            extra = format!("{extra}, attempt {}", t.attempts);
        }
        let _ = writeln!(o, "  {mark} {kind}{name} {}", self.dim(&extra));
        if let Some(b) = &t.bench {
            let mut line = format!(
                "mean {} ± {} · p50 {} · p95 {} · {} ops/s · {} runs",
                fmt_ns(b.mean_ns),
                fmt_ns(b.sd_ns),
                fmt_ns(b.p50_ns),
                fmt_ns(b.p95_ns),
                fmt_count(b.ops_per_sec),
                b.iters
            );
            if let Some(c) = b.change {
                let s = format!("{:+.1}% vs baseline", c * 100.0);
                line = format!("{line} · {}", if c > 0.0 { self.yellow(&s) } else { self.green(&s) });
            }
            let _ = writeln!(o, "      {line}");
        }
        let show_details = self.verbose || t.kind == "experiment" || t.status != Status::Passed;
        if show_details {
            for r in &t.reports {
                o.push_str(&report_item(&r.label, &r.value, &r.text, 6));
            }
        }
        for n in &t.notes {
            let _ = writeln!(o, "      {} {n}", self.dim("note:"));
        }
        if let Some(m) = &t.message {
            let m = if t.status == Status::Skipped { format!("skipped: {m}") } else { m.clone() };
            if !m.trim().is_empty() || t.status != Status::Skipped {
                o.push_str(&indent(&m, 6));
            }
        }
        if let Some(cx) = &t.counterexample {
            let _ = writeln!(o, "      {} {cx}", self.bold("smallest failing input:"));
        }
        if let Some(l) = &t.location {
            let _ = writeln!(o, "      {}", self.dim(&format!("at {l}")));
        }
        if let Some(tr) = &t.trace {
            o.push_str(&self.dim(&indent(tr.trim_end(), 4)));
        }
        if (t.status != Status::Passed || self.verbose) && !t.output.is_empty() {
            let _ = writeln!(o, "      {}", self.dim("output:"));
            o.push_str(&indent(t.output.trim_end(), 8));
        }
        o
    }

    pub fn summary(&self, s: &Summary, secs: f64, seed: u64, mode: &str) -> String {
        let mut parts = vec![self.green(&format!("{} passed", s.passed))];
        if s.failed > 0 {
            parts.push(self.red(&format!("{} failed", s.failed)));
        }
        if s.errors > 0 {
            parts.push(self.red(&format!("{} errors", s.errors)));
        }
        if s.skipped > 0 {
            parts.push(self.yellow(&format!("{} skipped", s.skipped)));
        }
        let noun = match (mode, s.total) {
            ("bench", 1) => "bench",
            ("bench", _) => "benches",
            (_, 1) => "block",
            _ => "blocks",
        };
        let mut o =
            format!("\n{} {} {}\n", self.bold(&format!("{} {noun}:", s.total)), parts.join(", "), self.dim(&format!("in {secs:.2}s")));
        let _ = writeln!(o, "{}", self.dim(&format!("seed {seed}  (same run again: mpp {mode} --seed {seed})")));
        o
    }
}

fn indent(s: &str, n: usize) -> String {
    let pad = " ".repeat(n);
    s.lines().map(|l| format!("{pad}{l}\n")).collect()
}

// one reported value; maps print one key per line
fn report_item(label: &str, v: &J, text: &str, n: usize) -> String {
    let pad = " ".repeat(n);
    match v {
        J::Object(m) if !m.is_empty() => {
            let mut o = format!("{pad}{label}:\n");
            let w = m.keys().map(|k| k.chars().count()).max().unwrap_or(0);
            for (k, x) in m {
                let _ = writeln!(o, "{pad}  {k:<w$}  {}", short(x));
            }
            o
        }
        _ => format!("{pad}{label}: {}\n", if matches!(v, J::Number(_)) { short(v) } else { text.to_string() }),
    }
}

// compact text for a json value
pub fn short(v: &J) -> String {
    match v {
        J::Number(n) if n.is_f64() => {
            let x = n.as_f64().unwrap_or(0.0);
            if x != 0.0 && (x.abs() < 1e-3 || x.abs() >= 1e7) { format!("{x:.4e}") } else { format!("{}", (x * 1e6).round() / 1e6) }
        }
        J::String(s) => s.clone(),
        J::Array(a) if a.len() > 8 => format!("[{} items]", a.len()),
        other => other.to_string(),
    }
}

pub fn fmt_ms(ms: f64) -> String {
    fmt_ns(ms * 1e6)
}

pub fn fmt_ns(ns: f64) -> String {
    if ns < 1e3 {
        format!("{ns:.0}ns")
    } else if ns < 1e6 {
        format!("{:.2}µs", ns / 1e3)
    } else if ns < 1e9 {
        format!("{:.2}ms", ns / 1e6)
    } else {
        format!("{:.2}s", ns / 1e9)
    }
}

fn fmt_count(x: f64) -> String {
    if x >= 1e6 {
        format!("{:.2}M", x / 1e6)
    } else if x >= 1e3 {
        format!("{:.1}k", x / 1e3)
    } else {
        format!("{x:.1}")
    }
}

pub fn json(r: &RunReport) -> String {
    serde_json::to_string_pretty(r).unwrap_or_default() + "\n"
}

pub fn xml_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            c if (c as u32) < 0x20 && !matches!(c, '\n' | '\r' | '\t') => {}
            c => o.push(c),
        }
    }
    o
}

// JUnit XML, read by most CI systems
pub fn junit(r: &RunReport) -> String {
    let s = &r.summary;
    let mut o = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = writeln!(
        o,
        "<testsuites name=\"mpp\" tests=\"{}\" failures=\"{}\" errors=\"{}\" skipped=\"{}\" time=\"{:.3}\">",
        s.total, s.failed, s.errors, s.skipped, r.duration_s
    );
    for f in &r.files {
        let c = |st: Status| f.results.iter().filter(|t| t.status == st).count();
        let _ = writeln!(
            o,
            "  <testsuite name=\"{}\" tests=\"{}\" failures=\"{}\" errors=\"{}\" skipped=\"{}\" time=\"{:.3}\">",
            xml_escape(&f.file),
            f.results.len(),
            c(Status::Failed),
            c(Status::Error),
            c(Status::Skipped),
            f.duration_ms / 1000.0
        );
        for t in &f.results {
            let _ = write!(
                o,
                "    <testcase name=\"{}\" classname=\"{}\" time=\"{:.6}\">",
                xml_escape(&t.name),
                xml_escape(&f.file),
                t.duration_ms / 1000.0
            );
            let msg = t.message.clone().unwrap_or_default();
            let first = msg.lines().next().unwrap_or("");
            let mut body = msg.clone();
            if let Some(cx) = &t.counterexample {
                body.push_str(&format!("\nsmallest failing input: {cx}"));
            }
            if let Some(l) = &t.location {
                body.push_str(&format!("\nat {l}"));
            }
            match t.status {
                Status::Failed => {
                    let _ = write!(o, "\n      <failure message=\"{}\">{}</failure>", xml_escape(first), xml_escape(&body));
                }
                Status::Error => {
                    let _ = write!(o, "\n      <error message=\"{}\">{}</error>", xml_escape(first), xml_escape(&body));
                }
                Status::Skipped => {
                    let _ = write!(o, "\n      <skipped message=\"{}\"/>", xml_escape(first));
                }
                Status::Passed => {}
            }
            if !t.output.is_empty() {
                let _ = write!(o, "\n      <system-out>{}</system-out>", xml_escape(&t.output));
            }
            o.push_str(if t.status == Status::Passed && t.output.is_empty() { "</testcase>\n" } else { "\n    </testcase>\n" });
        }
        o.push_str("  </testsuite>\n");
    }
    o.push_str("</testsuites>\n");
    o
}
