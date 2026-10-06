use super::value::*;
use crate::compile::chunk::Op;
use std::cell::RefCell;
use std::rc::Rc;

fn overflow() -> Flow {
    err("OverflowError", "integer overflow (result does not fit in 64 bits)")
}

fn zero_div() -> Flow {
    err("ZeroDivisionError", "division by zero")
}

fn bad_op(sym: &str, a: &Value, b: &Value) -> Flow {
    type_err(format!("cannot do {} {sym} {}", a.kind_name(), b.kind_name()))
}

fn is_num(v: &Value) -> bool {
    matches!(v, Value::Int(_) | Value::Float(_) | Value::Bool(_))
}

fn repeat(v: &Value, n: i64) -> R {
    if n < 0 {
        return Err(value_err("cannot repeat a negative number of times"));
    }
    let n = n as usize;
    Ok(match v {
        Value::Str(s) => Value::str(s.repeat(n)),
        Value::List(l) => {
            let l = l.borrow();
            let mut out = Vec::with_capacity(l.len() * n);
            for _ in 0..n {
                out.extend(l.iter().cloned());
            }
            Value::list(out)
        }
        _ => unreachable!(),
    })
}

// python-style modulo: sign follows divisor
fn fmod(a: f64, b: f64) -> f64 {
    let r = a % b;
    if r != 0.0 && (r < 0.0) != (b < 0.0) { r + b } else { r }
}

pub fn binary(op: Op, a: &Value, b: &Value) -> R {
    use Value::*;
    if let Object(o) = a
        && let Some(r) = o.binary(op, b, false)
    {
        return r;
    }
    if let Object(o) = b
        && let Some(r) = o.binary(op, a, true)
    {
        return r;
    }
    match op {
        Op::Add => match (a, b) {
            (Int(x), Int(y)) => x.checked_add(*y).map(Int).ok_or_else(overflow),
            (Str(x), Str(y)) => {
                let mut s = String::with_capacity(x.len() + y.len());
                s.push_str(x);
                s.push_str(y);
                Ok(Value::str(s))
            }
            (List(x), List(y)) => {
                let mut v = x.borrow().clone();
                v.extend(y.borrow().iter().cloned());
                Ok(Value::list(v))
            }
            _ if is_num(a) && is_num(b) => Ok(Float(a.num("")? + b.num("")?)),
            (Str(_), _) | (_, Str(_)) => {
                Err(type_err(format!("cannot do {} + {}; use str(x) or an f-string", a.kind_name(), b.kind_name())))
            }
            _ => Err(bad_op("+", a, b)),
        },
        Op::Sub => match (a, b) {
            (Int(x), Int(y)) => x.checked_sub(*y).map(Int).ok_or_else(overflow),
            _ if is_num(a) && is_num(b) => Ok(Float(a.num("")? - b.num("")?)),
            _ => Err(bad_op("-", a, b)),
        },
        Op::Mul => match (a, b) {
            (Int(x), Int(y)) => x.checked_mul(*y).map(Int).ok_or_else(overflow),
            (Str(_) | List(_), Int(n)) => repeat(a, *n),
            (Int(n), Str(_) | List(_)) => repeat(b, *n),
            _ if is_num(a) && is_num(b) => Ok(Float(a.num("")? * b.num("")?)),
            _ => Err(bad_op("*", a, b)),
        },
        Op::Div => {
            if !(is_num(a) && is_num(b)) {
                return Err(bad_op("/", a, b));
            }
            let (x, y) = (a.num("")?, b.num("")?);
            if y == 0.0 {
                return Err(zero_div());
            }
            Ok(Float(x / y))
        }
        Op::IntDiv => match (a, b) {
            (Int(_), Int(0)) => Err(zero_div()),
            (Int(x), Int(y)) => {
                let q = x.checked_div(*y).ok_or_else(overflow)?;
                Ok(Int(if (x % y != 0) && ((*x < 0) != (*y < 0)) { q - 1 } else { q }))
            }
            _ if is_num(a) && is_num(b) => {
                let (x, y) = (a.num("")?, b.num("")?);
                if y == 0.0 {
                    return Err(zero_div());
                }
                Ok(Float((x / y).floor()))
            }
            _ => Err(bad_op("//", a, b)),
        },
        Op::Mod => match (a, b) {
            (Int(_), Int(0)) => Err(zero_div()),
            (Int(x), Int(y)) => Ok(Int(x.checked_rem_euclid(*y).map(|r| if *y < 0 && r != 0 { r + y } else { r }).ok_or_else(overflow)?)),
            _ if is_num(a) && is_num(b) => {
                let (x, y) = (a.num("")?, b.num("")?);
                if y == 0.0 {
                    return Err(zero_div());
                }
                Ok(Float(fmod(x, y)))
            }
            _ => Err(bad_op("%", a, b)),
        },
        Op::Pow => match (a, b) {
            (Int(x), Int(y)) if *y >= 0 => {
                let e = u32::try_from(*y).map_err(|_| overflow())?;
                x.checked_pow(e).map(Int).ok_or_else(overflow)
            }
            _ if is_num(a) && is_num(b) => Ok(Float(a.num("")?.powf(b.num("")?))),
            _ => Err(bad_op("**", a, b)),
        },
        Op::Eq => Ok(Bool(equal(a, b))),
        Op::Ne => Ok(Bool(!equal(a, b))),
        Op::Lt => Ok(Bool(compare(a, b)?.is_lt())),
        Op::Le => Ok(Bool(compare(a, b)?.is_le())),
        Op::Gt => Ok(Bool(compare(a, b)?.is_gt())),
        Op::Ge => Ok(Bool(compare(a, b)?.is_ge())),
        Op::In => contains(b, a).map(Bool),
        Op::NotIn => contains(b, a).map(|x| Bool(!x)),
        Op::Approx => approx(a, b, 1e-6, 1e-12).map(Bool),
        _ => unreachable!("not a binary op"),
    }
}

