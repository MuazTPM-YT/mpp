// line charts as SVG; shown inline in HTML reports
use super::table::{Col, Table};
use crate::vm::*;
use std::fmt::Write as _;
use std::rc::Rc;

pub const METHODS: &[&str] = &["save"];

pub struct Chart {
    pub title: String,
    pub svg: String,
}

impl Object for Chart {
    fn type_name(&self) -> &'static str {
        "chart"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn display(&self) -> String {
        format!("<chart {}>", self.title)
    }
    fn get(&self, name: &str) -> Option<Value> {
        match name {
            "svg" => Some(Value::str(&self.svg)),
            "title" => Some(Value::str(&self.title)),
            _ => None,
        }
    }
    fn methods(&self) -> &'static [&'static str] {
        METHODS
    }
    fn call_method(&self, _vm: &mut Vm, _this: &Value, name: &str, a: Args) -> R {
        match name {
            "save" => {
                let [p] = a.bind(["path"])?;
                let p = need(p, "path")?.as_str("path")?.to_string();
                std::fs::write(&p, &self.svg).map_err(|e| err("IOError", format!("{p}: {e}")))?;
                Ok(Value::Nil)
            }
            _ => Err(err("AttributeError", format!("chart has no method `{name}`"))),
        }
    }
}

const COLORS: &[&str] = &["#5b7cfa", "#e8590c", "#2f9e44", "#ae3ec9", "#1098ad", "#f08c00", "#e64980"];

fn tick(x: f64) -> String {
    let a = x.abs();
    if a != 0.0 && !(1e-3..1e5).contains(&a) { format!("{x:.2e}") } else { format!("{}", (x * 1e4).round() / 1e4) }
}

// series: (name, points); missing y values are skipped
pub fn line_svg(title: &str, xlabel: &str, series: &[(String, Vec<(f64, f64)>)], log_y: bool) -> String {
    let (w, h, l, r, t, b) = (640.0, 300.0, 64.0, 16.0, 28.0, 36.0);
    let ty = |y: f64| if log_y { y.max(1e-300).log10() } else { y };
    let pts: Vec<(f64, f64)> = series.iter().flat_map(|s| s.1.iter().copied()).filter(|p| p.0.is_finite() && p.1.is_finite()).collect();
    let mut o = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w} {h}\" width=\"100%\" role=\"img\" aria-label=\"{}\" font-family=\"system-ui,sans-serif\" font-size=\"11\">",
        crate::runner::report::xml_escape(title)
    );
    let _ =
        write!(o, "<text x=\"{l}\" y=\"16\" font-weight=\"600\" fill=\"currentColor\">{}</text>", crate::runner::report::xml_escape(title));
    if pts.is_empty() {
        return o + "<text x=\"50%\" y=\"50%\" fill=\"currentColor\">no data</text></svg>";
    }
    let (mut x0, mut x1) = (f64::INFINITY, f64::NEG_INFINITY);
    let (mut y0, mut y1) = (f64::INFINITY, f64::NEG_INFINITY);
    for (x, y) in &pts {
        x0 = x0.min(*x);
        x1 = x1.max(*x);
        y0 = y0.min(ty(*y));
        y1 = y1.max(ty(*y));
    }
    if x1 == x0 {
        x1 = x0 + 1.0;
    }
    if y1 == y0 {
        y1 = y0 + 1.0;
    }
    let sx = |x: f64| l + (x - x0) / (x1 - x0) * (w - l - r);
    let sy = |y: f64| t + (1.0 - (ty(y) - y0) / (y1 - y0)) * (h - t - b);
    // grid + y ticks
    for i in 0..=4 {
        let v = y0 + (y1 - y0) * i as f64 / 4.0;
        let yy = t + (1.0 - i as f64 / 4.0) * (h - t - b);
        let lab = if log_y { tick(10f64.powf(v)) } else { tick(v) };
        let _ = write!(
            o,
            "<line x1=\"{l}\" x2=\"{}\" y1=\"{yy:.1}\" y2=\"{yy:.1}\" stroke=\"currentColor\" stroke-opacity=\"0.12\"/><text x=\"{}\" y=\"{:.1}\" text-anchor=\"end\" fill=\"currentColor\" fill-opacity=\"0.7\">{lab}</text>",
            w - r,
            l - 6.0,
            yy + 4.0
        );
    }
    for i in 0..=4 {
        let v = x0 + (x1 - x0) * i as f64 / 4.0;
        let _ = write!(
            o,
            "<text x=\"{:.1}\" y=\"{}\" text-anchor=\"middle\" fill=\"currentColor\" fill-opacity=\"0.7\">{}</text>",
            sx(v),
            h - b + 16.0,
            tick(v)
        );
    }
    let _ = write!(
        o,
        "<text x=\"{:.1}\" y=\"{}\" text-anchor=\"middle\" fill=\"currentColor\" fill-opacity=\"0.7\">{}</text>",
        (l + w - r) / 2.0,
        h - 4.0,
        crate::runner::report::xml_escape(xlabel)
    );
    for (i, (name, ps)) in series.iter().enumerate() {
        let c = COLORS[i % COLORS.len()];
        let mut d = String::new();
        let mut pen = false;
        for (x, y) in ps {
            if !(x.is_finite() && y.is_finite()) {
                pen = false;
                continue;
            }
            let _ = write!(d, "{}{:.1},{:.1}", if pen { "L" } else { "M" }, sx(*x), sy(*y));
            pen = true;
        }
        let _ = write!(o, "<path d=\"{d}\" fill=\"none\" stroke=\"{c}\" stroke-width=\"1.6\"/>");
        if ps.len() <= 40 {
            for (x, y) in ps.iter().filter(|p| p.0.is_finite() && p.1.is_finite()) {
                let _ = write!(o, "<circle cx=\"{:.1}\" cy=\"{:.1}\" r=\"2.5\" fill=\"{c}\"/>", sx(*x), sy(*y));
            }
        }
        let lx = l + 8.0 + i as f64 * 120.0;
        let _ = write!(
            o,
            "<rect x=\"{lx}\" y=\"{}\" width=\"10\" height=\"3\" fill=\"{c}\"/><text x=\"{}\" y=\"{}\" fill=\"currentColor\">{}</text>",
            t + 2.0,
            lx + 14.0,
            t + 6.0,
            crate::runner::report::xml_escape(name)
        );
    }
    o + "</svg>"
}

