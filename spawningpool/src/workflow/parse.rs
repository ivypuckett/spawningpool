//! Tokenizer and recursive-descent parser for the workflow DSL
//! (workflow-dsl.md §5–6).

use super::ast::{AccessKey, BinOp, Expr, Statement, Workflow};
use crate::script::parse_params;
use crate::types::Param;

/// A parse error with a human-readable message.
#[derive(Debug, Clone, PartialEq)]
pub struct ParseError(pub String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for ParseError {}

// ── Tokenizer ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Ident(String),
    Str(String),
    Num(f64),
    Eq,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Dot,
    Colon,
    Comma,
    Bang,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    PipePipe,
    AmpAmp,
}

fn tokenize(source: &str) -> Result<Vec<Token>, ParseError> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = source.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];

        if c.is_whitespace() {
            i += 1;
            continue;
        }

        match c {
            '=' => {
                tokens.push(Token::Eq);
                i += 1;
            }
            '(' => {
                tokens.push(Token::LParen);
                i += 1;
            }
            ')' => {
                tokens.push(Token::RParen);
                i += 1;
            }
            '{' => {
                tokens.push(Token::LBrace);
                i += 1;
            }
            '}' => {
                tokens.push(Token::RBrace);
                i += 1;
            }
            '[' => {
                tokens.push(Token::LBracket);
                i += 1;
            }
            ']' => {
                tokens.push(Token::RBracket);
                i += 1;
            }
            '.' => {
                tokens.push(Token::Dot);
                i += 1;
            }
            ':' => {
                tokens.push(Token::Colon);
                i += 1;
            }
            ',' => {
                tokens.push(Token::Comma);
                i += 1;
            }
            '+' => {
                tokens.push(Token::Plus);
                i += 1;
            }
            '-' => {
                tokens.push(Token::Minus);
                i += 1;
            }
            '*' => {
                tokens.push(Token::Star);
                i += 1;
            }
            '/' => {
                tokens.push(Token::Slash);
                i += 1;
            }
            '%' => {
                tokens.push(Token::Percent);
                i += 1;
            }
            '^' => {
                tokens.push(Token::Caret);
                i += 1;
            }
            '!' => {
                tokens.push(Token::Bang);
                i += 1;
            }
            '|' => {
                if chars.get(i + 1) == Some(&'|') {
                    tokens.push(Token::PipePipe);
                    i += 2;
                } else {
                    return Err(ParseError(
                        "unexpected character `|` — did you mean `||`?".to_string(),
                    ));
                }
            }
            '&' => {
                if chars.get(i + 1) == Some(&'&') {
                    tokens.push(Token::AmpAmp);
                    i += 2;
                } else {
                    return Err(ParseError(
                        "unexpected character `&` — did you mean `&&`?".to_string(),
                    ));
                }
            }
            '"' => {
                i += 1;
                let mut s = String::new();
                loop {
                    match chars.get(i) {
                        None => return Err(ParseError("unterminated string literal".to_string())),
                        Some('"') => {
                            i += 1;
                            break;
                        }
                        Some('\\') => {
                            i += 1;
                            match chars.get(i) {
                                Some('"') => {
                                    s.push('"');
                                    i += 1;
                                }
                                Some('\\') => {
                                    s.push('\\');
                                    i += 1;
                                }
                                Some('n') => {
                                    s.push('\n');
                                    i += 1;
                                }
                                Some('t') => {
                                    s.push('\t');
                                    i += 1;
                                }
                                Some(ec) => {
                                    return Err(ParseError(format!(
                                        "unknown escape sequence `\\{ec}`"
                                    )))
                                }
                                None => {
                                    return Err(ParseError(
                                        "unterminated escape in string literal".to_string(),
                                    ))
                                }
                            }
                        }
                        Some(ch) => {
                            s.push(*ch);
                            i += 1;
                        }
                    }
                }
                tokens.push(Token::Str(s));
            }
            c if c.is_ascii_digit() => {
                let start = i;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
                if i < chars.len()
                    && chars[i] == '.'
                    && chars.get(i + 1).is_some_and(|c| c.is_ascii_digit())
                {
                    i += 1;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                let s: String = chars[start..i].iter().collect();
                let n: f64 = s
                    .parse()
                    .map_err(|_| ParseError(format!("invalid number `{s}`")))?;
                tokens.push(Token::Num(n));
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let start = i;
                i += 1;
                while i < chars.len() {
                    let ch = chars[i];
                    if ch.is_ascii_alphanumeric() || ch == '_' {
                        i += 1;
                    } else if ch == '-' {
                        // Include '-' only when followed by a letter or '_', so
                        // `get-weather` is one identifier but `x - 3` is not.
                        if chars
                            .get(i + 1)
                            .is_some_and(|c| c.is_ascii_alphabetic() || *c == '_')
                        {
                            i += 1;
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                let ident: String = chars[start..i].iter().collect();
                tokens.push(Token::Ident(ident));
            }
            other => {
                return Err(ParseError(format!("unexpected character `{other}`")));
            }
        }
    }

    Ok(tokens)
}

// ── Parser ────────────────────────────────────────────────────────────────────

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Parser { tokens, pos: 0 }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    /// Clones the next token without consuming it; needed to avoid holding a
    /// borrow on `self` while also calling mutating methods.
    fn peek_cloned(&self) -> Option<Token> {
        self.tokens.get(self.pos).cloned()
    }

    /// If the next token is an `Ident`, return its string without consuming it
    /// (used for keyword dispatch without borrow-checker conflict).
    fn peek_ident(&self) -> Option<String> {
        match self.tokens.get(self.pos) {
            Some(Token::Ident(s)) => Some(s.clone()),
            _ => None,
        }
    }

    fn bump(&mut self) -> Option<Token> {
        let tok = self.tokens.get(self.pos).cloned();
        if tok.is_some() {
            self.pos += 1;
        }
        tok
    }

    fn expect_ident(&mut self) -> Result<String, ParseError> {
        match self.bump() {
            Some(Token::Ident(s)) => Ok(s),
            Some(other) => Err(ParseError(format!(
                "expected identifier, found `{other:?}`"
            ))),
            None => Err(ParseError(
                "expected identifier, reached end of input".to_string(),
            )),
        }
    }

    fn expect_token(&mut self, want: &Token) -> Result<(), ParseError> {
        match self.bump() {
            Some(ref got) if got == want => Ok(()),
            Some(got) => Err(ParseError(format!("expected `{want:?}`, found `{got:?}`"))),
            None => Err(ParseError(format!(
                "expected `{want:?}`, reached end of input"
            ))),
        }
    }

    fn parse_statement(&mut self) -> Result<Statement, ParseError> {
        let name = self.expect_ident()?;
        self.expect_token(&Token::Eq)?;
        let expr = self.parse_expr()?;
        Ok(Statement { name, expr })
    }

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_binary()
    }

    fn parse_binary(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Some(Token::Plus) => BinOp::Add,
                Some(Token::Minus) => BinOp::Sub,
                Some(Token::Star) => BinOp::Mul,
                Some(Token::Slash) => BinOp::Div,
                Some(Token::Percent) => BinOp::Rem,
                Some(Token::Caret) => BinOp::Pow,
                Some(Token::PipePipe) => BinOp::Or,
                Some(Token::AmpAmp) => BinOp::And,
                _ => break,
            };
            self.bump();
            let rhs = self.parse_unary()?;
            lhs = Expr::BinOp {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        if matches!(self.peek(), Some(Token::Bang)) {
            self.bump();
            let inner = self.parse_unary()?;
            return Ok(Expr::Not(Box::new(inner)));
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<Expr, ParseError> {
        let base = self.parse_primary()?;
        let mut keys = Vec::new();
        while matches!(self.peek(), Some(Token::Dot)) {
            self.bump();
            keys.push(self.parse_access_key()?);
        }
        if keys.is_empty() {
            Ok(base)
        } else {
            Ok(Expr::Access {
                base: Box::new(base),
                keys,
            })
        }
    }

    fn parse_access_key(&mut self) -> Result<AccessKey, ParseError> {
        match self.peek_cloned() {
            Some(Token::Str(s)) => {
                self.bump();
                Ok(AccessKey::Quoted(s))
            }
            Some(Token::Num(n)) if n.fract() == 0.0 && n >= 0.0 => {
                self.bump();
                Ok(AccessKey::Index(n as usize))
            }
            Some(Token::LParen) => {
                self.bump();
                let expr = self.parse_expr()?;
                self.expect_token(&Token::RParen)?;
                Ok(AccessKey::Computed(Box::new(expr)))
            }
            Some(Token::Ident(s)) => {
                self.bump();
                Ok(AccessKey::Ident(s))
            }
            Some(other) => Err(ParseError(format!(
                "expected access key after `.`, found `{other:?}`"
            ))),
            None => Err(ParseError(
                "expected access key after `.`, reached end of input".to_string(),
            )),
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        // Keyword dispatch: clone the ident string so we can freely call
        // mutating methods in the match arm bodies without a live borrow.
        match self.peek_ident().as_deref() {
            Some("true") => {
                self.bump();
                return Ok(Expr::Bool(true));
            }
            Some("false") => {
                self.bump();
                return Ok(Expr::Bool(false));
            }
            Some("if") => {
                self.bump();
                return self.parse_if();
            }
            Some("for") | Some("foreach") => {
                self.bump();
                return self.parse_for();
            }
            Some("run") | Some("spawn") => {
                self.bump();
                return self.parse_run();
            }
            Some(_) => {
                // Non-keyword identifier → variable reference.
                let s = self.expect_ident()?;
                return Ok(Expr::Var(s));
            }
            None => {}
        }

        // Non-ident tokens.
        match self.peek_cloned() {
            Some(Token::Str(s)) => {
                self.bump();
                Ok(Expr::Str(s))
            }
            Some(Token::Num(n)) => {
                self.bump();
                Ok(Expr::Num(n))
            }
            Some(Token::LBrace) => self.parse_object_literal(),
            Some(Token::LParen) => {
                self.bump();
                let expr = self.parse_expr()?;
                self.expect_token(&Token::RParen)?;
                Ok(expr)
            }
            Some(other) => Err(ParseError(format!(
                "unexpected token `{other:?}` in expression"
            ))),
            None => Err(ParseError(
                "expected expression, reached end of input".to_string(),
            )),
        }
    }

    fn parse_if(&mut self) -> Result<Expr, ParseError> {
        // if (cond) result, ..., (_) default
        let mut branches = Vec::new();
        loop {
            self.expect_token(&Token::LParen)?;

            // Default branch: (_)
            if self.peek_ident().as_deref() == Some("_") {
                self.bump();
                self.expect_token(&Token::RParen)?;
                let default = self.parse_expr()?;
                return Ok(Expr::If {
                    branches,
                    default: Box::new(default),
                });
            }

            let cond = self.parse_expr()?;
            self.expect_token(&Token::RParen)?;
            let result = self.parse_expr()?;
            branches.push((cond, result));

            self.expect_token(&Token::Comma)?;
        }
    }

    fn parse_for(&mut self) -> Result<Expr, ParseError> {
        // for [item: array_expr] (body_expr)
        self.expect_token(&Token::LBracket)?;
        let item = self.expect_ident()?;
        self.expect_token(&Token::Colon)?;
        let array = self.parse_expr()?;
        self.expect_token(&Token::RBracket)?;
        self.expect_token(&Token::LParen)?;
        let body = self.parse_expr()?;
        self.expect_token(&Token::RParen)?;
        Ok(Expr::For {
            item,
            array: Box::new(array),
            body: Box::new(body),
        })
    }

    fn parse_run(&mut self) -> Result<Expr, ParseError> {
        // run <kind> <name> <args>. The verb (run/spawn) and the kind keyword
        // accept the same aliases as the CLI's `run` subcommands.
        let kind = self.expect_ident()?;
        let name = self.expect_ident()?;
        match kind.as_str() {
            "tool" => Ok(Expr::RunTool {
                tool: name,
                args: self.parse_named_map()?,
            }),
            "workflow" | "overseer" => Ok(Expr::RunWorkflow {
                workflow: name,
                args: self.parse_named_map()?,
            }),
            "specialist" | "lenny" | "ling" => Ok(Expr::RunSpecialist {
                specialist: name,
                prompt: Box::new(self.parse_expr()?),
            }),
            other => Err(ParseError(format!(
                "expected `tool`, `workflow`, or `specialist` after `run`, found `{other}`"
            ))),
        }
    }

    /// Parse a `{ KEY: expr, ... }` argument map — the argument shape of
    /// `run tool` and `run workflow`.
    fn parse_named_map(&mut self) -> Result<Vec<(String, Expr)>, ParseError> {
        self.expect_token(&Token::LBrace)?;
        let mut args = Vec::new();
        if !matches!(self.peek(), Some(Token::RBrace)) {
            loop {
                let key = self.expect_ident()?;
                self.expect_token(&Token::Colon)?;
                let val = self.parse_expr()?;
                args.push((key, val));
                if matches!(self.peek(), Some(Token::Comma)) {
                    self.bump();
                } else {
                    break;
                }
            }
        }
        self.expect_token(&Token::RBrace)?;
        Ok(args)
    }

    fn parse_object_literal(&mut self) -> Result<Expr, ParseError> {
        // { "key": expr, ... }
        self.expect_token(&Token::LBrace)?;
        let mut fields = Vec::new();
        if !matches!(self.peek(), Some(Token::RBrace)) {
            loop {
                match self.bump() {
                    Some(Token::Str(k)) => {
                        self.expect_token(&Token::Colon)?;
                        let val = self.parse_expr()?;
                        fields.push((k, val));
                    }
                    Some(other) => {
                        return Err(ParseError(format!(
                            "expected quoted string key in object literal, found `{other:?}`"
                        )))
                    }
                    None => return Err(ParseError("unterminated object literal".to_string())),
                }
                if matches!(self.peek(), Some(Token::Comma)) {
                    self.bump();
                } else {
                    break;
                }
            }
        }
        self.expect_token(&Token::RBrace)?;
        Ok(Expr::Object(fields))
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

/// Pull the `# inputs:` declaration (if any) out of the source and blank every
/// full-line `#` comment so it acts as a separator rather than reaching the
/// tokenizer. The first `# inputs:` line wins (mirroring tool headers); a
/// comment line is any line whose first non-space character is `#`.
fn extract_header(source: &str) -> Result<(Vec<Param>, String), ParseError> {
    let mut inputs: Option<Vec<Param>> = None;
    let mut stripped = String::with_capacity(source.len());

    for line in source.lines() {
        if let Some(comment) = line.trim_start().strip_prefix('#') {
            if let Some(rest) = comment.trim().strip_prefix("inputs:") {
                if inputs.is_none() {
                    inputs = Some(
                        parse_params(rest)
                            .map_err(|e| ParseError(format!("invalid `# inputs:`: {e}")))?,
                    );
                }
            }
            // Blank the comment line so statement splitting is unaffected.
        } else {
            stripped.push_str(line);
        }
        stripped.push('\n');
    }

    Ok((inputs.unwrap_or_default(), stripped))
}

/// Split a workflow source into statement chunks. Statements are separated by
/// blank lines (lines containing only whitespace). A single blank line ends
/// the current chunk; successive blank lines are treated as one separator.
fn split_statements(source: &str) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();

    for line in source.lines() {
        if line.trim().is_empty() {
            if !current.trim().is_empty() {
                chunks.push(current.trim().to_string());
                current = String::new();
            }
        } else {
            if !current.is_empty() {
                current.push('\n');
            }
            current.push_str(line);
        }
    }
    if !current.trim().is_empty() {
        chunks.push(current.trim().to_string());
    }
    chunks
}

/// Parse a workflow source string into a [`Workflow`] AST.
///
/// Statements are separated by blank lines. Within a statement, newlines are
/// treated as ordinary whitespace. Returns a [`ParseError`] if the source
/// doesn't conform to the DSL grammar (workflow-dsl.md §5–6).
pub fn parse(source: &str) -> Result<Workflow, ParseError> {
    let (inputs, body) = extract_header(source)?;
    let chunks = split_statements(&body);
    let mut statements = Vec::new();

    for (i, chunk) in chunks.iter().enumerate() {
        let tokens =
            tokenize(chunk).map_err(|e| ParseError(format!("statement {}: {}", i + 1, e.0)))?;
        let mut parser = Parser::new(tokens);
        let stmt = parser
            .parse_statement()
            .map_err(|e| ParseError(format!("statement {}: {}", i + 1, e.0)))?;
        if parser.peek().is_some() {
            return Err(ParseError(format!(
                "statement {}: unexpected tokens after expression",
                i + 1
            )));
        }
        statements.push(stmt);
    }

    Ok(Workflow { inputs, statements })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "parse_tests.rs"]
mod tests;
