use super::stats::desc;
use super::vec::{NumVec, vec_value};
use crate::vm::*;
use indexmap::IndexMap;
use std::collections::HashMap;
use std::rc::Rc;

// one column: numbers (NaN = missing, int flag for whole numbers) or text
#[derive(Clone)]
pub enum Col {
    Num(Rc<Vec<f64>>, bool),
    Text(Rc<Vec<Option<Rc<str>>>>),
}

impl Col {
    pub fn len(&self) -> usize {
        match self {
            Col::Num(v, _) => v.len(),
            Col::Text(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get(&self, i: usize) -> Value {
        match self {
            Col::Num(v, int) => {
                let x = v[i];
                if x.is_nan() {
                    Value::Nil
                } else if *int {
                    Value::Int(x as i64)
                } else {
                    Value::Float(x)
                }
            }
            Col::Text(v) => v[i].clone().map_or(Value::Nil, Value::Str),
        }
    }

    fn take(&self, idx: &[usize]) -> Col {
        match self {
            Col::Num(v, int) => Col::Num(Rc::new(idx.iter().map(|&i| v[i]).collect()), *int),
            Col::Text(v) => Col::Text(Rc::new(idx.iter().map(|&i| v[i].clone()).collect())),
        }
    }

    // grouping/join key for row i
    fn key(&self, i: usize) -> Key {
        match self {
            Col::Num(v, _) => {
                let x = v[i];
                if x.is_nan() {
                    Key::Nil
                } else if x.fract() == 0.0 && x.abs() < 9e15 {
                    Key::Int(x as i64)
                } else {
                    Key::Str(fmt_float(x).into())
                }
            }
            Col::Text(v) => v[i].clone().map_or(Key::Nil, Key::Str),
        }
    }

    fn missing(&self, i: usize) -> bool {
        match self {
            Col::Num(v, _) => v[i].is_nan(),
            Col::Text(v) => v[i].is_none(),
        }
    }

    // column as a script value: vec for numbers, list for text
    pub fn to_value(&self) -> Value {
        match self {
            Col::Num(v, _) => vec_value(v.to_vec()),
            Col::Text(v) => Value::list(v.iter().map(|s| s.clone().map_or(Value::Nil, Value::Str)).collect()),
        }
    }

    fn numbers(&self, name: &str) -> Result<Vec<f64>, Flow> {
        match self {
            Col::Num(v, _) => Ok(v.iter().copied().filter(|x| !x.is_nan()).collect()),
            Col::Text(_) => Err(type_err(format!("column `{name}` is text, not numbers"))),
        }
    }
}

// build a column from script values
pub fn col_from_values(vals: &[Value]) -> Result<Col, Flow> {
    let numeric = vals.iter().all(|v| matches!(v, Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::Nil));
    if numeric {
        let int = vals.iter().all(|v| matches!(v, Value::Int(_) | Value::Bool(_) | Value::Nil));
        let data = vals.iter().map(|v| if matches!(v, Value::Nil) { f64::NAN } else { v.num("").unwrap_or(f64::NAN) }).collect();
        return Ok(Col::Num(Rc::new(data), int));
    }
    let mut out = Vec::with_capacity(vals.len());
    for v in vals {
        out.push(match v {
            Value::Nil => None,
            Value::Str(s) => Some(s.clone()),
            Value::Int(n) => Some(n.to_string().into()),
            Value::Float(x) => Some(fmt_float(*x).into()),
            Value::Bool(b) => Some(b.to_string().into()),
            other => return Err(type_err(format!("table cells must be numbers, strings, bools or nil, not {}", other.kind_name()))),
        });
    }
    Ok(Col::Text(Rc::new(out)))
}

pub struct Table {
    pub names: Vec<Rc<str>>,
    pub cols: Vec<Col>,
    pub nrows: usize,
}

pub fn table_value(t: Table) -> Value {
    Value::Object(Rc::new(t))
}

impl Table {
    pub fn new(names: Vec<Rc<str>>, cols: Vec<Col>) -> Result<Table, Flow> {
        let nrows = cols.first().map_or(0, Col::len);
        if cols.iter().any(|c| c.len() != nrows) {
            return Err(value_err("table columns must all have the same length"));
        }
        for (i, n) in names.iter().enumerate() {
            if names[..i].contains(n) {
                return Err(value_err(format!("column `{n}` appears twice")));
            }
        }
        Ok(Table { names, cols, nrows })
    }

    pub fn col_index(&self, name: &str) -> Result<usize, Flow> {
        self.names
            .iter()
            .position(|n| &**n == name)
            .ok_or_else(|| err("KeyError", format!("no column `{name}` (columns: {})", self.names.join(", "))))
    }

    pub fn col(&self, name: &str) -> Result<&Col, Flow> {
        Ok(&self.cols[self.col_index(name)?])
    }

    // numbers of one column, missing dropped
    pub fn nums(&self, name: &str) -> Result<Vec<f64>, Flow> {
        self.col(name)?.numbers(name)
    }

    pub fn row(&self, i: usize) -> Value {
        Value::map(self.names.iter().zip(&self.cols).map(|(n, c)| (Key::Str(n.clone()), c.get(i))).collect())
    }

    fn take(&self, idx: &[usize]) -> Table {
        Table { names: self.names.clone(), cols: self.cols.iter().map(|c| c.take(idx)).collect(), nrows: idx.len() }
    }

    fn keys(&self, by: &[usize], i: usize) -> Vec<Key> {
        by.iter().map(|&c| self.cols[c].key(i)).collect()
    }

    // rows grouped by key columns, first-seen order
    fn group_rows(&self, by: &[usize]) -> IndexMap<Vec<Key>, Vec<usize>> {
        let mut g: IndexMap<Vec<Key>, Vec<usize>> = IndexMap::new();
        for i in 0..self.nrows {
            g.entry(self.keys(by, i)).or_default().push(i);
        }
        g
    }
}

pub const METHODS: &[&str] = &[
    "col",
    "row",
    "rows",
    "head",
    "tail",
    "select",
    "drop",
    "rename",
    "filter",
    "where",
    "sort",
    "with_col",
    "group_by",
    "groups",
    "join",
    "describe",
    "value_counts",
    "unique",
    "dropna",
    "sample",
    "shuffle",
    "concat",
    "save_csv",
    "len",
];

fn names_arg(v: Value) -> Result<Vec<String>, Flow> {
    match &v {
        Value::Str(s) => Ok(vec![s.to_string()]),
        other => super::to_vec(other, "column names")?.iter().map(|x| Ok(x.as_str("column name")?.to_string())).collect(),
    }
}

fn spread_names(a: &Args) -> Result<Vec<String>, Flow> {
    a.no_kw()?;
    let mut out = Vec::new();
    for v in &a.pos {
        out.extend(names_arg(v.clone())?);
    }
    Ok(out)
}

impl Object for Table {
    fn type_name(&self) -> &'static str {
        "table"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn len(&self) -> Option<usize> {
        Some(self.nrows)
    }
    fn items(&self) -> Option<Vec<Value>> {
        Some((0..self.nrows).map(|i| self.row(i)).collect())
    }
    fn methods(&self) -> &'static [&'static str] {
        METHODS
    }
    fn get(&self, name: &str) -> Option<Value> {
        match name {
            "columns" => Some(Value::list(self.names.iter().map(|n| Value::Str(n.clone())).collect())),
            "nrows" => Some(Value::Int(self.nrows as i64)),
            "ncols" => Some(Value::Int(self.names.len() as i64)),
            _ => self.col(name).ok().map(Col::to_value),
        }
    }
    fn index(&self, idx: &Value) -> Option<R> {
        Some(match idx {
            Value::Str(s) => self.col(s).map(Col::to_value),
            Value::Int(i) => crate::vm::ops::norm_index(*i, self.nrows)
                .map(|j| self.row(j))
                .ok_or_else(|| err("IndexError", format!("row {i} out of range ({} rows)", self.nrows))),
            Value::Object(_) => match idx.object::<NumVec>() {
                Some(m) if m.0.len() == self.nrows => {
                    let keep: Vec<usize> = (0..self.nrows).filter(|&i| m.0[i] != 0.0 && !m.0[i].is_nan()).collect();
                    Ok(table_value(self.take(&keep)))
                }
                _ => Err(type_err("a table index must be a column name, a row number, or a mask vec of the same length")),
            },
            other => Err(type_err(format!("cannot index a table with {}", other.kind_name()))),
        })
    }
    fn slice(&self, lo: &Value, hi: &Value) -> Option<R> {
        let n = self.nrows as i64;
        let fix = |v: &Value, d: i64| -> Result<usize, Flow> {
            match v {
                Value::Nil => Ok(d as usize),
                v => {
                    let i = v.int("slice bound")?;
                    Ok((if i < 0 { i + n } else { i }).clamp(0, n) as usize)
                }
            }
        };
        Some((|| {
            let (a, b) = (fix(lo, 0)?, fix(hi, n)?);
            let idx: Vec<usize> = (a..b.max(a)).collect();
            Ok(table_value(self.take(&idx)))
        })())
    }
    fn display(&self) -> String {
        render(self, 10)
    }
    fn call_method(&self, vm: &mut Vm, _this: &Value, name: &str, a: Args) -> R {
        match name {
            "len" => {
                a.bind([])?;
                Ok(Value::Int(self.nrows as i64))
            }
            "col" => {
                let [c] = a.bind(["name"])?;
                Ok(self.col(need(c, "name")?.as_str("name")?)?.to_value())
            }
            "row" => {
                let [i] = a.bind(["i"])?;
                let i = need(i, "i")?.int("i")?;
                let j = crate::vm::ops::norm_index(i, self.nrows).ok_or_else(|| err("IndexError", format!("row {i} out of range")))?;
                Ok(self.row(j))
            }
            "rows" => {
                a.bind([])?;
                Ok(Value::list(self.items().unwrap()))
            }
            "head" | "tail" => {
                let [k] = a.bind(["n"])?;
                let k = k.map_or(Ok(5), |v| v.int("n"))?.max(0) as usize;
                let k = k.min(self.nrows);
                let idx: Vec<usize> = if name == "head" { (0..k).collect() } else { (self.nrows - k..self.nrows).collect() };
                Ok(table_value(self.take(&idx)))
            }
            "select" | "drop" => {
                let want = spread_names(&a)?;
                for w in &want {
                    self.col_index(w)?;
                }
                let keep: Vec<usize> = if name == "select" {
                    want.iter().map(|w| self.col_index(w).unwrap()).collect()
                } else {
                    (0..self.names.len()).filter(|&i| !want.iter().any(|w| **w == *self.names[i])).collect()
                };
                Ok(table_value(Table {
                    names: keep.iter().map(|&i| self.names[i].clone()).collect(),
                    cols: keep.iter().map(|&i| self.cols[i].clone()).collect(),
                    nrows: self.nrows,
                }))
            }
            "rename" => {
                let [m] = a.bind(["names"])?;
                let m = need(m, "names")?;
                let Value::Map(m) = &m else { return Err(type_err("rename() takes a map {\"old\": \"new\"}")) };
                let mut names = self.names.clone();
                for (k, v) in m.borrow().iter() {
                    let old = k.value();
                    let i = self.col_index(old.as_str("old name")?)?;
                    names[i] = v.as_str("new name")?.clone();
                }
                Ok(table_value(Table::new(names, self.cols.clone())?))
            }
            "filter" => {
                let [f] = a.bind(["fn"])?;
                let f = need(f, "fn")?;
                let mut keep = Vec::new();
                for i in 0..self.nrows {
                    if vm.call(&f, &[self.row(i)])?.truthy() {
                        keep.push(i);
                    }
                }
                Ok(table_value(self.take(&keep)))
            }
            "where" => {
                let [c, op, v] = a.bind(["col", "op", "value"])?;
                let c = need(c, "col")?;
                let col = self.col(c.as_str("col")?)?;
                let op = need(op, "op")?.as_str("op")?.to_string();
                let v = need(v, "value")?;
                let bin = match op.as_str() {
                    "==" => crate::compile::chunk::Op::Eq,
                    "!=" => crate::compile::chunk::Op::Ne,
                    "<" => crate::compile::chunk::Op::Lt,
                    "<=" => crate::compile::chunk::Op::Le,
                    ">" => crate::compile::chunk::Op::Gt,
                    ">=" => crate::compile::chunk::Op::Ge,
                    "in" => crate::compile::chunk::Op::In,
                    "not in" => crate::compile::chunk::Op::NotIn,
                    other => return Err(value_err(format!("unknown op {other:?} (use == != < <= > >= in, not in)"))),
                };
                let mut keep = Vec::new();
                for i in 0..self.nrows {
                    let cell = col.get(i);
                    if matches!(cell, Value::Nil) && !matches!(bin, crate::compile::chunk::Op::Eq | crate::compile::chunk::Op::Ne) {
                        continue;
                    }
                    if crate::vm::ops::binary(bin, &cell, &v)?.truthy() {
                        keep.push(i);
                    }
                }
                Ok(table_value(self.take(&keep)))
            }
            "sort" => {
                let [by, rev] = a.bind(["by", "reverse"])?;
                let by: Vec<usize> = names_arg(need(by, "by")?)?.iter().map(|n| self.col_index(n)).collect::<Result<_, _>>()?;
                let rev = rev.is_some_and(|r| r.truthy());
                let mut idx: Vec<usize> = (0..self.nrows).collect();
                let cmp_cell = |c: &Col, i: usize, j: usize| -> std::cmp::Ordering {
                    match c {
                        // missing goes last either way
                        Col::Num(v, _) => match (v[i].is_nan(), v[j].is_nan()) {
                            (true, true) => std::cmp::Ordering::Equal,
                            (true, false) => std::cmp::Ordering::Greater,
                            (false, true) => std::cmp::Ordering::Less,
                            _ => {
                                let o = v[i].total_cmp(&v[j]);
                                if rev { o.reverse() } else { o }
                            }
                        },
                        Col::Text(v) => match (&v[i], &v[j]) {
                            (None, None) => std::cmp::Ordering::Equal,
                            (None, _) => std::cmp::Ordering::Greater,
                            (_, None) => std::cmp::Ordering::Less,
                            (Some(a), Some(b)) => {
                                let o = a.cmp(b);
                                if rev { o.reverse() } else { o }
                            }
                        },
                    }
                };
                idx.sort_by(|&i, &j| {
                    by.iter().map(|&c| cmp_cell(&self.cols[c], i, j)).find(|o| o.is_ne()).unwrap_or(std::cmp::Ordering::Equal)
                });
                Ok(table_value(self.take(&idx)))
            }
            "with_col" => {
                let [n, v] = a.bind(["name", "values"])?;
                let n = need(n, "name")?.as_str("name")?.clone();
                let v = need(v, "values")?;
                let vals: Vec<Value> = if v.is_callable() {
                    let mut out = Vec::with_capacity(self.nrows);
                    for i in 0..self.nrows {
                        out.push(vm.call(&v, &[self.row(i)])?);
                    }
                    out
                } else if let Value::Int(_) | Value::Float(_) | Value::Str(_) | Value::Bool(_) | Value::Nil = v {
                    vec![v; self.nrows]
                } else {
                    super::to_vec(&v, "values")?
                };
                if vals.len() != self.nrows {
                    return Err(value_err(format!("new column has {} values, table has {} rows", vals.len(), self.nrows)));
                }
                let col = col_from_values(&vals)?;
                let (mut names, mut cols) = (self.names.clone(), self.cols.clone());
                match names.iter().position(|x| *x == n) {
                    Some(i) => cols[i] = col,
                    None => {
                        names.push(n);
                        cols.push(col);
                    }
                }
                Ok(table_value(Table { names, cols, nrows: self.nrows }))
            }
            "group_by" => {
                let [by, aggs] = a.bind(["by", "aggs"])?;
                let by_names = names_arg(need(by, "by")?)?;
                let by: Vec<usize> = by_names.iter().map(|n| self.col_index(n)).collect::<Result<_, _>>()?;
                let specs = match aggs {
                    Some(Value::Map(m)) => m
                        .borrow()
                        .iter()
                        .map(|(k, v)| Ok((k.value().as_str("agg name")?.clone(), v.as_str("agg spec")?.to_string())))
                        .collect::<Result<Vec<_>, Flow>>()?,
                    None => vec![("count".into(), "count()".to_string())],
                    _ => return Err(type_err("group_by aggs must be a map like {\"rate\": \"mean(converted)\"}")),
                };
                let groups = self.group_rows(&by);
                let mut names: Vec<Rc<str>> = by_names.iter().map(|n| Rc::from(n.as_str())).collect();
                let mut cols: Vec<Vec<Value>> = vec![Vec::new(); by.len()];
                for (keys, _) in &groups {
                    for (j, k) in keys.iter().enumerate() {
                        cols[j].push(k.value());
                    }
                }
                for (out_name, spec) in &specs {
                    let (f, c) = parse_agg(spec)?;
                    let mut vals = Vec::with_capacity(groups.len());
                    for rows in groups.values() {
                        vals.push(self.agg(&f, c.as_deref(), rows)?);
                    }
                    names.push(out_name.clone());
                    cols.push(vals);
                }
                let cols = cols.iter().map(|v| col_from_values(v)).collect::<Result<_, _>>()?;
                Ok(table_value(Table::new(names, cols)?))
            }
            "groups" => {
                let [by] = a.bind(["by"])?;
                let by: Vec<usize> = names_arg(need(by, "by")?)?.iter().map(|n| self.col_index(n)).collect::<Result<_, _>>()?;
                let mut out = IndexMap::new();
                for (keys, rows) in self.group_rows(&by) {
                    let k = if keys.len() == 1 {
                        keys[0].clone()
                    } else {
                        Key::Str(
                            keys.iter().map(|k| vm.display(&k.value(), false).unwrap_or_default()).collect::<Vec<_>>().join("|").into(),
                        )
                    };
                    out.insert(k, table_value(self.take(&rows)));
                }
                Ok(Value::map(out))
            }
            "join" => {
                let [other, on, how] = a.bind(["other", "on", "how"])?;
                let other = need(other, "other")?;
                let o = other.object::<Table>().ok_or_else(|| type_err("join() needs another table"))?;
                let on = names_arg(need(on, "on")?)?;
                let how = how.map_or(Ok("inner".to_string()), |h| Ok::<_, Flow>(h.as_str("how")?.to_string()))?;
                if how != "inner" && how != "left" {
                    return Err(value_err("how must be \"inner\" or \"left\""));
                }
                self.join(o, &on, how == "left").map(table_value)
            }
            "describe" => {
                a.bind([])?;
                let mut rows: Vec<Value> = Vec::new();
                for (n, c) in self.names.iter().zip(&self.cols) {
                    if let Col::Num(v, _) = c {
                        let d = super::vec::describe(v);
                        if let Value::Map(m) = &d {
                            m.borrow_mut().shift_insert(0, Key::Str("column".into()), Value::Str(n.clone()));
                        }
                        rows.push(d);
                    }
                }
                table_from_rows(&rows).map(table_value)
            }
            "value_counts" => {
                let [c] = a.bind(["col"])?;
                let c = self.col(need(c, "col")?.as_str("col")?)?;
                let mut counts: IndexMap<Key, i64> = IndexMap::new();
                for i in 0..self.nrows {
                    *counts.entry(c.key(i)).or_default() += 1;
                }
                counts.sort_by(|_, a, _, b| b.cmp(a));
                Ok(Value::map(counts.into_iter().map(|(k, n)| (k, Value::Int(n))).collect()))
            }
            "unique" => {
                let [c] = a.bind(["col"])?;
                let c = self.col(need(c, "col")?.as_str("col")?)?;
                let mut seen = IndexMap::new();
                for i in 0..self.nrows {
                    seen.entry(c.key(i)).or_insert_with(|| c.get(i));
                }
                Ok(Value::list(seen.into_values().collect()))
            }
            "dropna" => {
                let names = spread_names(&a)?;
                let cols: Vec<usize> = if names.is_empty() {
                    (0..self.cols.len()).collect()
                } else {
                    names.iter().map(|n| self.col_index(n)).collect::<Result<_, _>>()?
                };
                let keep: Vec<usize> = (0..self.nrows).filter(|&i| cols.iter().all(|&c| !self.cols[c].missing(i))).collect();
                Ok(table_value(self.take(&keep)))
            }
            "sample" | "shuffle" => {
                let (k, replace) = if name == "sample" {
                    let [k, replace] = a.bind(["n", "replace"])?;
                    (need(k, "n")?.int("n")?.max(0) as usize, replace.is_some_and(|r| r.truthy()))
                } else {
                    a.bind([])?;
                    (self.nrows, false)
                };
                let idx: Vec<usize> = if replace {
                    if self.nrows == 0 {
                        return Err(value_err("sample from empty table"));
                    }
                    (0..k).map(|_| vm.rng.below(self.nrows as u64) as usize).collect()
                } else {
                    if k > self.nrows {
                        return Err(value_err(format!("sample size {k} bigger than table ({} rows)", self.nrows)));
                    }
                    let mut v: Vec<usize> = (0..self.nrows).collect();
                    vm.rng.shuffle(&mut v);
                    v.truncate(k);
                    v
                };
                Ok(table_value(self.take(&idx)))
            }
            "concat" => {
                let [other] = a.bind(["other"])?;
                let other = need(other, "other")?;
                let o = other.object::<Table>().ok_or_else(|| type_err("concat() needs another table"))?;
                if o.names != self.names {
                    return Err(value_err("concat() needs the same columns in the same order"));
                }
                let mut cols = Vec::new();
                for (x, y) in self.cols.iter().zip(&o.cols) {
                    let vals: Vec<Value> = (0..x.len()).map(|i| x.get(i)).chain((0..y.len()).map(|i| y.get(i))).collect();
                    cols.push(col_from_values(&vals)?);
                }
                Ok(table_value(Table::new(self.names.clone(), cols)?))
            }
            "save_csv" => {
                let [p] = a.bind(["path"])?;
                let p = need(p, "path")?.as_str("path")?.to_string();
                self.save_csv(&p)?;
                Ok(Value::Nil)
            }
            _ => Err(err("AttributeError", format!("table has no method `{name}`"))),
        }
    }
}

// "mean(col)" -> ("mean", Some("col")); "count()" -> ("count", None)
fn parse_agg(spec: &str) -> Result<(String, Option<String>), Flow> {
    let bad = || value_err(format!("bad aggregate {spec:?}; write like \"mean(price)\" or \"count()\""));
    let (f, rest) = spec.split_once('(').ok_or_else(bad)?;
    let inner = rest.strip_suffix(')').ok_or_else(bad)?.trim();
    Ok((f.trim().to_string(), (!inner.is_empty()).then(|| inner.to_string())))
}

impl Table {
    fn agg(&self, f: &str, col: Option<&str>, rows: &[usize]) -> R {
        if f == "count" && col.is_none() {
            return Ok(Value::Int(rows.len() as i64));
        }
        let name = col.ok_or_else(|| value_err(format!("{f}() needs a column, like {f}(price)")))?;
        let c = self.col(name)?;
        let present: Vec<usize> = rows.iter().copied().filter(|&i| !c.missing(i)).collect();
        match f {
            "count" => return Ok(Value::Int(present.len() as i64)),
            "nunique" => {
                let mut s: Vec<Key> = present.iter().map(|&i| c.key(i)).collect();
                s.dedup();
                let set: std::collections::HashSet<Key> = s.into_iter().collect();
                return Ok(Value::Int(set.len() as i64));
            }
            "first" => return Ok(present.first().map_or(Value::Nil, |&i| c.get(i))),
            "last" => return Ok(present.last().map_or(Value::Nil, |&i| c.get(i))),
            _ => {}
        }
        let Col::Num(v, int) = c else { return Err(type_err(format!("{f}({name}) needs a number column"))) };
        let xs: Vec<f64> = present.iter().map(|&i| v[i]).collect();
        let r = match f {
            "sum" => {
                let s = desc::sum(&xs);
                if *int {
                    return Ok(Value::Int(s as i64));
                }
                s
            }
            "mean" => desc::mean(&xs),
            "median" => desc::median(&xs),
            "min" => desc::min(&xs),
            "max" => desc::max(&xs),
            "std" => desc::std(&xs, 1.0),
            "var" => desc::var(&xs, 1.0),
            other => {
                return Err(value_err(format!(
                    "unknown aggregate `{other}` (use count, sum, mean, median, min, max, std, var, nunique, first, last)"
                )));
            }
        };
        Ok(if r.is_nan() { Value::Nil } else { Value::Float(r) })
    }

    fn join(&self, o: &Table, on: &[String], left: bool) -> Result<Table, Flow> {
        let lk: Vec<usize> = on.iter().map(|n| self.col_index(n)).collect::<Result<_, _>>()?;
        let rk: Vec<usize> = on.iter().map(|n| o.col_index(n)).collect::<Result<_, _>>()?;
        let mut index: HashMap<Vec<Key>, Vec<usize>> = HashMap::new();
        for j in 0..o.nrows {
            index.entry(o.keys(&rk, j)).or_default().push(j);
        }
        let (mut li, mut ri): (Vec<usize>, Vec<Option<usize>>) = (Vec::new(), Vec::new());
        for i in 0..self.nrows {
            match index.get(&self.keys(&lk, i)) {
                Some(js) => {
                    for &j in js {
                        li.push(i);
                        ri.push(Some(j));
                    }
                }
                None if left => {
                    li.push(i);
                    ri.push(None);
                }
                None => {}
            }
        }
        let mut names = self.names.clone();
        let mut cols: Vec<Col> = self.cols.iter().map(|c| c.take(&li)).collect();
        for (c, n) in o.cols.iter().zip(&o.names) {
            if on.iter().any(|k| **k == **n) {
                continue;
            }
            let vals: Vec<Value> = ri.iter().map(|j| j.map_or(Value::Nil, |j| c.get(j))).collect();
            let name: Rc<str> = if names.contains(n) { format!("{n}_right").into() } else { n.clone() };
            names.push(name);
            cols.push(match c {
                Col::Num(_, int) => match col_from_values(&vals)? {
                    Col::Num(v, _) => Col::Num(v, *int),
                    t => t,
                },
                _ => col_from_values(&vals)?,
            });
        }
        Table::new(names, cols)
    }

    fn save_csv(&self, path: &str) -> Result<(), Flow> {
        let io = |e: csv::Error| err("IOError", format!("{path}: {e}"));
        let mut w = csv::Writer::from_path(path).map_err(io)?;
        w.write_record(self.names.iter().map(|n| n.as_bytes())).map_err(io)?;
        for i in 0..self.nrows {
            let rec: Vec<String> = self
                .cols
                .iter()
                .map(|c| match c.get(i) {
                    Value::Nil => String::new(),
                    Value::Str(s) => s.to_string(),
                    Value::Int(n) => n.to_string(),
                    Value::Float(x) => format!("{x}"),
                    _ => String::new(),
                })
                .collect();
            w.write_record(&rec).map_err(io)?;
        }
        w.flush().map_err(|e| err("IOError", format!("{path}: {e}")))
    }
}

// pretty text table, first `max` rows
fn render(t: &Table, max: usize) -> String {
    let shown = t.nrows.min(max);
    let cell = |c: &Col, i: usize| match c.get(i) {
        Value::Nil => "·".to_string(),
        Value::Float(x) => {
            let s = format!("{:.4}", x);
            let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
            if s.is_empty() || s == "-" { "0".into() } else { s }
        }
        Value::Int(n) => n.to_string(),
        Value::Str(s) if s.chars().count() > 48 => s.chars().take(47).collect::<String>() + "…",
        Value::Str(s) => s.to_string(),
        _ => "?".into(),
    };
    let grid: Vec<Vec<String>> = t.cols.iter().map(|c| (0..shown).map(|i| cell(c, i)).collect()).collect();
    let widths: Vec<usize> =
        t.names.iter().zip(&grid).map(|(n, g)| g.iter().map(|s| s.chars().count()).chain([n.chars().count()]).max().unwrap_or(1)).collect();
    let mut o = String::new();
    let line = |o: &mut String, parts: Vec<String>| {
        o.push_str(parts.join("  ").trim_end());
        o.push('\n');
    };
    line(&mut o, t.names.iter().zip(&widths).map(|(n, w)| format!("{n:<w$}")).collect());
    line(&mut o, widths.iter().map(|w| "─".repeat(*w)).collect());
    for i in 0..shown {
        line(
            &mut o,
            t.cols
                .iter()
                .zip(&grid)
                .zip(&widths)
                .map(|((c, g), w)| if matches!(c, Col::Num(..)) { format!("{:>w$}", g[i]) } else { format!("{:<w$}", g[i]) })
                .collect(),
        );
    }
    if t.nrows > shown {
        o.push_str(&format!("… {} more rows\n", t.nrows - shown));
    }
    o.push_str(&format!("[{} rows × {} columns]", t.nrows, t.names.len()));
    o
}

// list of maps -> table (column order = first-seen keys)
pub fn table_from_rows(rows: &[Value]) -> Result<Table, Flow> {
    let mut names: IndexMap<Rc<str>, ()> = IndexMap::new();
    for r in rows {
        let Value::Map(m) = r else { return Err(type_err(format!("each row must be a map, got {}", r.kind_name()))) };
        for k in m.borrow().keys() {
            let Key::Str(s) = k else { return Err(type_err("row keys must be strings")) };
            names.entry(s.clone()).or_insert(());
        }
    }
    let mut cols = Vec::new();
    for n in names.keys() {
        let vals: Vec<Value> = rows
            .iter()
            .map(|r| if let Value::Map(m) = r { m.borrow().get(&Key::Str(n.clone())).cloned().unwrap_or(Value::Nil) } else { Value::Nil })
            .collect();
        cols.push(col_from_values(&vals)?);
    }
    Table::new(names.into_keys().collect(), cols)
}

// table(x): x is {col: values} or [row maps]
pub fn table_fn(_: &mut Vm, a: Args) -> R {
    let [x] = a.bind(["data"])?;
    match x {
        None => Ok(table_value(Table { names: vec![], cols: vec![], nrows: 0 })),
        Some(Value::Map(m)) => {
            let mut names = Vec::new();
            let mut cols = Vec::new();
            for (k, v) in m.borrow().iter() {
                let Key::Str(n) = k else { return Err(type_err("column names must be strings")) };
                names.push(n.clone());
                cols.push(col_from_values(&super::to_vec(v, "column")?)?);
            }
            Ok(table_value(Table::new(names, cols)?))
        }
        Some(Value::List(l)) => table_from_rows(&l.borrow()).map(table_value),
        Some(other) => Err(type_err(format!("table() wants a map of columns or a list of rows, got {}", other.kind_name()))),
    }
}

// sha256 of a file we read, kept for the run report
fn record_file(vm: &mut Vm, path: &str, bytes: &[u8]) {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(bytes);
    let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    if !vm.data_files.iter().any(|(p, _)| p == path) {
        vm.data_files.push((path.to_string(), hex));
    }
}

fn read_bytes(vm: &mut Vm, path: &str) -> Result<Vec<u8>, Flow> {
    let bytes = std::fs::read(path).map_err(|e| err("IOError", format!("{path}: {e}")))?;
    record_file(vm, path, &bytes);
    Ok(bytes)
}

fn is_missing(s: &str) -> bool {
    matches!(s.trim(), "" | "NA" | "N/A" | "NaN" | "nan" | "null" | "NULL" | "None")
}

// load_csv(path, sep=",") with type guessing per column
pub fn load_csv(vm: &mut Vm, a: Args) -> R {
    let [p, sep] = a.bind(["path", "sep"])?;
    let path = need(p, "path")?.as_str("path")?.to_string();
    let sep = match sep {
        Some(s) => *s.as_str("sep")?.as_bytes().first().ok_or_else(|| value_err("empty sep"))?,
        None if path.ends_with(".tsv") => b'\t',
        None => b',',
    };
    let bytes = read_bytes(vm, &path)?;
    let mut rd = csv::ReaderBuilder::new().delimiter(sep).flexible(false).from_reader(&bytes[..]);
    let bad = |e: csv::Error| err("CSVError", format!("{path}: {e}"));
    let names: Vec<Rc<str>> = rd.headers().map_err(bad)?.iter().map(|h| Rc::from(h.trim())).collect();
    let mut raw: Vec<Vec<String>> = vec![Vec::new(); names.len()];
    for rec in rd.records() {
        let rec = rec.map_err(bad)?;
        for (i, f) in rec.iter().enumerate() {
            raw[i].push(f.to_string());
        }
    }
    let cols = raw.into_iter().map(|v| guess_col(&v)).collect();
    Ok(table_value(Table::new(names, cols)?))
}

fn guess_col(v: &[String]) -> Col {
    let mut nums = Vec::with_capacity(v.len());
    let mut int = true;
    for s in v {
        let t = s.trim();
        if is_missing(t) {
            nums.push(f64::NAN);
            continue;
        }
        let x = match t {
            "true" | "True" | "TRUE" => 1.0,
            "false" | "False" | "FALSE" => 0.0,
            _ => match t.parse::<f64>() {
                Ok(x) => {
                    if t.contains(['.', 'e', 'E']) || t.eq_ignore_ascii_case("inf") || t.eq_ignore_ascii_case("-inf") {
                        int = false;
                    }
                    x
                }
                Err(_) => {
                    return Col::Text(Rc::new(v.iter().map(|s| if is_missing(s) { None } else { Some(Rc::from(s.as_str())) }).collect()));
                }
            },
        };
        nums.push(x);
    }
    Col::Num(Rc::new(nums), int)
}

pub fn load_jsonl(vm: &mut Vm, a: Args) -> R {
    let [p] = a.bind(["path"])?;
    let path = need(p, "path")?.as_str("path")?.to_string();
    let bytes = read_bytes(vm, &path)?;
    let text = String::from_utf8_lossy(&bytes);
    let mut rows = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let v = super::json::decode(line).map_err(|_| err("JSONError", format!("{path}:{}: bad JSON line", i + 1)))?;
        rows.push(v);
    }
    table_from_rows(&rows).map(table_value)
}

pub fn load_json(vm: &mut Vm, a: Args) -> R {
    let [p] = a.bind(["path"])?;
    let path = need(p, "path")?.as_str("path")?.to_string();
    let bytes = read_bytes(vm, &path)?;
    let v = super::json::decode(&String::from_utf8_lossy(&bytes))?;
    match &v {
        Value::List(l) => table_from_rows(&l.borrow()).map(table_value),
        Value::Map(_) => table_fn(vm, Args::new(vec![v.clone()])),
        _ => Err(type_err("JSON file must hold a list of rows or a map of columns")),
    }
}