pub fn chart_value(title: &str, svg: String) -> Value {
    Value::Object(Rc::new(Chart { title: title.to_string(), svg }))
}

// plot(table, x, ys, title=, log=false)
pub fn plot(_: &mut Vm, a: Args) -> R {
    let [data, x, ys, title, log] = a.bind(["data", "x", "y", "title", "log"])?;
    let data = need(data, "data")?;
    let t = match data.object::<Table>() {
        Some(t) => t,
        None => match data.object::<super::llm::train::TrainLog>() {
            Some(l) => return l.plot_value(x, ys, title, log),
            None => return Err(type_err("plot() needs a table or a training log")),
        },
    };
    let x = need(x, "x")?.as_str("x")?.to_string();
    let xs = match t.col(&x)? {
        Col::Num(v, _) => v.to_vec(),
        Col::Text(_) => return Err(type_err("x column must be numbers")),
    };
    let ynames: Vec<String> = match need(ys, "y")? {
        Value::Str(s) => vec![s.to_string()],
        other => super::to_vec(&other, "y")?.iter().map(|v| Ok(v.as_str("y")?.to_string())).collect::<Result<_, Flow>>()?,
    };
    let mut series = Vec::new();
    for y in &ynames {
        let Col::Num(v, _) = t.col(y)? else { return Err(type_err(format!("column `{y}` must be numbers"))) };
        series.push((y.clone(), xs.iter().zip(v.iter()).map(|(a, b)| (*a, *b)).collect()));
    }
    let title = opt(title).map_or(Ok(ynames.join(", ")), |v| Ok::<_, Flow>(v.as_str("title")?.to_string()))?;
    let svg = line_svg(&title, &x, &series, log.is_some_and(|l| l.truthy()));
    Ok(chart_value(&title, svg))
}
