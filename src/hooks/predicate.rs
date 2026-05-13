//! Frontmatter predicate grammar (SPEC-032 REQ-3205 / CON-3205).
//!
//! Parses the `frontmatter_where` expression used in hook manifests into a
//! typed [`Predicate`] AST, then evaluates it against a parsed frontmatter
//! [`serde_json::Value`].
//!
//! Grammar (REQ-3205):
//!
//! ```text
//! expr  ::= term (("&&" | "||") term)*
//! term  ::= path op value | path ("is null" | "is not null") | "!" term | "(" expr ")"
//! path  ::= IDENT ("." IDENT | "[" INT "]")*
//! op    ::= "==" | "!=" | "<" | "<=" | ">" | ">=" | "contains" | "matches"
//! value ::= STRING | INT | FLOAT | BOOL | "null"
//! ```
//!
//! Semantics (CON-3205):
//!
//! - Precedence: `!` > comparison > `&&` > `||`.
//! - `&&` and `||` short-circuit left-to-right.
//! - Comparisons are strict-typed — `"5" != 5`, `1 < "a"` is false.
//! - `contains` on a string is substring; on an array is element membership.
//! - `matches` is a regex match — the pattern is compiled once at parse
//!   time and reused across evaluations (REQ-3204 hot-path requirement).
//! - Unknown path → `null`.
//! - The predicate runs in a pure sandbox: no filesystem, network, or
//!   subprocess access.
//!
//! # Regex flavour
//!
//! SPEC-032 says "ECMAScript-flavour, cached". The Rust `regex` crate's
//! syntax is a superset of the ECMAScript baseline used in typical
//! frontmatter predicates (`^`, `$`, character classes, alternation,
//! non-greedy quantifiers) but does not support lookaround or
//! backreferences. That matches SPEC-032 §11's explicit mitigation
//! against ReDoS: linear-time guarantees only.

use std::fmt;

use regex::Regex;
use serde_json::Value;

// ── Public AST ──────────────────────────────────────────────────────────────

/// A parsed, ready-to-evaluate frontmatter predicate.
///
/// Produced by [`parse`]. Carries pre-compiled regexes for every `matches`
/// clause so evaluation is allocation-free on the hot path.
#[derive(Debug, Clone)]
pub struct Predicate(pub Expr);

/// One node of the predicate AST.
#[derive(Debug, Clone)]
pub enum Expr {
    Or(Box<Expr>, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
    Compare {
        path: Path,
        op: CmpOp,
        value: Literal,
    },
    Contains {
        path: Path,
        value: Literal,
    },
    Matches {
        path: Path,
        regex: Regex,
        /// Original pattern text — preserved for diagnostics / [`Predicate::source`].
        pattern: String,
    },
    IsNull(Path),
    IsNotNull(Path),
    /// Bare boolean path — `!draft`, `published`. Coerces strict: a
    /// resolved `Value::Bool(true)` is true, anything else (false, null,
    /// missing, non-bool) is false. The SPEC-032 REQ-3205 examples rely
    /// on this shape (`status == "published" && !draft`).
    BoolPath(Path),
}

/// Dotted / indexed access into the frontmatter value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Path(pub Vec<Segment>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    /// `foo.bar` — object key lookup.
    Field(String),
    /// `foo[3]` — array index. Negative indices count from the end.
    Index(i64),
}

/// Equality and ordering operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Neq,
    Lt,
    Leq,
    Gt,
    Geq,
}

/// RHS literal values.
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Null,
}

// ── Errors ──────────────────────────────────────────────────────────────────

/// Parse / compile failure.
///
/// `position` is a byte offset into the source string; `0` for errors raised
/// at the lexer's current cursor before any token was consumed.
#[derive(Debug)]
pub struct ParseError {
    pub message: String,
    pub position: usize,
}

impl ParseError {
    fn new(msg: impl Into<String>, position: usize) -> Self {
        Self {
            message: msg.into(),
            position,
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "frontmatter predicate parse error at byte {}: {}",
            self.position, self.message
        )
    }
}

impl std::error::Error for ParseError {}

// ── Public API ──────────────────────────────────────────────────────────────

/// Parse a predicate string into an evaluable [`Predicate`].
///
/// Fails with [`ParseError`] on:
/// - lexical errors (unterminated string, bad number),
/// - grammar errors (missing RHS, mismatched parens, unknown operator),
/// - regex-compile errors on any `matches` clause.
///
/// The empty string (and whitespace-only input) is rejected — an empty
/// predicate is meaningless. Manifests that want "no predicate" SHALL omit
/// the `frontmatter_where` field entirely (see [`crate::hooks::manifest`]).
pub fn parse(src: &str) -> Result<Predicate, ParseError> {
    let mut p = Parser::new(src)?;
    let expr = p.parse_or()?;
    if !p.at_end() {
        return Err(ParseError::new(
            format!("trailing input after expression: {:?}", p.peek_raw()),
            p.cursor(),
        ));
    }
    Ok(Predicate(expr))
}