// `needle in hay`
pub fn contains(hay: &Value, needle: &Value) -> Result<bool, Flow> {
    match hay {
        Value::Object(o) if o.items().is_some() => Ok(o.items().unwrap().iter().any(|x| equal(x, needle))),
        Value::List(l) => Ok(l.borrow().iter().any(|x| equal(x, needle))),
        Value::Map(m) => Ok(Key::from(needle).is_ok_and(|k| m.borrow().contains_key(&k))),
        Value::Str(s) => Ok(s.contains(&**needle.as_str("left side of `in` a string")?)),
        Value::Range(a, b) => Ok(match needle {
            Value::Int(n) => a <= n && n < b,
            _ => false,
        }),
        Value::Instance(i) => Ok(needle.as_str("field name").is_ok_and(|s| i.fields.borrow().contains_key(&**s))),
        other => Err(type_err(format!("cannot use `in` on {}", other.kind_name()))),
    }
}

// python index: negative counts from end
pub fn norm_index(i: i64, len: usize) -> Option<usize> {
    let i = if i < 0 { i + len as i64 } else { i };
    (0..len as i64).contains(&i).then_some(i as usize)
}

fn out_of_range(i: i64, len: usize) -> Flow {
    err("IndexError", format!("index {i} out of range (length {len})"))
}

pub fn get_index(obj: &Value, idx: &Value) -> R {
    if let Value::Object(o) = obj
        && let Some(r) = o.index(idx)
    {
        return r;
    }
    match obj {
        Value::List(l) => {
            let l = l.borrow();
            let i = idx.int("list index")?;
            norm_index(i, l.len()).map(|i| l[i].clone()).ok_or_else(|| out_of_range(i, l.len()))
        }
        Value::Map(m) => {
            let k = Key::from(idx)?;
            m.borrow().get(&k).cloned().ok_or_else(|| err("KeyError", format!("key {:?} not in map", k.value())))
        }
        Value::Str(s) => {
            let i = idx.int("string index")?;
            if s.is_ascii() {
                return norm_index(i, s.len()).map(|i| Value::str(&s[i..i + 1])).ok_or_else(|| out_of_range(i, s.len()));
            }
            let n = s.chars().count();
            let j = norm_index(i, n).ok_or_else(|| out_of_range(i, n))?;
            Ok(Value::str(s.chars().nth(j).unwrap().to_string()))
        }
        Value::Range(a, b) => {
            let len = (b - a).max(0) as usize;
            let i = idx.int("range index")?;
            norm_index(i, len).map(|i| Value::Int(a + i as i64)).ok_or_else(|| out_of_range(i, len))
        }
        other => Err(type_err(format!("cannot index {}", other.kind_name()))),
    }
}

pub fn set_index(obj: &Value, idx: &Value, v: Value) -> Result<(), Flow> {
    match obj {
        Value::List(l) => {
            let mut l = l.borrow_mut();
            let i = idx.int("list index")?;
            let len = l.len();
            let j = norm_index(i, len).ok_or_else(|| out_of_range(i, len))?;
            l[j] = v;
            Ok(())
        }
        Value::Map(m) => {
            m.borrow_mut().insert(Key::from(idx)?, v);
            Ok(())
        }
        Value::Str(_) => Err(type_err("strings cannot change; build a new one")),
        other => Err(type_err(format!("cannot set items on {}", other.kind_name()))),
    }
}

