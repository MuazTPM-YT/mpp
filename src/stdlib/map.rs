use crate::vm::*;

pub const METHODS: &[&str] =
    &["len", "keys", "values", "items", "get", "has", "contains", "pop", "remove", "update", "copy", "clear", "is_empty"];

pub fn call(_vm: &mut Vm, m: &MapRef, name: &str, a: Args) -> R {
    match name {
        "len" | "keys" | "values" | "items" | "copy" | "clear" | "is_empty" => {
            a.bind([])?;
            let mut mm = m.borrow_mut();
            Ok(match name {
                "len" => Value::Int(mm.len() as i64),
                "keys" => Value::list(mm.keys().map(Key::value).collect()),
                "values" => Value::list(mm.values().cloned().collect()),
                "items" => Value::list(mm.iter().map(|(k, v)| Value::list(vec![k.value(), v.clone()])).collect()),
                "copy" => Value::map(mm.clone()),
                "is_empty" => Value::Bool(mm.is_empty()),
                _ => {
                    mm.clear();
                    Value::Nil
                }
            })
        }
        "get" => {
            let [k, d] = a.bind(["key", "default"])?;
            let k = Key::from(&need(k, "key")?)?;
            Ok(m.borrow().get(&k).cloned().or(d).unwrap_or(Value::Nil))
        }
        "has" | "contains" => {
            let [k] = a.bind(["key"])?;
            let k = need(k, "key")?;
            Ok(Value::Bool(Key::from(&k).is_ok_and(|k| m.borrow().contains_key(&k))))
        }
        "pop" | "remove" => {
            let [k, d] = a.bind(["key", "default"])?;
            let key = Key::from(&need(k, "key")?)?;
            match (m.borrow_mut().shift_remove(&key), d) {
                (Some(v), _) => Ok(v),
                (None, Some(d)) => Ok(d),
                (None, None) => Err(err("KeyError", format!("key {:?} not in map", key.value()))),
            }
        }
        "update" => {
            let [other] = a.bind(["other"])?;
            let other = need(other, "other")?;
            let Value::Map(o) = &other else {
                return Err(type_err("update() needs a map"));
            };
            let items: Vec<(Key, Value)> = o.borrow().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            m.borrow_mut().extend(items);
            Ok(Value::Nil)
        }
        _ => Err(err("AttributeError", format!("map has no method `{name}`"))),
    }
}
