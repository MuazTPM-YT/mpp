use super::token::{FPiece, Tok, Token, keyword};
use super::{Diag, Span};

// turn source into tokens; `base` shifts spans (used for f-string parts)
pub fn lex(src: &str, base: u32) -> Result<Vec<Token>, Diag> {
    let mut lx = Lexer { src, b: src.as_bytes(), pos: 0, base: base as usize, stack: Vec::new(), out: Vec::new() };
    lx.run()?;
    Ok(lx.out)
}

struct Lexer<'a> {
    src: &'a str,
    b: &'a [u8],
    pos: usize,
    base: usize,
    // open brackets; newlines inside ( or [ are ignored
    stack: Vec<u8>,
    out: Vec<Token>,
}

impl Lexer<'_> {
    fn span(&self, start: usize) -> Span {
        Span::new(self.base + start, self.base + self.pos)
    }

    fn err(&self, msg: impl Into<String>, start: usize) -> Diag {
        Diag::new(msg, Span::new(self.base + start, self.base + self.pos.max(start + 1)))
    }

    fn peek(&self, off: usize) -> u8 {
        *self.b.get(self.pos + off).unwrap_or(&0)
    }

    fn push(&mut self, tok: Tok, start: usize) {
        let span = self.span(start);
        self.out.push(Token { tok, span });
    }

    fn last_is_newline(&self) -> bool {
        matches!(self.out.last().map(|t| &t.tok), None | Some(Tok::Newline))
    }

    // next line starts with `.name`? then this newline is a method chain
    fn chain_follows(&self) -> bool {
        let mut i = self.pos;
        loop {
            match self.b.get(i) {
                Some(b' ' | b'\t' | b'\r' | b'\n') => i += 1,
                Some(b'#') => {
                    while !matches!(self.b.get(i), None | Some(b'\n')) {
                        i += 1;
                    }
                }
                Some(b'.') => return self.b.get(i + 1) != Some(&b'.'),
                _ => return false,
            }
        }
    }

    fn run(&mut self) -> Result<(), Diag> {
        loop {
            while matches!(self.peek(0), b' ' | b'\t' | b'\r') {
                self.pos += 1;
            }
            if self.peek(0) == b'#' {
                while !matches!(self.peek(0), 0 | b'\n') {
                    self.pos += 1;
                }
            }
            let start = self.pos;
            if self.pos >= self.b.len() {
                if let Some(&open) = self.stack.last() {
                    return Err(self.err(format!("`{}` never closed", open as char), self.pos.saturating_sub(1)));
                }
                if !self.last_is_newline() {
                    self.push(Tok::Newline, start);
                }
                self.push(Tok::Eof, start);
                return Ok(());
            }
            let c = self.peek(0);
            if c == b'\n' {
                self.pos += 1;
                let in_parens = matches!(self.stack.last(), Some(b'(' | b'['));
                if !in_parens && !self.last_is_newline() && !self.chain_follows() {
                    self.push(Tok::Newline, start);
                }
                continue;
            }
            if c.is_ascii_digit() {
                self.number()?;
                continue;
            }
            if (c == b'f' || c == b'r') && matches!(self.peek(1), b'"' | b'\'') {
                self.pos += 1;
                if c == b'f' {
                    self.fstring(start)?
                } else {
                    self.string(start, true)?
                }
                continue;
            }
            if c == b'"' || c == b'\'' {
                self.string(start, false)?;
                continue;
            }
            if c == b'_' || c.is_ascii_alphabetic() || c >= 0x80 {
                self.ident()?;
                continue;
            }
            self.punct(start)?;
        }
    }

    fn ident(&mut self) -> Result<(), Diag> {
        let start = self.pos;
        while let Some(ch) = self.src[self.pos..].chars().next() {
            if ch == '_' || ch.is_alphanumeric() {
                self.pos += ch.len_utf8();
            } else {
                break;
            }
        }
        if self.pos == start {
            let ch = self.src[self.pos..].chars().next().unwrap_or('?');
            self.pos += ch.len_utf8();
            return Err(self.err(format!("unexpected character `{ch}`"), start));
        }
        let word = &self.src[start..self.pos];
        let tok = keyword(word).unwrap_or_else(|| Tok::Ident(word.into()));
        self.push(tok, start);
        Ok(())
    }

    fn number(&mut self) -> Result<(), Diag> {
        let start = self.pos;
        let radix = match (self.peek(0), self.peek(1)) {
            (b'0', b'x' | b'X') => 16,
            (b'0', b'b' | b'B') => 2,
            (b'0', b'o' | b'O') => 8,
            _ => 10,
        };
        if radix != 10 {
            self.pos += 2;
            let ds = self.pos;
            while self.peek(0).is_ascii_alphanumeric() || self.peek(0) == b'_' {
                self.pos += 1;
            }
            let digits: String = self.src[ds..self.pos].chars().filter(|&c| c != '_').collect();
            let n = i64::from_str_radix(&digits, radix).map_err(|_| self.err("bad number", start))?;
            self.push(Tok::Int(n), start);
            return Ok(());
        }
        let mut float = false;
        while self.peek(0).is_ascii_digit() || self.peek(0) == b'_' {
            self.pos += 1;
        }
        if self.peek(0) == b'.' && self.peek(1).is_ascii_digit() {
            float = true;
            self.pos += 1;
            while self.peek(0).is_ascii_digit() || self.peek(0) == b'_' {
                self.pos += 1;
            }
        }
        if matches!(self.peek(0), b'e' | b'E') {
            let save = self.pos;
            self.pos += 1;
            if matches!(self.peek(0), b'+' | b'-') {
                self.pos += 1;
            }
            if self.peek(0).is_ascii_digit() {
                float = true;
                while self.peek(0).is_ascii_digit() {
                    self.pos += 1;
                }
            } else {
                self.pos = save;
            }
        }
        if self.peek(0).is_ascii_alphabetic() || self.peek(0) == b'_' {
            return Err(self.err("letters right after a number", start));
        }
        let text: String = self.src[start..self.pos].chars().filter(|&c| c != '_').collect();
        let tok = if float {
            Tok::Float(text.parse().map_err(|_| self.err("bad number", start))?)
        } else {
            Tok::Int(text.parse().map_err(|_| self.err("number too big for int (max 9223372036854775807)", start))?)
        };
        self.push(tok, start);
        Ok(())
    }

    // opening quote: returns (quote char, triple?)
    fn open_quote(&mut self) -> (u8, bool) {
        let q = self.peek(0);
        if self.peek(1) == q && self.peek(2) == q {
            self.pos += 3;
            (q, true)
        } else {
            self.pos += 1;
            (q, false)
        }
    }

    fn at_close(&self, q: u8, triple: bool) -> bool {
        self.peek(0) == q && (!triple || (self.peek(1) == q && self.peek(2) == q))
    }

    // read one escape after backslash into out
    fn escape(&mut self, out: &mut String, start: usize) -> Result<(), Diag> {
        let e = self.peek(1);
        self.pos += 2;
        match e {
            b'n' => out.push('\n'),
            b't' => out.push('\t'),
            b'r' => out.push('\r'),
            b'0' => out.push('\0'),
            b'\\' => out.push('\\'),
            b'"' => out.push('"'),
            b'\'' => out.push('\''),
            b'{' => out.push('{'),
            b'}' => out.push('}'),
            b'\n' => {}
            b'u' if self.peek(0) == b'{' => {
                let hs = self.pos + 1;
                while !matches!(self.peek(0), b'}' | 0 | b'\n') {
                    self.pos += 1;
                }
                let hex = &self.src[hs..self.pos];
                self.pos += 1;
                let ch = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32);
                out.push(ch.ok_or_else(|| self.err("bad unicode escape", start))?);
            }
            _ => return Err(self.err("unknown escape, use \\\\ for a backslash", self.pos - 2)),
        }
        Ok(())
    }

    fn string(&mut self, start: usize, raw: bool) -> Result<(), Diag> {
        let (q, triple) = self.open_quote();
        let mut out = String::new();
        loop {
            if self.pos >= self.b.len() || (!triple && self.peek(0) == b'\n') {
                return Err(self.err("string never closed", start));
            }
            if self.at_close(q, triple) {
                self.pos += if triple { 3 } else { 1 };
                break;
            }
            if self.peek(0) == b'\\' && !raw {
                self.escape(&mut out, start)?;
                continue;
            }
            let ch = self.src[self.pos..].chars().next().unwrap();
            out.push(ch);
            self.pos += ch.len_utf8();
        }
        self.push(Tok::Str(out.into()), start);
        Ok(())
    }

    fn fstring(&mut self, start: usize) -> Result<(), Diag> {
        let (q, triple) = self.open_quote();
        let mut pieces = Vec::new();
        let mut lit = String::new();
        loop {
            if self.pos >= self.b.len() || (!triple && self.peek(0) == b'\n') {
                return Err(self.err("f-string never closed", start));
            }
            if self.at_close(q, triple) {
                self.pos += if triple { 3 } else { 1 };
                break;
            }
            match self.peek(0) {
                b'{' if self.peek(1) == b'{' => {
                    lit.push('{');
                    self.pos += 2;
                }
                b'}' if self.peek(1) == b'}' => {
                    lit.push('}');
                    self.pos += 2;
                }
                b'}' => return Err(self.err("single `}` in f-string, write `}}`", self.pos)),
                b'{' => {
                    if !lit.is_empty() {
                        pieces.push(FPiece::Lit(std::mem::take(&mut lit)));
                    }
                    self.pos += 1;
                    pieces.push(self.fstring_expr()?);
                }
                b'\\' => self.escape(&mut lit, start)?,
                _ => {
                    let ch = self.src[self.pos..].chars().next().unwrap();
                    lit.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
        if !lit.is_empty() {
            pieces.push(FPiece::Lit(lit));
        }
        self.push(Tok::FStr(pieces), start);
        Ok(())
    }

    // inside `{...}` of an f-string; stops at `}` or `:` on depth 0
    fn fstring_expr(&mut self) -> Result<FPiece, Diag> {
        let es = self.pos;
        let mut depth = 0i32;
        loop {
            let c = self.peek(0);
            match c {
                0 | b'\n' => return Err(self.err("`{` in f-string never closed", es - 1)),
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' => depth -= 1,
                b'}' if depth > 0 => depth -= 1,
                b'}' | b':' if depth == 0 => break,
                b'"' | b'\'' => {
                    self.pos += 1;
                    while !matches!(self.peek(0), 0 | b'\n') && self.peek(0) != c {
                        if self.peek(0) == b'\\' {
                            self.pos += 1;
                        }
                        self.pos += 1;
                    }
                }
                _ => {}
            }
            self.pos += 1;
        }
        let src = self.src[es..self.pos].to_string();
        if src.trim().is_empty() {
            return Err(self.err("empty `{}` in f-string", es - 1));
        }
        let mut spec = None;
        if self.peek(0) == b':' {
            self.pos += 1;
            let ss = self.pos;
            while !matches!(self.peek(0), b'}' | 0 | b'\n') {
                self.pos += 1;
            }
            spec = Some(self.src[ss..self.pos].to_string());
        }
        if self.peek(0) != b'}' {
            return Err(self.err("`{` in f-string never closed", es - 1));
        }
        self.pos += 1;
        Ok(FPiece::Expr { src, offset: (self.base + es) as u32, spec })
    }

    fn punct(&mut self, start: usize) -> Result<(), Diag> {
        let (c, n, n2) = (self.peek(0), self.peek(1), self.peek(2));
        let (tok, len) = match (c, n) {
            (b'.', b'.') if n2 == b'=' => (Tok::DotDotEq, 3),
            (b'.', b'.') => (Tok::DotDot, 2),
            (b'*', b'*') => (Tok::StarStar, 2),
            (b'/', b'/') => (Tok::SlashSlash, 2),
            (b'=', b'>') => (Tok::Arrow, 2),
            (b'=', b'=') => (Tok::EqEq, 2),
            (b'!', b'=') => (Tok::Ne, 2),
            (b'<', b'=') => (Tok::Le, 2),
            (b'>', b'=') => (Tok::Ge, 2),
            (b'+', b'=') => (Tok::PlusEq, 2),
            (b'-', b'=') => (Tok::MinusEq, 2),
            (b'*', b'=') => (Tok::StarEq, 2),
            (b'/', b'=') => (Tok::SlashEq, 2),
            (b'%', b'=') => (Tok::PercentEq, 2),
            (b'~', b'=') => (Tok::Approx, 2),
            (b'(', _) => (Tok::LParen, 1),
            (b')', _) => (Tok::RParen, 1),
            (b'[', _) => (Tok::LBracket, 1),
            (b']', _) => (Tok::RBracket, 1),
            (b'{', _) => (Tok::LBrace, 1),
            (b'}', _) => (Tok::RBrace, 1),
            (b',', _) => (Tok::Comma, 1),
            (b'.', _) => (Tok::Dot, 1),
            (b':', _) => (Tok::Colon, 1),
            (b';', _) => (Tok::Semi, 1),
            (b'?', _) => (Tok::Question, 1),
            (b'+', _) => (Tok::Plus, 1),
            (b'-', _) => (Tok::Minus, 1),
            (b'*', _) => (Tok::Star, 1),
            (b'/', _) => (Tok::Slash, 1),
            (b'%', _) => (Tok::Percent, 1),
            (b'=', _) => (Tok::Assign, 1),
            (b'<', _) => (Tok::Lt, 1),
            (b'>', _) => (Tok::Gt, 1),
            (b'!', _) => {
                self.pos += 1;
                return Err(self.err("use `not` instead of `!`", start));
            }
            (b'&', b'&') | (b'|', b'|') => {
                self.pos += 2;
                return Err(self.err(if c == b'&' { "use `and` instead of `&&`" } else { "use `or` instead of `||`" }, start));
            }
            _ => {
                self.pos += 1;
                return Err(self.err(format!("unexpected character `{}`", c as char), start));
            }
        };
        self.pos += len;
        match tok {
            Tok::LParen => self.stack.push(b'('),
            Tok::LBracket => self.stack.push(b'['),
            Tok::LBrace => self.stack.push(b'{'),
            Tok::RParen | Tok::RBracket | Tok::RBrace => {
                let want = match tok {
                    Tok::RParen => b'(',
                    Tok::RBracket => b'[',
                    _ => b'{',
                };
                match self.stack.pop() {
                    Some(open) if open == want => {}
                    Some(open) => {
                        return Err(self.err(format!("`{}` does not match open `{}`", c as char, open as char), start));
                    }
                    None => {
                        return Err(self.err(format!("`{}` has no matching open bracket", c as char), start));
                    }
                }
            }
            _ => {}
        }
        self.push(tok, start);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<Tok> {
        lex(s, 0).unwrap().into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn newlines_and_parens() {
        assert_eq!(
            toks("f(1,\n2)\n"),
            vec![Tok::Ident("f".into()), Tok::LParen, Tok::Int(1), Tok::Comma, Tok::Int(2), Tok::RParen, Tok::Newline, Tok::Eof]
        );
        // block inside parens keeps newlines
        let t = toks("g(fn() {\na\nb\n})");
        assert_eq!(t.iter().filter(|t| **t == Tok::Newline).count(), 4);
    }

    #[test]
    fn chain_continues() {
        assert_eq!(toks("a\n  .b()").iter().filter(|t| **t == Tok::Newline).count(), 1);
    }

    #[test]
    fn numbers_and_strings() {
        assert_eq!(
            toks("1_000 0x1f 2.5e3 1..3")[..6],
            [Tok::Int(1000), Tok::Int(31), Tok::Float(2500.0), Tok::Int(1), Tok::DotDot, Tok::Int(3)]
        );
        assert_eq!(toks(r#""a\tb""#)[0], Tok::Str("a\tb".into()));
        assert_eq!(toks(r#"r"a\d""#)[0], Tok::Str("a\\d".into()));
        assert_eq!(toks("\"\"\"x\ny\"\"\"")[0], Tok::Str("x\ny".into()));
    }

    #[test]
    fn fstring_parts() {
        let t = toks(r#"f"x={m["k"]:.2f}!""#);
        match &t[0] {
            Tok::FStr(p) => {
                assert_eq!(p[0], FPiece::Lit("x=".into()));
                assert!(matches!(&p[1], FPiece::Expr { src, spec: Some(s), .. } if src == "m[\"k\"]" && s == ".2f"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn errors() {
        assert!(lex("\"abc", 0).is_err());
        assert!(lex("(]", 0).is_err());
        assert!(lex("a && b", 0).is_err());
    }
}
