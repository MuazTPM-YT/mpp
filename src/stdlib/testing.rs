use crate::vm::*;

// mark the running test as skipped
pub fn skip(_: &mut Vm, a: Args) -> R {
    let [reason] = a.bind(["reason"])?;
    let r = match reason {
        Some(v) => v.as_str("reason")?.to_string(),
        None => String::new(),
    };
    Err(err("Skipped", r))
}

// extra line shown in test reports
pub fn note(vm: &mut Vm, a: Args) -> R {
    a.no_kw()?;
    let parts: Vec<String> = a.pos.iter().map(|v| vm.display(v, false)).collect::<Result<_, _>>()?;
    let text = parts.join(" ");
    match &mut vm.test_ctx {
        Some(ctx) => ctx.notes.push(text),
        None => {
            let _ = std::io::Write::flush(&mut vm.out);
            eprintln!("note: {text}");
        }
    }
    Ok(Value::Nil)
}

// call f; it must throw (optionally a given kind / message part)
pub fn expect_throws(vm: &mut Vm, a: Args) -> R {
    let [f, kind, contains] = a.bind(["fn", "kind", "contains"])?;
    let f = need(f, "fn")?;
    match vm.call(&f, &[]) {
        Ok(v) => {
            let shown = vm.display(&v, true)?;
            Err(err("ExpectFailed", format!("expected an error, but it returned {shown}")))
        }
        Err(Flow::Throw(e)) => {
            let Value::Error(obj) = &e else { return Ok(e) };
            if &*obj.kind == "Skipped" || &*obj.kind == "TimeoutError" {
                return Err(Flow::Throw(e));
            }
            if let Some(k) = opt(kind) {
                let k = k.as_str("kind")?;
                if obj.kind != *k {
                    return Err(err("ExpectFailed", format!("expected a {k} error, got {}: {}", obj.kind, obj.message)));
                }
            }
            if let Some(c) = opt(contains) {
                let c = c.as_str("contains")?;
                if !obj.message.contains(&**c) {
                    return Err(err("ExpectFailed", format!("expected error message to contain {:?}, got {:?}", &**c, &*obj.message)));
                }
            }
            Ok(e)
        }
        Err(exit) => Err(exit),
    }
}

// compare value with the saved snapshot; first run saves it
pub fn expect_snapshot(vm: &mut Vm, a: Args) -> R {
    let [v, name] = a.bind(["value", "name"])?;
    let v = need(v, "value")?;
    let json = match super::json::to_json(&v, 0) {
        Ok(j) => j,
        Err(_) => serde_json::Value::String(vm.display(&v, true)?),
    };
    let Some(ctx) = &mut vm.test_ctx else {
        return Err(err("Error", "expect_snapshot() only works inside a test block, run with `mpp test`"));
    };
    let Some(store) = ctx.snapshots.clone() else { return Err(err("Error", "no snapshot store")) };
    ctx.snap_index += 1;
    let key = match name {
        Some(n) => format!("{} :: {}", ctx.name, n.as_str("name")?),
        None => format!("{} #{}", ctx.name, ctx.snap_index),
    };
    let mut st = store.borrow_mut();
    match st.data.get(&key) {
        None => {
            st.data.insert(key.clone(), json);
            st.dirty = true;
            ctx.notes.push(format!("new snapshot saved: {key}"));
        }
        Some(old) if *old == json => {}
        Some(_) if ctx.update_snapshots => {
            st.data.insert(key.clone(), json);
            st.dirty = true;
            ctx.notes.push(format!("snapshot updated: {key}"));
        }
        Some(old) => {
            let want = serde_json::to_string_pretty(old).unwrap_or_default();
            let got = serde_json::to_string_pretty(&json).unwrap_or_default();
            return Err(err(
                "ExpectFailed",
                format!(
                    "snapshot `{key}` changed (run with --update-snapshots to accept)\n  saved: {}\n  now:   {}",
                    want.replace('\n', "\n         "),
                    got.replace('\n', "\n         ")
                ),
            ));
        }
    }
    Ok(Value::Nil)
}
