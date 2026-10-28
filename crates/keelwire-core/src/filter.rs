//! Sniffer filter expression language.
//!
//! ```text
//! expr    := or
//! or      := and ( "||" and )*
//! and     := unary ( "&&" unary )*
//! unary   := "!" unary | "(" expr ")" | cmp
//! cmp     := ident op literal | ident "in" "[" literal ("," literal)* "]" | ident
//! op      := == != < > <= >= ~ (contains, case-insensitive)
//! ```
//! Identifiers: pgn, src, dst, prio, len, name, data (hex), tx (bool) and
//! any decoded field id. Number literals are decimal or 0x-hex.

#[derive(Clone, Debug, PartialEq)]
pub enum Lit {
    Num(f64),
    Str(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    Contains,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
    Cmp(String, Op, Lit),
    In(String, Vec<Lit>),
    Truthy(String),
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    Num(f64),
    Str(String),
    Op(Op),
    And,
    Or,
    Not,
    LParen,
    RParen,
    LBrack,
    RBrack,
    Comma,
    In,
}

fn lex(s: &str) -> Result<Vec<Tok>, String> {
    let c: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
            continue;
        }
        let two: String = c[i..(i + 2).min(c.len())].iter().collect();
        let push2 = |t: Tok, out: &mut Vec<Tok>| out.push(t);
        match two.as_str() {
            "&&" => {
                push2(Tok::And, &mut out);
                i += 2;
                continue;
            }
            "||" => {
                push2(Tok::Or, &mut out);
                i += 2;
                continue;
            }
            "==" => {
                push2(Tok::Op(Op::Eq), &mut out);
                i += 2;
                continue;
            }
            "!=" => {
                push2(Tok::Op(Op::Ne), &mut out);
                i += 2;
                continue;
            }
            "<=" => {
                push2(Tok::Op(Op::Le), &mut out);
                i += 2;
                continue;
            }
            ">=" => {
                push2(Tok::Op(Op::Ge), &mut out);
                i += 2;
                continue;
            }
            _ => {}
        }
        match ch {
            '<' => out.push(Tok::Op(Op::Lt)),
            '>' => out.push(Tok::Op(Op::Gt)),
            '~' => out.push(Tok::Op(Op::Contains)),
            '=' => out.push(Tok::Op(Op::Eq)),
            '!' => out.push(Tok::Not),
            '(' => out.push(Tok::LParen),
            ')' => out.push(Tok::RParen),
            '[' => out.push(Tok::LBrack),
            ']' => out.push(Tok::RBrack),
            ',' => out.push(Tok::Comma),
            '"' | '\'' => {
                let q = ch;
                let mut j = i + 1;
                let mut s = String::new();
                while j < c.len() && c[j] != q {
                    s.push(c[j]);
                    j += 1;
                }
                if j >= c.len() {
                    return Err("unterminated string".into());
                }
                out.push(Tok::Str(s));
                i = j + 1;
                continue;
            }
            _ if ch.is_ascii_digit() || (ch == '-' && c.get(i + 1).map(|x| x.is_ascii_digit()).unwrap_or(false)) => {
                let mut j = i + 1;
                while j < c.len() && (c[j].is_ascii_alphanumeric() || c[j] == '.') {
                    j += 1;
                }
                let t: String = c[i..j].iter().collect();
                let v = if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
                    i64::from_str_radix(h, 16).map(|x| x as f64).map_err(|_| format!("bad hex '{t}'"))?
                } else {
                    t.parse::<f64>().map_err(|_| format!("bad number '{t}'"))?
                };
                out.push(Tok::Num(v));
                i = j;
                continue;
            }
            _ if ch.is_alphabetic() || ch == '_' => {
                let mut j = i + 1;
                while j < c.len() && (c[j].is_alphanumeric() || c[j] == '_' || c[j] == '.') {
                    j += 1;
                }
                let t: String = c[i..j].iter().collect();
                match t.as_str() {
                    "in" => out.push(Tok::In),
                    "and" => out.push(Tok::And),
                    "or" => out.push(Tok::Or),
                    "not" => out.push(Tok::Not),
                    "contains" => out.push(Tok::Op(Op::Contains)),
                    _ => out.push(Tok::Ident(t)),
                }
                i = j;
                continue;
            }
            _ => return Err(format!("unexpected character '{ch}'")),
        }
        i += 1;
    }
    Ok(out)
}

struct P {
    t: Vec<Tok>,
    i: usize,
}