// clamp python-style slice bounds
fn bounds(lo: &Value, hi: &Value, len: usize) -> Result<(usize, usize), Flow> {
    let fix = |v: &Value, dflt: usize| -> Result<usize, Flow> {
        match v {
            Value::Nil => Ok(dflt),
            v => {
                let i = v.int("slice bound")?;
                let i = if i < 0 { i + len as i64 } else { i };
                Ok(i.clamp(0, len as i64) as usize)
            }
        }
    };
    let (a, b) = (fix(lo, 0)?, fix(hi, len)?);
    Ok((a, b.max(a)))
}

pub fn get_slice(obj: &Value, lo: &Value, hi: &Value) -> R {
    if let Value::Object(o) = obj
        && let Some(r) = o.slice(lo, hi)
    {
        return r;
    }
    match obj {
        Value::List(l) => {
            let l = l.borrow();
            let (a, b) = bounds(lo, hi, l.len())?;
            Ok(Value::list(l[a..b].to_vec()))
        }
        Value::Str(s) => {
            if s.is_ascii() {
                let (a, b) = bounds(lo, hi, s.len())?;
                return Ok(Value::str(&s[a..b]));
            }
            let chars: Vec<char> = s.chars().collect();
            let (a, b) = bounds(lo, hi, chars.len())?;
            Ok(Value::str(chars[a..b].iter().collect::<String>()))
        }
        Value::Range(x, y) => {
            let len = (y - x).max(0) as usize;
            let (a, b) = bounds(lo, hi, len)?;
            Ok(Value::Range(x + a as i64, x + b as i64))
        }
        other => Err(type_err(format!("cannot slice {}", other.kind_name()))),
    }
}

pub fn iter_init(v: &Value) -> R {
    let st = match v {
        Value::Range(a, b) => IterState::Range(*a, *b),
        Value::List(l) => IterState::List(l.clone(), 0),
        Value::Str(s) => IterState::Items(s.chars().map(|c| Value::str(c.to_string())).collect(), 0),
        Value::Map(m) => IterState::Items(m.borrow().keys().map(Key::value).collect(), 0),
        Value::Iter(_) => return Ok(v.clone()),
        Value::Object(o) if o.items().is_some() => IterState::Items(o.items().unwrap(), 0),
        other => return Err(type_err(format!("cannot loop over {}", other.kind_name()))),
    };
    Ok(Value::Iter(Rc::new(RefCell::new(st))))
}

pub fn iter_next(it: &Value) -> Option<Value> {
    let Value::Iter(it) = it else { return None };
    match &mut *it.borrow_mut() {
        IterState::Range(a, b) => {
            if a < b {
                *a += 1;
                Some(Value::Int(*a - 1))
            } else {
                None
            }
        }
        IterState::List(l, i) => {
            let v = l.borrow().get(*i).cloned();
            *i += 1;
            v
        }
        IterState::Items(v, i) => {
            let x = v.get(*i).cloned();
            *i += 1;
            x
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(op: Op, a: Value, c: Value) -> R {
        binary(op, &a, &c)
    }

    #[test]
    fn math_rules() {
        assert!(matches!(b(Op::Div, Value::Int(1), Value::Int(2)), Ok(Value::Float(x)) if x == 0.5));
        assert!(matches!(b(Op::IntDiv, Value::Int(-7), Value::Int(2)), Ok(Value::Int(-4))));
        assert!(matches!(b(Op::Mod, Value::Int(-7), Value::Int(3)), Ok(Value::Int(2))));
        assert!(matches!(b(Op::Mod, Value::Int(7), Value::Int(-3)), Ok(Value::Int(-2))));
        assert!(matches!(b(Op::Pow, Value::Int(2), Value::Int(10)), Ok(Value::Int(1024))));
        assert!(b(Op::Add, Value::Int(i64::MAX), Value::Int(1)).is_err());
        assert!(b(Op::Div, Value::Float(1.0), Value::Int(0)).is_err());
        assert!(matches!(b(Op::Approx, Value::Float(0.1 + 0.2), Value::Float(0.3)), Ok(Value::Bool(true))));
    }

    #[test]
    fn indexing() {
        let l = Value::list(vec![Value::Int(1), Value::Int(2), Value::Int(3)]);
        assert!(matches!(get_index(&l, &Value::Int(-1)), Ok(Value::Int(3))));
        assert!(get_index(&l, &Value::Int(3)).is_err());
        let s = get_slice(&Value::str("héllo"), &Value::Int(1), &Value::Int(-1)).unwrap();
        assert!(matches!(s, Value::Str(x) if &*x == "éll"));
    }
}
