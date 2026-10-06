// mpp fmt: re-space and re-indent tokens; keeps comments, strings and line breaks
use crate::syntax::lexer::lex;
use crate::syntax::token::{Tok, Token};
use crate::syntax::{Diag, Span};

#[derive(Clone, Copy, PartialEq)]
enum Open {
    Paren,
    Bracket,
    Block,
    Map,
}

struct Frame {
    kind: Open,
    line: usize,
    ternary: usize,
}

// format source; refuses (Err) rather than change meaning
pub fn format(src: &str) -> Result<String, Diag> {
    let toks: Vec<Token> = lex(src, 0)?.into_iter().filter(|t| !matches!(t.tok, Tok::Newline | Tok::Eof)).collect();
    let out = emit(src, &toks);
    // safety net: same tokens, same statement breaks
    let before = shape(src)?;
    let after =
        shape(&out).map_err(|d| Diag::new(format!("formatter produced bad code ({}); file left unchanged", d.msg), Span::default()))?;
    if before != after {
        return Err(Diag::new("formatter would change the meaning of this file; left unchanged (please report this)", Span::default()));
    }
    Ok(out)
}

// token kinds and text, ignoring positions (f-string parts carry offsets)
fn shape(src: &str) -> Result<Vec<String>, Diag> {
    use crate::syntax::token::FPiece;
    Ok(lex(src, 0)?
        .iter()
        .map(|t| match &t.tok {
            Tok::FStr(ps) => ps
                .iter()
                .map(|p| match p {
                    FPiece::Lit(s) => format!("L{s:?}"),
                    FPiece::Expr { src, spec, .. } => format!("E{src:?}{spec:?}"),
                })
                .collect::<Vec<_>>()
                .join("|"),
            other => format!("{other:?}"),
        })
        .collect())
}

fn is_operand_end(t: &Tok) -> bool {
    matches!(
        t,
        Tok::Ident(_)
            | Tok::Int(_)
            | Tok::Float(_)
            | Tok::Str(_)
            | Tok::FStr(_)
            | Tok::RParen
            | Tok::RBracket
            | Tok::RBrace
            | Tok::Nil
            | Tok::True
            | Tok::False
            | Tok::Super
    )
}

fn is_binop(t: &Tok) -> bool {
    matches!(
        t,
        Tok::Plus
            | Tok::Minus
            | Tok::Star
            | Tok::StarStar
            | Tok::Slash
            | Tok::SlashSlash
            | Tok::Percent
            | Tok::Assign
            | Tok::EqEq
            | Tok::Ne
            | Tok::Lt
            | Tok::Le
            | Tok::Gt
            | Tok::Ge
            | Tok::PlusEq
            | Tok::MinusEq
            | Tok::StarEq
            | Tok::SlashEq
            | Tok::PercentEq
            | Tok::Approx
            | Tok::Arrow
            | Tok::Question
            | Tok::And
            | Tok::Or
            | Tok::In
    )
}

// gap between tokens: comments (same-line?, text) and newline count
struct Gap {
    trailing: Option<String>,
    lines: Vec<Option<String>>, // after the first newline: None = blank line, Some = comment line
    breaks: usize,
}

fn read_gap(g: &str) -> Gap {
    let mut gap = Gap { trailing: None, lines: Vec::new(), breaks: 0 };
    for (i, part) in g.split('\n').enumerate() {
        let t = part.trim();
        if i > 0 {
            gap.breaks += 1;
        }
        let comment = t.strip_prefix('#').map(|_| t.trim_end().to_string());
        if i == 0 {
            gap.trailing = comment;
        } else if i < g.split('\n').count() - 1 || comment.is_some() {
            gap.lines.push(comment);
        }
    }
    gap
}