impl P {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.t.get(self.i).cloned();
        self.i += 1;
        t
    }
    fn or(&mut self) -> Result<Expr, String> {
        let mut l = self.and()?;
        while self.peek() == Some(&Tok::Or) {
            self.i += 1;
            l = Expr::Or(Box::new(l), Box::new(self.and()?));
        }
        Ok(l)
    }
    fn and(&mut self) -> Result<Expr, String> {
        let mut l = self.unary()?;
        while self.peek() == Some(&Tok::And) {
            self.i += 1;
            l = Expr::And(Box::new(l), Box::new(self.unary()?));
        }
        Ok(l)
    }
    fn lit(&mut self) -> Result<Lit, String> {
        match self.next() {
            Some(Tok::Num(n)) => Ok(Lit::Num(n)),
            Some(Tok::Str(s)) => Ok(Lit::Str(s)),
            Some(Tok::Ident(s)) => Ok(Lit::Str(s)),
            t => Err(format!("expected literal, got {t:?}")),
        }
    }
    fn unary(&mut self) -> Result<Expr, String> {
        match self.next() {
            Some(Tok::Not) => Ok(Expr::Not(Box::new(self.unary()?))),
            Some(Tok::LParen) => {
                let e = self.or()?;
                if self.next() != Some(Tok::RParen) {
                    return Err("expected ')'".into());
                }
                Ok(e)
            }
            Some(Tok::Ident(id)) => match self.peek().cloned() {
                Some(Tok::Op(op)) => {
                    self.i += 1;
                    Ok(Expr::Cmp(id, op, self.lit()?))
                }
                Some(Tok::In) => {
                    self.i += 1;
                    if self.next() != Some(Tok::LBrack) {
                        return Err("expected '['".into());
                    }
                    let mut v = vec![self.lit()?];
                    while self.peek() == Some(&Tok::Comma) {
                        self.i += 1;
                        v.push(self.lit()?);
                    }
                    if self.next() != Some(Tok::RBrack) {
                        return Err("expected ']'".into());
                    }
                    Ok(Expr::In(id, v))
                }
                _ => Ok(Expr::Truthy(id)),
            },
            // Bare number: shorthand for pgn == N
            Some(Tok::Num(n)) => Ok(Expr::Cmp("pgn".into(), Op::Eq, Lit::Num(n))),
            t => Err(format!("unexpected token {t:?}")),
        }
    }
}

pub fn parse(s: &str) -> Result<Option<Expr>, String> {
    let toks = lex(s)?;
    if toks.is_empty() {
        return Ok(None);
    }
    let mut p = P { t: toks, i: 0 };
    let e = p.or()?;
    if p.i != p.t.len() {
        return Err(format!("unexpected trailing input at token {}", p.i + 1));
    }
    Ok(Some(e))
}

/// Value provider for evaluation.
pub trait Context {
    fn get(&self, ident: &str) -> Option<Lit>;
}

fn cmp(a: &Lit, op: &Op, b: &Lit) -> bool {
    match (a, b) {
        (Lit::Num(x), Lit::Num(y)) => match op {
            Op::Eq => (x - y).abs() < 1e-9,
            Op::Ne => (x - y).abs() >= 1e-9,
            Op::Lt => x < y,
            Op::Gt => x > y,
            Op::Le => x <= y,
            Op::Ge => x >= y,
            Op::Contains => x.to_string().contains(&y.to_string()),
        },
        _ => {
            let x = match a {
                Lit::Num(n) => n.to_string(),
                Lit::Str(s) => s.to_lowercase(),
            };
            let y = match b {
                Lit::Num(n) => n.to_string(),
                Lit::Str(s) => s.to_lowercase(),
            };
            match op {
                Op::Eq => x == y,
                Op::Ne => x != y,
                Op::Lt => x < y,
                Op::Gt => x > y,
                Op::Le => x <= y,
                Op::Ge => x >= y,
                Op::Contains => x.contains(&y),
            }
        }
    }
}

pub fn eval(e: &Expr, ctx: &dyn Context) -> bool {
    match e {
        Expr::And(a, b) => eval(a, ctx) && eval(b, ctx),
        Expr::Or(a, b) => eval(a, ctx) || eval(b, ctx),
        Expr::Not(a) => !eval(a, ctx),
        Expr::Cmp(id, op, lit) => match ctx.get(id) {
            Some(v) => cmp(&v, op, lit),
            None => matches!(op, Op::Ne),
        },
        Expr::In(id, list) => ctx.get(id).map(|v| list.iter().any(|l| cmp(&v, &Op::Eq, l))).unwrap_or(false),
        Expr::Truthy(id) => match ctx.get(id) {
            Some(Lit::Num(n)) => n != 0.0,
            Some(Lit::Str(s)) => !s.is_empty(),
            None => false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct M(HashMap<&'static str, Lit>);
    impl Context for M {
        fn get(&self, id: &str) -> Option<Lit> {
            self.0.get(id).cloned()
        }
    }

    fn ctx() -> M {
        M(HashMap::from([
            ("pgn", Lit::Num(127501.0)),
            ("src", Lit::Num(5.0)),
            ("name", Lit::Str("Binary Switch Bank Status".into())),
            ("tx", Lit::Num(0.0)),
        ]))
    }

    #[test]
    fn spec_example() {
        let e = parse("pgn == 127501 && src == 0x05").unwrap().unwrap();
        assert!(eval(&e, &ctx()));
    }

    #[test]
    fn misc() {
        let c = ctx();
        let t = |s: &str| eval(&parse(s).unwrap().unwrap(), &c);
        assert!(t("pgn in [127250, 127501]"));
        assert!(t("name ~ \"switch\""));
        assert!(t("!(src > 10) and not tx"));
        assert!(!t("pgn != 127501 || src >= 6"));
        assert!(t("127501"));
        assert!(parse("").unwrap().is_none());
        assert!(parse("pgn ==").is_err());
        assert!(parse("(pgn == 1").is_err());
    }
}