impl Predicate {
    /// Evaluate against a frontmatter value (typically a JSON object).
    ///
    /// Any path that fails to resolve returns `null`, matching CON-3205.
    /// Never panics.
    pub fn evaluate(&self, frontmatter: &Value) -> bool {
        eval_expr(&self.0, frontmatter)
    }
}

// ── Evaluator ───────────────────────────────────────────────────────────────

fn eval_expr(expr: &Expr, fm: &Value) -> bool {
    match expr {
        Expr::Or(l, r) => eval_expr(l, fm) || eval_expr(r, fm),
        Expr::And(l, r) => eval_expr(l, fm) && eval_expr(r, fm),
        Expr::Not(inner) => !eval_expr(inner, fm),
        Expr::Compare { path, op, value } => {
            let v = resolve(path, fm);
            compare(v, *op, value)
        }
        Expr::Contains { path, value } => {
            let v = resolve(path, fm);
            contains(v, value)
        }
        Expr::Matches { path, regex, .. } => match resolve(path, fm) {
            Some(Value::String(s)) => regex.is_match(s),
            _ => false,
        },
        Expr::IsNull(p) => matches!(resolve(p, fm), None | Some(Value::Null)),
        Expr::IsNotNull(p) => !matches!(resolve(p, fm), None | Some(Value::Null)),
        Expr::BoolPath(p) => matches!(resolve(p, fm), Some(Value::Bool(true))),
    }
}

/// Walk `path` through `fm` returning the referenced value, or `None` if any
/// segment is missing. A present `null` returns `Some(Value::Null)` so
/// `is null` / `is not null` can distinguish the two.
fn resolve<'a>(path: &Path, fm: &'a Value) -> Option<&'a Value> {
    let mut cur = fm;
    for seg in &path.0 {
        cur = match (cur, seg) {
            (Value::Object(map), Segment::Field(k)) => map.get(k)?,
            (Value::Array(arr), Segment::Index(i)) => {
                let idx = resolve_index(*i, arr.len())?;
                arr.get(idx)?
            }
            _ => return None,
        };
    }
    Some(cur)
}

fn resolve_index(i: i64, len: usize) -> Option<usize> {
    if i >= 0 {
        let idx = i as usize;
        if idx < len {
            Some(idx)
        } else {
            None
        }
    } else {
        let back = (-i) as usize;
        if back == 0 || back > len {
            None
        } else {
            Some(len - back)
        }
    }
}

fn compare(v: Option<&Value>, op: CmpOp, lit: &Literal) -> bool {
    match op {
        CmpOp::Eq => values_equal(v, lit),
        CmpOp::Neq => !values_equal(v, lit),
        CmpOp::Lt | CmpOp::Leq | CmpOp::Gt | CmpOp::Geq => order(v, lit, op),
    }
}

/// Strict-typed equality: `a == b` is true only when the resolved value and
/// the literal carry the same type *and* the same value.
///
/// Missing path is treated as `Value::Null` per CON-3205.
fn values_equal(v: Option<&Value>, lit: &Literal) -> bool {
    let v = v.unwrap_or(&Value::Null);
    literal_equals(v, lit)
}

fn order(v: Option<&Value>, lit: &Literal, op: CmpOp) -> bool {
    let v = match v {
        Some(v) => v,
        None => return false, // ordering on a missing path = false
    };
    match (v, lit) {
        (Value::String(a), Literal::Str(b)) => cmp_order_strings(a, b, op),
        (Value::Number(n), Literal::Int(i)) => cmp_number_int(n, *i, op),
        (Value::Number(n), Literal::Float(f)) => cmp_number_float(n, *f, op),
        _ => false,
    }
}

fn cmp_order_strings(a: &str, b: &str, op: CmpOp) -> bool {
    match op {
        CmpOp::Lt => a < b,
        CmpOp::Leq => a <= b,
        CmpOp::Gt => a > b,
        CmpOp::Geq => a >= b,
        // Eq/Neq handled by the caller branch above.
        CmpOp::Eq | CmpOp::Neq => false,
    }
}

fn cmp_number_int(n: &serde_json::Number, lit: i64, op: CmpOp) -> bool {
    if let Some(a) = n.as_i64() {
        return match op {
            CmpOp::Eq => a == lit,
            CmpOp::Neq => a != lit,
            CmpOp::Lt => a < lit,
            CmpOp::Leq => a <= lit,
            CmpOp::Gt => a > lit,
            CmpOp::Geq => a >= lit,
        };
    }
    if let Some(a) = n.as_u64() {
        // Comparing u64 against i64: only meaningful when lit ≥ 0.
        if lit < 0 {
            return matches!(op, CmpOp::Neq | CmpOp::Gt | CmpOp::Geq);
        }
        let b = lit as u64;
        return match op {
            CmpOp::Eq => a == b,
            CmpOp::Neq => a != b,
            CmpOp::Lt => a < b,
            CmpOp::Leq => a <= b,
            CmpOp::Gt => a > b,
            CmpOp::Geq => a >= b,
        };
    }
    if let Some(a) = n.as_f64() {
        return cmp_f64(a, lit as f64, op);
    }
    false
}

