use crate::vm::*;
use indexmap::IndexMap;
use serde_json::Value as J;

pub static FNS: &[Native] = &[
    Native { name: "parse", f: parse },
    Native { name: "dump", f: dump },
    Native { name: "load", f: load },
    Native { name: "save", f: save },
];

pub fn from_json(j: J) -> Value {
    match j {
        J::Null => Value::Nil,
        J::Bool(b) => Value::Bool(b),
        J::Number(n) => match n.as_i64() {
            Some(i) => Value::Int(i),
            None => Value::Float(n.as_f64().unwrap_or(f64::NAN)),
        },
        J::String(s) => Value::str(s),
        J::Array(a) => Value::list(a.into_iter().map(from_json).collect()),
        J::Object(o) => Value::map(o.into_iter().map(|(k, v)| (Key::Str(k.into()), from_json(v))).collect::<IndexMap<_, _>>()),
    }
}

pub fn to_json(v: &Value, depth: usize) -> Result<J, Flow> {
    if depth > 200 {
        return Err(value_err("value nests too deep for JSON (loop?)"));
    }
    Ok(match v {
        Value::Nil => J::Null,
        Value::Bool(b) => J::Bool(*b),
        Value::Int(n) => J::from(*n),
        Value::Float(x) => match serde_json::Number::from_f64(*x) {
            Some(n) => J::Number(n),
            None => return Err(value_err(format!("JSON cannot hold {}", fmt_float(*x)))),
        },
        Value::Str(s) => J::String(s.to_string()),
        Value::List(l) => J::Array(l.borrow().iter().map(|x| to_json(x, depth + 1)).collect::<Result<_, _>>()?),
        Value::Range(a, b) => J::Array((*a..*b).map(J::from).collect()),
        Value::Map(m) => {
            let mut o = serde_json::Map::new();
            for (k, x) in m.borrow().iter() {
                let key = match k {
                    Key::Str(s) => s.to_string(),
                    Key::Int(n) => n.to_string(),
                    Key::Bool(b) => b.to_string(),
                    Key::Nil => "null".into(),
                };
                o.insert(key, to_json(x, depth + 1)?);
            }
            J::Object(o)
        }
        Value::Instance(i) => {
            let mut o = serde_json::Map::new();
            for (k, x) in i.fields.borrow().iter() {
                o.insert(k.to_string(), to_json(x, depth + 1)?);
            }
            J::Object(o)
        }
        Value::Error(e) => serde_json::json!({"kind": &*e.kind, "message": &*e.message}),
        Value::Object(_) if v.object::<super::chart::Chart>().is_some() => {
            let c = v.object::<super::chart::Chart>().unwrap();
            serde_json::json!({"chart": c.title, "svg": c.svg})
        }
        // vecs become number arrays (NaN as null), tables become row lists
        Value::Object(o) if o.numbers().is_some() => {
            J::Array(o.numbers().unwrap().iter().map(|x| serde_json::Number::from_f64(*x).map_or(J::Null, J::Number)).collect())
        }
        Value::Object(o) if o.items().is_some() => {
            J::Array(o.items().unwrap().iter().map(|x| to_json(x, depth + 1)).collect::<Result<_, _>>()?)
        }
        other => return Err(type_err(format!("JSON cannot hold {}", other.kind_name()))),
    })
}

pub fn encode(v: &Value, indent: Option<usize>) -> Result<String, Flow> {
    let j = to_json(v, 0)?;
    Ok(match indent {
        None => j.to_string(),
        Some(n) => {
            let pad = vec![b' '; n];
            let mut buf = Vec::new();
            let fmt = serde_json::ser::PrettyFormatter::with_indent(&pad);
            let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
            serde::Serialize::serialize(&j, &mut ser).map_err(|e| value_err(e.to_string()))?;
            String::from_utf8(buf).unwrap_or_default()
        }
    })
}

pub fn decode(text: &str) -> R {
    serde_json::from_str::<J>(text).map(from_json).map_err(|e| err("JSONError", format!("bad JSON: {e}")))
}

fn indent(v: Option<Value>) -> Result<Option<usize>, Flow> {
    match opt(v) {
        Some(n) => Ok(Some(n.int("indent")?.clamp(0, 16) as usize)),
        None => Ok(None),
    }
}

fn parse(_: &mut Vm, a: Args) -> R {
    let [s] = a.bind(["text"])?;
    decode(need(s, "text")?.as_str("text")?)
}

fn dump(_: &mut Vm, a: Args) -> R {
    let [v, ind] = a.bind(["value", "indent"])?;
    Ok(Value::str(encode(&need(v, "value")?, indent(ind)?)?))
}

fn load(_: &mut Vm, a: Args) -> R {
    let [p] = a.bind(["path"])?;
    let p = need(p, "path")?;
    let text = super::io::read_text(p.as_str("path")?)?;
    decode(&text)
}

fn save(_: &mut Vm, a: Args) -> R {
    let [p, v, ind] = a.bind(["path", "value", "indent"])?;
    let p = need(p, "path")?.as_str("path")?.to_string();
    let ind = indent(ind)?.or(Some(2));
    let text = encode(&need(v, "value")?, ind)?;
    std::fs::write(&p, text + "\n").map_err(|e| err("IOError", format!("{p}: {e}")))?;
    Ok(Value::Nil)
}
