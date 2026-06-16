//! Tokenizer for the workflow DSL (workflow-dsl.md §5): turns source text
//! into the [`Token`] stream the parser in [`super`] consumes.

use super::ParseError;

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Token {
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

pub(super) fn tokenize(source: &str) -> Result<Vec<Token>, ParseError> {
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