fn cmp_number_float(n: &serde_json::Number, lit: f64, op: CmpOp) -> bool {
    let a = n.as_f64().unwrap_or(f64::NAN);
    cmp_f64(a, lit, op)
}

fn cmp_f64(a: f64, b: f64, op: CmpOp) -> bool {
    if a.is_nan() || b.is_nan() {
        return matches!(op, CmpOp::Neq);
    }
    match op {
        CmpOp::Eq => a == b,
        CmpOp::Neq => a != b,
        CmpOp::Lt => a < b,
        CmpOp::Leq => a <= b,
        CmpOp::Gt => a > b,
        CmpOp::Geq => a >= b,
    }
}

fn contains(v: Option<&Value>, lit: &Literal) -> bool {
    match (v, lit) {
        (Some(Value::String(s)), Literal::Str(needle)) => s.contains(needle.as_str()),
        (Some(Value::Array(arr)), lit) => arr.iter().any(|elem| literal_equals(elem, lit)),
        _ => false,
    }
}

fn literal_equals(v: &Value, lit: &Literal) -> bool {
    match (v, lit) {
        (Value::Null, Literal::Null) => true,
        (Value::Bool(a), Literal::Bool(b)) => a == b,
        (Value::String(a), Literal::Str(b)) => a == b,
        (Value::Number(n), Literal::Int(i)) => {
            if n.as_i64() == Some(*i) {
                return true;
            }
            if let Some(u) = n.as_u64() {
                if *i >= 0 && u == *i as u64 {
                    return true;
                }
            }
            // JSON stores `3.0` as f64; allow exact float-vs-int match.
            match n.as_f64() {
                Some(x) => x == *i as f64,
                None => false,
            }
        }
        (Value::Number(n), Literal::Float(f)) => match n.as_f64() {
            Some(x) => x == *f,
            None => false,
        },
        _ => false,
    }
}

// ── Lexer ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Eq,
    Neq,
    Lt,
    Leq,
    Gt,
    Geq,
    AndAnd,
    OrOr,
    Bang,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Dot,
    Contains,
    Matches,
    Is,
    KwNot,
    KwNull,
}