fn emit(src: &str, toks: &[Token]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    // root frame never pops and never indents
    let mut stack: Vec<Frame> = vec![Frame { kind: Open::Block, line: usize::MAX, ternary: 0 }];
    let mut prev: Option<&Tok> = None;
    let mut prev2: Option<&Tok> = None;
    let mut line_no = 0usize;
    let mut last_end = 0usize;
    let mut class_head = false;
    let mut colon_spaced = true;
    for t in toks {
        let gap = read_gap(&src[last_end..t.span.start as usize]);
        let text = &src[t.span.start as usize..t.span.end as usize];
        last_end = t.span.end as usize;
        let closes = matches!(t.tok, Tok::RParen | Tok::RBracket | Tok::RBrace);
        let fresh = gap.breaks > 0 || prev.is_none();
        if fresh {
            match (&gap.trailing, prev) {
                (Some(c), None) => out.push(c.clone()),
                (Some(c), Some(_)) => {
                    line.push_str("  ");
                    line.push_str(c);
                }
                _ => {}
            }
            if prev.is_some() {
                out.push(line.trim_end().to_string());
                line.clear();
            }
            // a closer line drops every frame opened on the same line as its partner
            let frames: Vec<&Frame> = match (closes && stack.len() > 1).then(|| stack.last().unwrap().line) {
                Some(l) => stack.iter().filter(|f| f.line != l).collect(),
                None => stack.iter().collect(),
            };
            let ind = indent(&frames, 0);
            push_gap_lines(&mut out, &gap.lines, &ind);
            line_no += 1;
            line.push_str(&indent(&frames, matches!(t.tok, Tok::Dot) as usize));
        } else if space_between(prev.unwrap(), prev2, &t.tok, stack.last().unwrap(), class_head, colon_spaced) {
            line.push(' ');
        }
        line.push_str(text);
        match &t.tok {
            Tok::LParen => stack.push(Frame { kind: Open::Paren, line: line_no, ternary: 0 }),
            Tok::LBracket => stack.push(Frame { kind: Open::Bracket, line: line_no, ternary: 0 }),
            Tok::LBrace => {
                let top = stack.last().unwrap().kind;
                let map = (fresh && !matches!(top, Open::Paren | Open::Bracket)) || is_map_open(prev);
                stack.push(Frame { kind: if map { Open::Map } else { Open::Block }, line: line_no, ternary: 0 });
            }
            Tok::RParen | Tok::RBracket | Tok::RBrace if stack.len() > 1 => {
                stack.pop();
            }
            Tok::Question => stack.last_mut().unwrap().ternary += 1,
            Tok::Colon => {
                let f = stack.last_mut().unwrap();
                colon_spaced = f.ternary > 0 || class_head || f.kind != Open::Bracket;
                f.ternary = f.ternary.saturating_sub(1);
            }
            _ => {}
        }
        class_head = match t.tok {
            Tok::Class => true,
            Tok::LBrace => false,
            _ => class_head,
        };
        prev2 = prev;
        prev = Some(&t.tok);
    }
    let tail = read_gap(&src[last_end..]);
    if let Some(c) = &tail.trailing {
        if prev.is_some() {
            line.push_str("  ");
        }
        line.push_str(c);
    }
    if !line.trim().is_empty() {
        out.push(line.trim_end().to_string());
    }
    push_gap_lines(&mut out, &tail.lines, "");
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

// one indent per distinct line that opened a bracket
fn indent(stack: &[&Frame], extra: usize) -> String {
    let mut lines: Vec<usize> = stack.iter().map(|f| f.line).filter(|l| *l != usize::MAX).collect();
    lines.dedup();
    "    ".repeat(lines.len() + extra)
}

// comment lines between tokens; blank lines squeezed to one
fn push_gap_lines(out: &mut Vec<String>, lines: &[Option<String>], ind: &str) {
    let mut blank = false;
    for l in lines {
        match l {
            None => blank = true,
            Some(c) => {
                if blank && out.last().is_some_and(|x| !x.is_empty()) {
                    out.push(String::new());
                }
                blank = false;
                out.push(format!("{ind}{c}"));
            }
        }
    }
    if blank && out.last().is_some_and(|x| !x.is_empty()) {
        out.push(String::new());
    }
}

// `{` starts a map literal (not a block) when it sits where a value goes
fn is_map_open(prev: Option<&Tok>) -> bool {
    prev.is_some_and(|p| {
        matches!(p, Tok::Assign | Tok::LParen | Tok::LBracket | Tok::Comma | Tok::Colon | Tok::Return | Tok::Question)
            || (is_binop(p) && !matches!(p, Tok::Arrow))
            || matches!(p, Tok::Ident(w) if &**w == "report" || &**w == "expect")
    })
}

fn space_between(p: &Tok, pp: Option<&Tok>, c: &Tok, top: &Frame, class_head: bool, colon_spaced: bool) -> bool {
    // closers and separators hug the left
    if matches!(c, Tok::Comma | Tok::Semi | Tok::RParen | Tok::RBracket) {
        return false;
    }
    if matches!(c, Tok::RBrace) {
        return !matches!(p, Tok::LBrace) && top.kind == Open::Block;
    }
    if matches!(p, Tok::LParen | Tok::LBracket) {
        return false;
    }
    if matches!(p, Tok::LBrace) {
        return top.kind == Open::Block;
    }
    if matches!(p, Tok::Comma | Tok::Semi) {
        return true;
    }
    if matches!(c, Tok::Dot | Tok::DotDot | Tok::DotDotEq) || matches!(p, Tok::Dot | Tok::DotDot | Tok::DotDotEq) {
        return false;
    }
    if matches!(c, Tok::Colon) {
        // ternary and class bases are spaced; map keys and slices hug
        return top.ternary > 0 || class_head;
    }
    if matches!(p, Tok::Colon) {
        return colon_spaced;
    }
    // unary minus / plus: no space after
    if matches!(p, Tok::Minus | Tok::Plus) && pp.is_none_or(|x| !is_operand_end(x)) {
        return false;
    }
    if matches!(c, Tok::LParen) {
        return !matches!(p, Tok::Ident(_) | Tok::RParen | Tok::RBracket | Tok::Fn);
    }
    if matches!(c, Tok::LBracket) {
        return !is_operand_end(p);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::format;

    fn f(s: &str) -> String {
        format(s).unwrap()
    }

    #[test]
    fn spacing_and_indent() {
        assert_eq!(f("x=1+2*  3\n"), "x = 1 + 2 * 3\n");
        assert_eq!(f("fn add(a,b=2){\nreturn a+b\n}\n"), "fn add(a, b = 2) {\n    return a + b\n}\n");
        assert_eq!(f("y = -x + f(-1, a[1:2], [1, 2])\n"), "y = -x + f(-1, a[1:2], [1, 2])\n");
        assert_eq!(f("m = {\"a\":1, \"b\" : [1,2]}\n"), "m = {\"a\": 1, \"b\": [1, 2]}\n");
        assert_eq!(f("if a {b()} else {c()}\n"), "if a { b() } else { c() }\n");
        assert_eq!(f("z = c ? 1:2\n"), "z = c ? 1 : 2\n");
        assert_eq!(f("class B:A {\nfn init(self) { super.init() }\n}\n"), "class B : A {\n    fn init(self) { super.init() }\n}\n");
        assert_eq!(f("xs.map(x=>x*2)\n"), "xs.map(x => x * 2)\n");
        assert_eq!(f("for i in 0..n {}\n"), "for i in 0..n {}\n");
        assert_eq!(f("fn add(a,b){\nreturn f\"{a}\"\n}\n"), "fn add(a, b) {\n    return f\"{a}\"\n}\n");
    }

    #[test]
    fn comments_blank_lines_chains() {
        let src = "# top\n\n\n\nx = 1   # one\nif x {\n  # inside\n      y = 2\n}\nw = data\n.filter(r => r.ok)\n  .len()\n";
        assert_eq!(f(src), "# top\n\nx = 1  # one\nif x {\n    # inside\n    y = 2\n}\nw = data\n    .filter(r => r.ok)\n    .len()\n");
    }

    #[test]
    fn nested_openers_one_level() {
        let src = "xs.each(fn(x) {\nprint(x)\n})\nt = f(\n1,\n2\n)\n";
        assert_eq!(f(src), "xs.each(fn(x) {\n    print(x)\n})\nt = f(\n    1,\n    2\n)\n");
    }

    #[test]
    fn idempotent_on_cases() {
        for e in std::fs::read_dir("tests/cases").unwrap().flatten() {
            if e.path().extension().is_some_and(|x| x == "mpp") {
                let src = std::fs::read_to_string(e.path()).unwrap();
                let once = format(&src).unwrap_or_else(|d| panic!("{}: {}", e.path().display(), d.msg));
                assert_eq!(format(&once).unwrap(), once, "{}", e.path().display());
            }
        }
    }
}
