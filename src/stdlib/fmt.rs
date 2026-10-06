use crate::vm::*;

// parsed `[[fill]align][sign][0][width][,][.prec][type]`
#[derive(Default)]
struct Spec {
    fill: char,
    align: Option<char>,
    sign: Option<char>,
    zero: bool,
    width: usize,
    group: Option<char>,
    prec: Option<usize>,
    ty: Option<char>,
}

fn parse(s: &str) -> Result<Spec, Flow> {
    let bad = || value_err(format!("bad format spec {s:?}"));
    let c: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut sp = Spec { fill: ' ', ..Default::default() };
    let is_align = |ch: char| matches!(ch, '<' | '>' | '^' | '=');
    if c.len() >= 2 && is_align(c[1]) {
        sp.fill = c[0];
        sp.align = Some(c[1]);
        i = 2;
    } else if !c.is_empty() && is_align(c[0]) {
        sp.align = Some(c[0]);
        i = 1;
    }
    if i < c.len() && matches!(c[i], '+' | '-' | ' ') {
        sp.sign = Some(c[i]);
        i += 1;
    }
    if i < c.len() && c[i] == '0' {
        sp.zero = true;
        i += 1;
    }
    let ws = i;
    while i < c.len() && c[i].is_ascii_digit() {
        i += 1;
    }
    if i > ws {
        sp.width = c[ws..i].iter().collect::<String>().parse().map_err(|_| bad())?;
    }
    if i < c.len() && matches!(c[i], ',' | '_') {
        sp.group = Some(c[i]);
        i += 1;
    }
    if i < c.len() && c[i] == '.' {
        i += 1;
        let ps = i;
        while i < c.len() && c[i].is_ascii_digit() {
            i += 1;
        }
        if i == ps {
            return Err(bad());
        }
        sp.prec = Some(c[ps..i].iter().collect::<String>().parse().map_err(|_| bad())?);
    }
    if i < c.len() {
        sp.ty = Some(c[i]);
        i += 1;
    }
    if i != c.len() || sp.width > 1000 || sp.prec.is_some_and(|p| p > 100) {
        return Err(bad());
    }
    Ok(sp)
}

fn group_digits(s: &str, sep: char) -> String {
    let (int, rest) = s.split_at(s.find(['.', 'e', 'E']).unwrap_or(s.len()));
    let mut out = String::new();
    for (i, ch) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push(sep);
        }
        out.push(ch);
    }
    out + rest
}

// python 'g': significant digits, trims zeros
fn general(x: f64, prec: usize) -> String {
    let p = prec.max(1);
    if x == 0.0 || !x.is_finite() {
        return fmt_float(x).trim_end_matches(".0").to_string();
    }
    let exp = x.abs().log10().floor() as i32;
    let s = if exp < -4 || exp >= p as i32 {
        py_exp(format!("{:.*e}", p - 1, x), false)
    } else {
        format!("{:.*}", (p as i32 - 1 - exp).max(0) as usize, x)
    };
    let (m, e) = s.split_at(s.find('e').unwrap_or(s.len()));
    let m = if m.contains('.') { m.trim_end_matches('0').trim_end_matches('.') } else { m };
    format!("{m}{e}")
}