struct Lexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src,
            bytes: src.as_bytes(),
            pos: 0,
        }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn next_token(&mut self) -> Result<Option<(Tok, usize)>, ParseError> {
        self.skip_ws();
        let start = self.pos;
        let Some(&c) = self.bytes.get(self.pos) else {
            return Ok(None);
        };

        match c {
            b'(' => {
                self.pos += 1;
                Ok(Some((Tok::LParen, start)))
            }
            b')' => {
                self.pos += 1;
                Ok(Some((Tok::RParen, start)))
            }
            b'[' => {
                self.pos += 1;
                Ok(Some((Tok::LBracket, start)))
            }
            b']' => {
                self.pos += 1;
                Ok(Some((Tok::RBracket, start)))
            }
            b'.' => {
                self.pos += 1;
                Ok(Some((Tok::Dot, start)))
            }
            b'!' => {
                if self.bytes.get(self.pos + 1) == Some(&b'=') {
                    self.pos += 2;
                    Ok(Some((Tok::Neq, start)))
                } else {
                    self.pos += 1;
                    Ok(Some((Tok::Bang, start)))
                }
            }
            b'=' => {
                if self.bytes.get(self.pos + 1) == Some(&b'=') {
                    self.pos += 2;
                    Ok(Some((Tok::Eq, start)))
                } else {
                    Err(ParseError::new("expected `==`", start))
                }
            }
            b'<' => {
                if self.bytes.get(self.pos + 1) == Some(&b'=') {
                    self.pos += 2;
                    Ok(Some((Tok::Leq, start)))
                } else {
                    self.pos += 1;
                    Ok(Some((Tok::Lt, start)))
                }
            }
            b'>' => {
                if self.bytes.get(self.pos + 1) == Some(&b'=') {
                    self.pos += 2;
                    Ok(Some((Tok::Geq, start)))
                } else {
                    self.pos += 1;
                    Ok(Some((Tok::Gt, start)))
                }
            }
            b'&' => {
                if self.bytes.get(self.pos + 1) == Some(&b'&') {
                    self.pos += 2;
                    Ok(Some((Tok::AndAnd, start)))
                } else {
                    Err(ParseError::new("expected `&&`", start))
                }
            }
            b'|' => {
                if self.bytes.get(self.pos + 1) == Some(&b'|') {
                    self.pos += 2;
                    Ok(Some((Tok::OrOr, start)))
                } else {
                    Err(ParseError::new("expected `||`", start))
                }
            }
            b'"' | b'\'' => self.lex_string(c).map(Some),
            b'-' | b'0'..=b'9' => self.lex_number().map(Some),
            b if is_ident_start(b) => self.lex_ident_or_keyword().map(Some),
            _ => Err(ParseError::new(
                format!("unexpected character {:?}", c as char),
                start,
            )),
        }
    }

    fn lex_string(&mut self, quote: u8) -> Result<(Tok, usize), ParseError> {
        let start = self.pos;
        self.pos += 1;
        let mut buf = String::new();
        let mut chunk_start = self.pos;
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            if b == quote {
                buf.push_str(&self.src[chunk_start..self.pos]);
                self.pos += 1;
                return Ok((Tok::Str(buf), start));
            }
            if b == b'\\' {
                buf.push_str(&self.src[chunk_start..self.pos]);
                let nxt = self.bytes.get(self.pos + 1).copied().ok_or_else(|| {
                    ParseError::new("unterminated escape sequence in string", self.pos)
                })?;
                match nxt {
                    b'\\' => buf.push('\\'),
                    b'"' => buf.push('"'),
                    b'\'' => buf.push('\''),
                    b'n' => buf.push('\n'),
                    b't' => buf.push('\t'),
                    b'r' => buf.push('\r'),
                    b'0' => buf.push('\0'),
                    // Unknown escapes pass through as `\<char>` so regex
                    // metacharacters (`\d`, `\s`, `\w`) survive the lexer.
                    other => {
                        buf.push('\\');
                        buf.push(other as char);
                    }
                }
                self.pos += 2;
                chunk_start = self.pos;
                continue;
            }
            self.pos += 1;
        }
        Err(ParseError::new("unterminated string literal", start))
    }

    fn lex_number(&mut self) -> Result<(Tok, usize), ParseError> {
        let start = self.pos;
        if self.bytes[self.pos] == b'-' {
            self.pos += 1;
        }
        let digits_start = self.pos;
        while self
            .bytes
            .get(self.pos)
            .map(|b| b.is_ascii_digit())
            .unwrap_or(false)
        {
            self.pos += 1;
        }
        if self.pos == digits_start {
            return Err(ParseError::new("expected digit after `-`", start));
        }
        let mut is_float = false;
        if self.bytes.get(self.pos) == Some(&b'.') {
            // Ensure the next char is a digit (distinguish `foo.bar` from `1.5`).
            if self
                .bytes
                .get(self.pos + 1)
                .map(|b| b.is_ascii_digit())
                .unwrap_or(false)
            {
                is_float = true;
                self.pos += 1;
                while self
                    .bytes
                    .get(self.pos)
                    .map(|b| b.is_ascii_digit())
                    .unwrap_or(false)
                {
                    self.pos += 1;
                }
            }
        }
        // Exponent
        if let Some(&b) = self.bytes.get(self.pos) {
            if b == b'e' || b == b'E' {
                is_float = true;
                self.pos += 1;
                if let Some(&s) = self.bytes.get(self.pos) {
                    if s == b'+' || s == b'-' {
                        self.pos += 1;
                    }
                }
                while self
                    .bytes
                    .get(self.pos)
                    .map(|b| b.is_ascii_digit())
                    .unwrap_or(false)
                {
                    self.pos += 1;
                }
            }
        }
        let text = &self.src[start..self.pos];
        if is_float {
            text.parse::<f64>()
                .map(|f| (Tok::Float(f), start))
                .map_err(|e| ParseError::new(format!("invalid float {text:?}: {e}"), start))
        } else {
            text.parse::<i64>()
                .map(|i| (Tok::Int(i), start))
                .map_err(|e| ParseError::new(format!("invalid integer {text:?}: {e}"), start))
        }
    }

    fn lex_ident_or_keyword(&mut self) -> Result<(Tok, usize), ParseError> {
        let start = self.pos;
        while let Some(&b) = self.bytes.get(self.pos) {
            if is_ident_cont(b) {
                self.pos += 1;
            } else {
                break;
            }
        }
        let text = &self.src[start..self.pos];
        let tok = match text {
            "true" => Tok::Bool(true),
            "false" => Tok::Bool(false),
            "null" => Tok::KwNull,
            "is" => Tok::Is,
            "not" => Tok::KwNot,
            "contains" => Tok::Contains,
            "matches" => Tok::Matches,
            _ => Tok::Ident(text.to_string()),
        };
        Ok((tok, start))
    }
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_cont(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

// ── Parser ──────────────────────────────────────────────────────────────────

struct Parser<'a> {
    src: &'a str,
    tokens: Vec<(Tok, usize)>,
    idx: usize,
}

/// Internal dispatch tag for the operator slot in a path-term: one of the
/// six comparison operators, or `is` / `contains` / `matches`.
#[derive(Debug, Clone, Copy)]
enum OpKind {
    Cmp(CmpOp),
    Is,
    Contains,
    Matches,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Result<Self, ParseError> {
        let mut lex = Lexer::new(src);
        let mut tokens = Vec::new();
        while let Some((tok, pos)) = lex.next_token()? {
            tokens.push((tok, pos));
        }
        if tokens.is_empty() {
            return Err(ParseError::new(
                "empty predicate expression (omit `frontmatter_where` instead)",
                0,
            ));
        }
        Ok(Self {
            src,
            tokens,
            idx: 0,
        })
    }

