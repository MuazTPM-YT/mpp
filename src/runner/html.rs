use super::report::{fmt_ms, fmt_ns, short, xml_escape as esc};
use super::{RunReport, Status, TestResult};
use serde_json::Value as J;
use std::fmt::Write as _;

const STYLE: &str = r#"
:root{--bg:#f7f7f5;--card:#fff;--ink:#1d1d1f;--mute:#6b6b70;--line:#e3e3e0;--ok:#1f8a4c;--bad:#c62f2f;--warn:#b07400;--code:#f1f1ee}
@media (prefers-color-scheme:dark){:root{--bg:#141416;--card:#1d1d20;--ink:#ececee;--mute:#9a9aa2;--line:#2e2e33;--ok:#4cc27e;--bad:#ff6b6b;--warn:#e0a83a;--code:#26262b}}
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--ink);font:14px/1.5 system-ui,-apple-system,Segoe UI,sans-serif}
main{max-width:1000px;margin:0 auto;padding:24px 16px 64px}h1{font-size:22px;margin:0 0 4px}
.meta{color:var(--mute);font-size:12px;margin-bottom:20px}
.cards{display:grid;grid-template-columns:repeat(auto-fit,minmax(120px,1fr));gap:10px;margin-bottom:20px}
.card{background:var(--card);border:1px solid var(--line);border-radius:10px;padding:12px}.card b{display:block;font-size:24px}
.card.ok b{color:var(--ok)}.card.bad b{color:var(--bad)}.card.warn b{color:var(--warn)}
.bar{display:flex;gap:6px;margin-bottom:14px;flex-wrap:wrap}.bar button{background:var(--card);color:var(--ink);border:1px solid var(--line);border-radius:999px;padding:4px 12px;cursor:pointer;font:inherit}
.bar button.on{background:var(--ink);color:var(--bg)}
section{background:var(--card);border:1px solid var(--line);border-radius:10px;margin-bottom:14px;overflow:hidden}
section>h2{font-size:14px;margin:0;padding:10px 14px;border-bottom:1px solid var(--line);display:flex;justify-content:space-between;gap:8px}
details{border-bottom:1px solid var(--line)}details:last-child{border-bottom:0}
summary{padding:8px 14px;cursor:pointer;display:flex;gap:10px;align-items:baseline;list-style:none}summary::-webkit-details-marker{display:none}
.st{font-weight:700;width:14px;text-align:center}.passed .st{color:var(--ok)}.failed .st,.error .st{color:var(--bad)}.skipped .st{color:var(--warn)}
.nm{flex:1;overflow-wrap:anywhere}.kd{color:var(--mute);font-size:12px}.tm{color:var(--mute);font-size:12px;white-space:nowrap}
.body{padding:4px 14px 14px 38px}pre{background:var(--code);border-radius:6px;padding:10px;overflow-x:auto;margin:6px 0;font:12px/1.45 ui-monospace,Menlo,Consolas,monospace;white-space:pre-wrap}
table{border-collapse:collapse;margin:6px 0;font-size:13px}td,th{padding:3px 12px 3px 0;text-align:left;vertical-align:top}th{color:var(--mute);font-weight:500}
.lbl{color:var(--mute);font-size:12px;margin-top:8px}svg text{fill:var(--ink);font-size:11px}
.hide{display:none}
"#;

const SCRIPT: &str = r#"
document.querySelectorAll('.bar button').forEach(b=>b.onclick=()=>{
 document.querySelectorAll('.bar button').forEach(x=>x.classList.toggle('on',x===b));
 const f=b.dataset.f;document.querySelectorAll('details.t').forEach(d=>d.classList.toggle('hide',f!=='all'&&!d.classList.contains(f)));
 document.querySelectorAll('section').forEach(s=>s.classList.toggle('hide',!s.querySelector('details.t:not(.hide)')));
});
"#;

fn mark(s: Status) -> (&'static str, &'static str) {
    match s {
        Status::Passed => ("passed", "✓"),
        Status::Failed => ("failed", "✗"),
        Status::Error => ("error", "!"),
        Status::Skipped => ("skipped", "○"),
    }
}

fn value_html(v: &J, text: &str) -> String {
    match v {
        J::Object(m) if !m.is_empty() => {
            let mut o = String::from("<table>");
            for (k, x) in m {
                let _ = write!(o, "<tr><th>{}</th><td>{}</td></tr>", esc(k), esc(&short(x)));
            }
            o + "</table>"
        }
        J::Number(_) => esc(&short(v)),
        _ => format!("<pre>{}</pre>", esc(text)),
    }
}

fn test_html(t: &TestResult) -> String {
    let (cls, sym) = mark(t.status);
    let open = if matches!(t.status, Status::Failed | Status::Error) || t.kind == "experiment" { " open" } else { "" };
    let mut o = format!(
        "<details class=\"t {cls}\"{open}><summary><span class=st>{sym}</span><span class=nm>{}</span><span class=kd>{}</span><span class=tm>{}</span></summary><div class=body>",
        esc(&t.name),
        esc(&t.kind),
        fmt_ms(t.duration_ms)
    );
    if let Some(m) = &t.message {
        let _ = write!(o, "<pre>{}</pre>", esc(m));
    }
    if let Some(cx) = &t.counterexample {
        let _ = write!(o, "<div class=lbl>smallest failing input (seed {})</div><pre>{}</pre>", t.seed, esc(cx));
    }
    if let Some(l) = &t.location {
        let _ = write!(o, "<div class=lbl>at {}</div>", esc(l));
    }
    if let Some(tr) = &t.trace {
        let _ = write!(o, "<pre>{}</pre>", esc(tr));
    }
    if let Some(b) = &t.bench {
        let _ = write!(
            o,
            "<table><tr><th>mean</th><td>{}</td><th>sd</th><td>{}</td><th>p50</th><td>{}</td><th>p95</th><td>{}</td><th>runs</th><td>{}</td></tr></table>",
            fmt_ns(b.mean_ns),
            fmt_ns(b.sd_ns),
            fmt_ns(b.p50_ns),
            fmt_ns(b.p95_ns),
            b.iters
        );
        if let Some(c) = b.change {
            let _ = write!(o, "<div class=lbl>{:+.1}% vs baseline</div>", c * 100.0);
        }
    }
    for r in &t.reports {
        let _ = write!(o, "<div class=lbl>{}</div>{}", esc(&r.label), value_html(&r.value, &r.text));
    }
    for n in &t.notes {
        let _ = write!(o, "<div class=lbl>note: {}</div>", esc(n));
    }
    if !t.output.is_empty() {
        let _ = write!(o, "<div class=lbl>output</div><pre>{}</pre>", esc(&t.output));
    }
    if t.runs.is_some() || t.attempts > 1 {
        let _ = write!(o, "<div class=lbl>{} cases · attempt {}</div>", t.runs.unwrap_or(1), t.attempts);
    }
    o + "</div></details>"
}

// horizontal bars: mean, with a tick at p95
fn bench_chart(r: &RunReport) -> String {
    let rows: Vec<(String, f64, f64)> =
        r.files.iter().flat_map(|f| &f.results).filter_map(|t| t.bench.as_ref().map(|b| (t.name.clone(), b.mean_ns, b.p95_ns))).collect();
    if rows.is_empty() {
        return String::new();
    }
    let max = rows.iter().map(|r| r.2.max(r.1)).fold(0.0, f64::max).max(1.0);
    let (w, lw, h) = (640.0, 200.0, 22.0);
    let mut o = format!(
        "<section><h2>Benchmarks <span class=tm>bar = mean, tick = p95</span></h2><div class=body style=\"padding-left:14px\"><svg viewBox=\"0 0 {} {}\" width=\"100%\" role=img aria-label=\"benchmark means\">",
        w + lw + 90.0,
        rows.len() as f64 * h + 6.0
    );
    for (i, (name, mean, p95)) in rows.iter().enumerate() {
        let y = i as f64 * h + 4.0;
        let bw = mean / max * w;
        let tx = lw + p95 / max * w;
        let short_name: String = name.chars().take(28).collect();
        let _ = write!(
            o,
            "<text x=\"{}\" y=\"{}\" text-anchor=end>{}</text><rect x=\"{lw}\" y=\"{y}\" width=\"{bw:.1}\" height=\"14\" rx=\"3\" fill=\"#5b7cfa\"/><line x1=\"{tx:.1}\" x2=\"{tx:.1}\" y1=\"{}\" y2=\"{}\" stroke=\"currentColor\" stroke-width=\"2\"/><text x=\"{}\" y=\"{}\">{}</text>",
            lw - 8.0,
            y + 11.0,
            esc(&short_name),
            y - 1.0,
            y + 15.0,
            lw + w + 8.0,
            y + 11.0,
            fmt_ns(*mean)
        );
    }
    o + "</svg></div></section>"
}

pub fn render(r: &RunReport) -> String {
    let s = &r.summary;
    let mut o = format!(
        "<!doctype html><html lang=en><head><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\"><title>Muaz++ {} report</title><style>{STYLE}</style></head><body><main>",
        esc(&r.mode)
    );
    let _ = write!(
        o,
        "<h1>Muaz++ {} report</h1><div class=meta>{} · seed {} · mpp {} · {:.2}s</div>",
        esc(&r.mode),
        esc(&r.started),
        r.seed,
        esc(&r.mpp_version),
        r.duration_s
    );
    let _ = write!(
        o,
        "<div class=cards><div class=card><span>total</span><b>{}</b></div><div class=\"card ok\"><span>passed</span><b>{}</b></div><div class=\"card bad\"><span>failed</span><b>{}</b></div><div class=\"card bad\"><span>errors</span><b>{}</b></div><div class=\"card warn\"><span>skipped</span><b>{}</b></div></div>",
        s.total, s.passed, s.failed, s.errors, s.skipped
    );
    o.push_str("<div class=bar><button class=on data-f=all>all</button><button data-f=failed>failed</button><button data-f=error>errors</button><button data-f=passed>passed</button><button data-f=skipped>skipped</button></div>");
    o.push_str(&bench_chart(r));
    for f in &r.files {
        let bad = f.results.iter().filter(|t| matches!(t.status, Status::Failed | Status::Error)).count();
        let _ = write!(
            o,
            "<section><h2><span>{}</span><span class=tm>{} tests · {} bad · {}</span></h2>",
            esc(&f.file),
            f.results.len(),
            bad,
            fmt_ms(f.duration_ms)
        );
        for t in &f.results {
            o.push_str(&test_html(t));
        }
        if !f.data_files.is_empty() {
            o.push_str("<div class=body style=\"padding-left:14px\"><div class=lbl>data files (sha256)</div><table>");
            for (p, h) in &f.data_files {
                let _ = write!(o, "<tr><td>{}</td><td><code>{}</code></td></tr>", esc(p), esc(h));
            }
            o.push_str("</table></div>");
        }
        o.push_str("</section>");
    }
    let _ = writeln!(o, "<script>{SCRIPT}</script></main></body></html>");
    o
}