pub fn format_spec(vm: &mut Vm, v: &Value, spec: &str) -> Result<String, Flow> {
    let sp = parse(spec)?;
    let is_num = matches!(v, Value::Int(_) | Value::Float(_) | Value::Bool(_));
    let body = match (sp.ty, v) {
        (None | Some('s'), _) if !is_num || sp.ty == Some('s') => {
            let s = vm.display(v, false)?;
            match sp.prec {
                Some(p) => s.chars().take(p).collect(),
                None => s,
            }
        }
        (Some('d'), _) | (None, Value::Int(_)) => {
            let n = v.int("value for {:d}")?;
            n.abs().to_string()
        }
        (Some('x' | 'X' | 'b' | 'o'), _) => {
            let n = v.int("value for hex/bin/oct")?;
            let m = n.unsigned_abs();
            match sp.ty {
                Some('x') => format!("{m:x}"),
                Some('X') => format!("{m:X}"),
                Some('b') => format!("{m:b}"),
                _ => format!("{m:o}"),
            }
        }
        (Some('f' | 'F') | None, _) => {
            let x = v.num("value")?;
            match sp.prec {
                Some(p) => format!("{:.*}", p, x.abs()),
                None if sp.ty.is_some() => format!("{:.6}", x.abs()),
                None => fmt_float(x.abs()),
            }
        }
        (Some('e' | 'E'), _) => {
            let x = v.num("value")?;
            py_exp(format!("{:.*e}", sp.prec.unwrap_or(6), x.abs()), sp.ty == Some('E'))
        }
        (Some('g' | 'G'), _) => {
            let s = general(v.num("value")?.abs(), sp.prec.unwrap_or(6));
            if sp.ty == Some('G') { s.to_uppercase() } else { s }
        }
        (Some('%'), _) => {
            let x = v.num("value")? * 100.0;
            format!("{:.*}%", sp.prec.unwrap_or(6), x.abs())
        }
        (Some(t), _) => return Err(value_err(format!("unknown format type `{t}`"))),
    };
    let neg = match v {
        Value::Int(n) => *n < 0,
        Value::Float(x) => x.is_sign_negative() && *x != 0.0 && !x.is_nan(),
        _ => false,
    } && is_num
        && !matches!(sp.ty, Some('s'));
    let body = match sp.group {
        Some(g) if is_num => group_digits(&body, g),
        _ => body,
    };
    let sign = if !is_num || sp.ty == Some('s') {
        ""
    } else if neg {
        "-"
    } else {
        match sp.sign {
            Some('+') => "+",
            Some(' ') => " ",
            _ => "",
        }
    };
    let len = sign.chars().count() + body.chars().count();
    if len >= sp.width {
        return Ok(format!("{sign}{body}"));
    }
    let gap = sp.width - len;
    if sp.zero && is_num && sp.align.is_none() {
        return Ok(format!("{sign}{}{body}", "0".repeat(gap)));
    }
    let align = sp.align.unwrap_or(if is_num { '>' } else { '<' });
    let f = |n: usize| sp.fill.to_string().repeat(n);
    Ok(match align {
        '<' => format!("{sign}{body}{}", f(gap)),
        '^' => format!("{}{sign}{body}{}", f(gap / 2), f(gap - gap / 2)),
        '=' => format!("{sign}{}{body}", f(gap)),
        _ => format!("{}{sign}{body}", f(gap)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::chunk::Program;

    fn f(v: Value, spec: &str) -> String {
        let mut vm = Vm::new(Program { entry: "x".into(), modules: vec![] }, Box::new(std::io::sink()));
        format_spec(&mut vm, &v, spec).unwrap()
    }

    #[test]
    fn specs() {
        assert_eq!(f(Value::Float(1.23456), ".2f"), "1.23");
        assert_eq!(f(Value::Float(1.23456), ".2"), "1.23");
        assert_eq!(f(Value::Float(0.1234), ".1%"), "12.3%");
        assert_eq!(f(Value::Float(-0.00012), ".2e"), "-1.20e-04");
        assert_eq!(f(Value::Int(1234567), ","), "1,234,567");
        assert_eq!(f(Value::Int(-42), "05d"), "-0042");
        assert_eq!(f(Value::Int(255), "x"), "ff");
        assert_eq!(f(Value::str("ab"), ">5"), "   ab");
        assert_eq!(f(Value::str("ab"), "*^6"), "**ab**");
        assert_eq!(f(Value::Float(1234.5), ",.1f"), "1,234.5");
        assert_eq!(f(Value::Float(0.000012345), "g"), "1.2345e-05");
        assert_eq!(f(Value::Float(123.0), "g"), "123");
        assert_eq!(f(Value::Float(5.0), "+.1f"), "+5.0");
    }
}