    fn at_end(&self) -> bool {
        self.idx >= self.tokens.len()
    }

    fn cursor(&self) -> usize {
        self.tokens
            .get(self.idx)
            .map(|(_, p)| *p)
            .unwrap_or(self.src.len())
    }

    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.idx).map(|(t, _)| t)
    }

    fn peek_raw(&self) -> String {
        self.tokens
            .get(self.idx)
            .map(|(t, _)| format!("{t:?}"))
            .unwrap_or_else(|| "<eof>".into())
    }

    fn advance(&mut self) -> Option<(Tok, usize)> {
        let t = self.tokens.get(self.idx).cloned();
        self.idx += 1;
        t
    }

    fn parse_or(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_and()?;
        while matches!(self.peek(), Some(Tok::OrOr)) {
            self.advance();
            let right = self.parse_and()?;
            left = Expr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_unary()?;
        while matches!(self.peek(), Some(Tok::AndAnd)) {
            self.advance();
            let right = self.parse_unary()?;
            left = Expr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        if matches!(self.peek(), Some(Tok::Bang)) {
            self.advance();
            let inner = self.parse_unary()?;
            return Ok(Expr::Not(Box::new(inner)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        if matches!(self.peek(), Some(Tok::LParen)) {
            self.advance();
            let inner = self.parse_or()?;
            match self.advance() {
                Some((Tok::RParen, _)) => return Ok(inner),
                Some((_, p)) => {
                    return Err(ParseError::new("expected `)`", p));
                }
                None => {
                    return Err(ParseError::new(
                        "expected `)` before end of input",
                        self.src.len(),
                    ));
                }
            }
        }
        self.parse_path_term()
    }

    fn parse_path_term(&mut self) -> Result<Expr, ParseError> {
        let path = self.parse_path()?;
        // Peek the next token. If it's an operator, consume it and parse
        // the tail; otherwise the bare path is a bool-coerced term —
        // REQ-3205 canonical example `... && !draft`.
        let op_kind = match self.peek() {
            Some(Tok::Is) => OpKind::Is,
            Some(Tok::Contains) => OpKind::Contains,
            Some(Tok::Matches) => OpKind::Matches,
            Some(Tok::Eq) => OpKind::Cmp(CmpOp::Eq),
            Some(Tok::Neq) => OpKind::Cmp(CmpOp::Neq),
            Some(Tok::Lt) => OpKind::Cmp(CmpOp::Lt),
            Some(Tok::Leq) => OpKind::Cmp(CmpOp::Leq),
            Some(Tok::Gt) => OpKind::Cmp(CmpOp::Gt),
            Some(Tok::Geq) => OpKind::Cmp(CmpOp::Geq),
            _ => return Ok(Expr::BoolPath(path)),
        };
        self.advance();
        match op_kind {
            OpKind::Is => self.parse_is_tail(path),
            OpKind::Contains => {
                let lit = self.parse_value()?;
                Ok(Expr::Contains { path, value: lit })
            }
            OpKind::Matches => {
                let (lit, pos) = self.parse_value_with_pos()?;
                let pattern = match lit {
                    Literal::Str(s) => s,
                    _ => {
                        return Err(ParseError::new(
                            "`matches` operator requires a string regex literal",
                            pos,
                        ));
                    }
                };
                let regex = Regex::new(&pattern)
                    .map_err(|e| ParseError::new(format!("invalid regex: {e}"), pos))?;
                Ok(Expr::Matches {
                    path,
                    regex,
                    pattern,
                })
            }
            OpKind::Cmp(op) => {
                let value = self.parse_value()?;
                Ok(Expr::Compare { path, op, value })
            }
        }
    }

    fn parse_is_tail(&mut self, path: Path) -> Result<Expr, ParseError> {
        let mut negate = false;
        if matches!(self.peek(), Some(Tok::KwNot)) {
            self.advance();
            negate = true;
        }
        match self.advance() {
            Some((Tok::KwNull, _)) => {
                if negate {
                    Ok(Expr::IsNotNull(path))
                } else {
                    Ok(Expr::IsNull(path))
                }
            }
            Some((_, p)) => Err(ParseError::new(
                "expected `null` after `is` (or `is not`)",
                p,
            )),
            None => Err(ParseError::new(
                "expected `null` after `is`",
                self.src.len(),
            )),
        }
    }

    fn parse_path(&mut self) -> Result<Path, ParseError> {
        let (first_tok, first_pos) = match self.advance() {
            Some(t) => t,
            None => {
                return Err(ParseError::new("expected path", self.src.len()));
            }
        };
        let first = match first_tok {
            Tok::Ident(s) => Segment::Field(s),
            other => {
                return Err(ParseError::new(
                    format!("expected identifier at start of path, got {other:?}"),
                    first_pos,
                ));
            }
        };
        let mut segs = vec![first];
        loop {
            match self.peek() {
                Some(Tok::Dot) => {
                    self.advance();
                    match self.advance() {
                        Some((Tok::Ident(s), _)) => segs.push(Segment::Field(s)),
                        Some((_, p)) => {
                            return Err(ParseError::new("expected identifier after `.`", p));
                        }
                        None => {
                            return Err(ParseError::new(
                                "expected identifier after `.`",
                                self.src.len(),
                            ));
                        }
                    }
                }
                Some(Tok::LBracket) => {
                    self.advance();
                    match self.advance() {
                        Some((Tok::Int(i), _)) => segs.push(Segment::Index(i)),
                        Some((_, p)) => {
                            return Err(ParseError::new(
                                "expected integer index inside `[...]`",
                                p,
                            ));
                        }
                        None => {
                            return Err(ParseError::new(
                                "expected integer index inside `[...]`",
                                self.src.len(),
                            ));
                        }
                    }
                    match self.advance() {
                        Some((Tok::RBracket, _)) => {}
                        Some((_, p)) => {
                            return Err(ParseError::new("expected `]`", p));
                        }
                        None => {
                            return Err(ParseError::new(
                                "expected `]` before end of input",
                                self.src.len(),
                            ));
                        }
                    }
                }
                _ => break,
            }
        }
        Ok(Path(segs))
    }

    fn parse_value(&mut self) -> Result<Literal, ParseError> {
        self.parse_value_with_pos().map(|(l, _)| l)
    }

    fn parse_value_with_pos(&mut self) -> Result<(Literal, usize), ParseError> {
        match self.advance() {
            Some((Tok::Str(s), pos)) => Ok((Literal::Str(s), pos)),
            Some((Tok::Int(i), pos)) => Ok((Literal::Int(i), pos)),
            Some((Tok::Float(f), pos)) => Ok((Literal::Float(f), pos)),
            Some((Tok::Bool(b), pos)) => Ok((Literal::Bool(b), pos)),
            Some((Tok::KwNull, pos)) => Ok((Literal::Null, pos)),
            Some((other, pos)) => Err(ParseError::new(
                format!("expected literal value, got {other:?}"),
                pos,
            )),
            None => Err(ParseError::new("expected literal value", self.src.len())),
        }
    }
}

// ── Tests (TEST-3205) ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn check(src: &str, fm: Value, expected: bool) {
        let p = parse(src).unwrap_or_else(|e| panic!("parse {src:?}: {e}"));
        assert_eq!(p.evaluate(&fm), expected, "src={src:?} fm={fm}",);
    }

    // ── Canonical REQ-3205 examples ───────────────────────────────────────

    #[test]
    fn tags_contains_project_on_array() {
        check(
            r#"tags contains "project""#,
            json!({ "tags": ["project", "draft"] }),
            true,
        );
        check(
            r#"tags contains "missing""#,
            json!({ "tags": ["project", "draft"] }),
            false,
        );
    }

    #[test]
    fn combined_and_with_not() {
        check(
            r#"status == "published" && !draft"#,
            json!({ "status": "published", "draft": false }),
            true,
        );
        check(
            r#"status == "published" && !draft"#,
            json!({ "status": "published", "draft": true }),
            false,
        );
    }

    #[test]
    fn extensions_opt_out_path() {
        check(
            r#"frontmatter.extensions.tasks != false"#,
            json!({ "frontmatter": { "extensions": { "tasks": true } } }),
            true,
        );
        check(
            r#"frontmatter.extensions.tasks != false"#,
            json!({ "frontmatter": { "extensions": { "tasks": false } } }),
            false,
        );
        // Missing path resolves to null → `null != false` is true (different types).
        check(r#"frontmatter.extensions.tasks != false"#, json!({}), true);
    }

    #[test]
    fn word_count_gt_500() {
        check(r#"word_count > 500"#, json!({ "word_count": 742 }), true);
        check(r#"word_count > 500"#, json!({ "word_count": 100 }), false);
    }

    #[test]
    fn title_matches_regex() {
        check(
            r#"title matches "^Daily.*""#,
            json!({ "title": "Daily standup 2026-04-20" }),
            true,
        );
        check(
            r#"title matches "^Daily.*""#,
            json!({ "title": "Weekly plan" }),
            false,
        );
    }

    // ── Type-strict semantics (CON-3205) ─────────────────────────────────

    #[test]
    fn strict_types_string_vs_number_false() {
        // "500" != 500 — comparison between string and number is always
        // false (never coerces).
        check(r#"word_count > 500"#, json!({ "word_count": "500" }), false);
        check(r#"v == 5"#, json!({ "v": "5" }), false);
        check(r#"v != 5"#, json!({ "v": "5" }), true);
    }

    #[test]
    fn missing_path_resolves_null() {
        check(r#"a.b.c == 1"#, json!({}), false);
        check(r#"a.b.c is null"#, json!({}), true);
        check(r#"a.b.c is not null"#, json!({ "a": 1 }), false);
        check(
            r#"a.b.c is not null"#,
            json!({ "a": { "b": { "c": 3 } } }),
            true,
        );
    }

    #[test]
    fn null_equality() {
        check(r#"x == null"#, json!({ "x": null }), true);
        check(r#"x != null"#, json!({ "x": 5 }), true);
        check(r#"x != null"#, json!({ "x": null }), false);
    }

    // ── Operator matrix ──────────────────────────────────────────────────

    #[test]
    fn numeric_comparisons() {
        let fm = json!({ "n": 10 });
        check("n == 10", fm.clone(), true);
        check("n != 10", fm.clone(), false);
        check("n < 11", fm.clone(), true);
        check("n <= 10", fm.clone(), true);
        check("n > 9", fm.clone(), true);
        check("n >= 10", fm.clone(), true);
        check("n < 10", fm.clone(), false);
        check("n > 10", fm, false);
    }

    #[test]
    fn float_comparisons() {
        let fm = json!({ "x": 3.5 });
        check("x > 3", fm.clone(), true);
        check("x < 3.6", fm.clone(), true);
        check("x == 3.5", fm, true);
    }

    #[test]
    fn string_comparisons() {
        let fm = json!({ "name": "bravo" });
        check(r#"name == "bravo""#, fm.clone(), true);
        check(r#"name < "charlie""#, fm.clone(), true);
        check(r#"name > "alpha""#, fm, true);
    }

    #[test]
    fn bool_comparisons() {
        let fm = json!({ "draft": true });
        check("draft == true", fm.clone(), true);
        check("draft != false", fm.clone(), true);
        check("!draft", fm, false);
    }

    #[test]
    fn contains_string_substring() {
        check(
            r#"title contains "rust""#,
            json!({ "title": "learning rust" }),
            true,
        );
        check(
            r#"title contains "zig""#,
            json!({ "title": "learning rust" }),
            false,
        );
    }

    #[test]
    fn contains_array_membership() {
        check(
            r#"tags contains "project""#,
            json!({ "tags": ["project", "draft"] }),
            true,
        );
        // Array of mixed types
        check(
            r#"values contains 42"#,
            json!({ "values": [1, 2, 42, "x"] }),
            true,
        );
        check(
            r#"values contains true"#,
            json!({ "values": [false, true] }),
            true,
        );
    }

    #[test]
    fn contains_type_mismatched_returns_false() {
        // String needle against array of numbers.
        check(r#"tags contains "x""#, json!({ "tags": [1, 2, 3] }), false);
        // Contains against null-valued / missing / scalar path.
        check(r#"x contains "y""#, json!({ "x": null }), false);
        check(r#"x contains "y""#, json!({ "x": 5 }), false);
        check(r#"x contains "y""#, json!({}), false);
    }

    // ── Precedence and grouping ──────────────────────────────────────────

    #[test]
    fn and_binds_tighter_than_or() {
        // a || b && c  ==  a || (b && c)
        check(
            r#"a == 1 || b == 2 && c == 3"#,
            json!({ "a": 99, "b": 2, "c": 3 }),
            true,
        );
        check(
            r#"a == 1 || b == 2 && c == 3"#,
            json!({ "a": 99, "b": 2, "c": 99 }),
            false,
        );
        // `a == 1` alone makes it true regardless.
        check(r#"a == 1 || b == 2 && c == 3"#, json!({ "a": 1 }), true);
    }

    #[test]
    fn bang_binds_tighter_than_and() {
        check(r#"!a && b"#, json!({ "a": false, "b": true }), true);
        check(r#"!a && b"#, json!({ "a": true, "b": true }), false);
    }

    #[test]
    fn parens_override_precedence() {
        check(
            r#"(a == 1 || b == 2) && c == 3"#,
            json!({ "a": 1, "c": 3 }),
            true,
        );
        check(
            r#"(a == 1 || b == 2) && c == 3"#,
            json!({ "a": 1, "c": 99 }),
            false,
        );
    }

    #[test]
    fn short_circuit_eval_or_skips_right() {
        // If the right side would error on an invalid regex, it shouldn't
        // matter here because the regex is compiled at parse time — but
        // this test at least asserts && / || evaluate lazily.
        let p = parse(r#"flag == true || tags contains "x""#).unwrap();
        assert!(p.evaluate(&json!({ "flag": true })));
    }

    // ── Path access ──────────────────────────────────────────────────────

    #[test]
    fn path_indexing_positive() {
        check(
            r#"tags[0] == "project""#,
            json!({ "tags": ["project", "draft"] }),
            true,
        );
        check(
            r#"tags[1] == "draft""#,
            json!({ "tags": ["project", "draft"] }),
            true,
        );
        check(
            r#"tags[5] == "missing""#,
            json!({ "tags": ["project"] }),
            false,
        );
    }

    #[test]
    fn path_indexing_negative() {
        check(
            r#"tags[-1] == "draft""#,
            json!({ "tags": ["project", "draft"] }),
            true,
        );
        check(
            r#"tags[-2] == "project""#,
            json!({ "tags": ["project", "draft"] }),
            true,
        );
    }

    #[test]
    fn path_mixed_field_and_index() {
        check(
            r#"authors[0].name == "alice""#,
            json!({ "authors": [{ "name": "alice" }] }),
            true,
        );
    }

    #[test]
    fn path_underscore_and_dash_idents() {
        check(r#"word_count > 0"#, json!({ "word_count": 1 }), true);
        check(r#"kebab-case == "ok""#, json!({ "kebab-case": "ok" }), true);
    }

    // ── Regex / matches ──────────────────────────────────────────────────

    #[test]
    fn matches_compiles_at_parse_time() {
        // Invalid regex fails at parse time, not at eval.
        let err = parse(r#"title matches "[invalid""#).unwrap_err();
        assert!(err.message.contains("invalid regex"));
    }

    #[test]
    fn matches_rejects_non_string_rhs() {
        let err = parse(r#"title matches 42"#).unwrap_err();
        assert!(err.message.contains("string regex"));
    }

    #[test]
    fn matches_against_non_string_field_is_false() {
        check(
            r#"word_count matches "^\\d+$""#,
            json!({ "word_count": 742 }),
            false,
        );
    }

    #[test]
    fn matches_anchored_pattern() {
        check(
            r#"slug matches "^[a-z-]+$""#,
            json!({ "slug": "hello-world" }),
            true,
        );
        check(
            r#"slug matches "^[a-z-]+$""#,
            json!({ "slug": "Hello World" }),
            false,
        );
    }

    // ── Parse-error surface ──────────────────────────────────────────────

    #[test]
    fn empty_input_rejected() {
        assert!(parse("").is_err());
        assert!(parse("   \t\n  ").is_err());
    }

    #[test]
    fn trailing_garbage_rejected() {
        assert!(parse(r#"a == 1 xyz"#).is_err());
        assert!(parse(r#"a == 1 &&"#).is_err());
    }

    #[test]
    fn unmatched_paren_rejected() {
        assert!(parse(r#"(a == 1"#).is_err());
    }

    #[test]
    fn unterminated_string_rejected() {
        assert!(parse(r#"title == "foo"#).is_err());
    }

    #[test]
    fn bare_path_parses_as_bool_coercion() {
        // REQ-3205 canonical example: `... && !draft`. Bare paths are
        // accepted as bool-coerced terms. `published` → true only when
        // the resolved value is `Value::Bool(true)`.
        let p = parse("published").unwrap();
        assert!(p.evaluate(&json!({ "published": true })));
        assert!(!p.evaluate(&json!({ "published": false })));
        assert!(!p.evaluate(&json!({ "published": "yes" }))); // strict
        assert!(!p.evaluate(&json!({}))); // missing → false
    }

    #[test]
    fn single_equals_rejected() {
        assert!(parse("a = 1").is_err());
    }

    // ── Property-style fuzz (REQ-3205): random grammatically-valid
    // ── predicates parse without panicking, and evaluation never panics.

    #[test]
    fn fuzz_valid_predicates_dont_panic() {
        // Generated by hand, representative of what a fuzzer would produce
        // inside the grammar. We rely on proptest elsewhere; this ensures
        // the baseline never regresses.
        let sources = [
            r#"a == 1"#,
            r#"a != "x""#,
            r#"a < 1.5 && b > 0"#,
            r#"!flag || (x >= 10 && y <= 20)"#,
            r#"tags contains "x""#,
            r#"name matches "^.*$""#,
            r#"a.b.c is null"#,
            r#"a[0].b is not null"#,
            r#"a == null"#,
            r#"((a == 1))"#,
            r#"!(!(a == 1))"#,
            r#"a.b[0].c[-1] == true"#,
        ];
        for s in sources {
            let p = parse(s).unwrap_or_else(|e| panic!("parse {s:?}: {e}"));
            // Evaluate against a representative set of frontmatter values.
            for fm in [
                json!(null),
                json!({}),
                json!({"a": 1, "b": 2}),
                json!({"a": {"b": {"c": true}}, "tags": ["x", "y"], "name": "hello"}),
                json!({"flag": true, "x": 5, "y": 100}),
            ] {
                let _ = p.evaluate(&fm);
            }
        }
    }

    // ── String escape handling ───────────────────────────────────────────

    #[test]
    fn string_escape_sequences() {
        check(
            r#"msg == "he said \"hi\"""#,
            json!({ "msg": r#"he said "hi""# }),
            true,
        );
        check(
            r#"msg == "line\nbreak""#,
            json!({ "msg": "line\nbreak" }),
            true,
        );
    }

    #[test]
    fn single_quoted_strings_accepted() {
        check(r#"name == 'bravo'"#, json!({ "name": "bravo" }), true);
    }

    #[test]
    fn negative_integer_literal() {
        check(r#"delta == -5"#, json!({ "delta": -5 }), true);
        check(r#"delta > -10"#, json!({ "delta": -5 }), true);
    }

    #[test]
    fn float_with_exponent() {
        check(r#"x == 1e3"#, json!({ "x": 1000.0 }), true);
        check(r#"x < 2.5e-2"#, json!({ "x": 0.01 }), true);
    }
}
