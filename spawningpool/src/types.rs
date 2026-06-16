//! The workflow DSL type system: the notation a tool header declares (in its
//! `# params:` and `# output:` lines) and the rule for lowering it to JSON
//! Schema.
//!
//! This is the foundation the Workflow DSL builds on (see `docs/workflow-dsl.md`
//! §2). A [`Type`] is parsed from the header notation and lowered with
//! [`Type::to_schema`] into the same JSON Schema the tool-call validator
//! ([`crate::ai::validation::validate_tool_call`]) and the schema builder in
//! [`crate::domain::ToolDef::to_tool`] already consume, so nothing here
//! duplicates schema handling.

use serde::{Serialize, Serializer};

/// A value's declared shape. The grammar (workflow-dsl §2):
///
/// | Type | Notation |
/// | --- | --- |
/// | string | `string` |
/// | number | `number` |
/// | bool | `bool` |
/// | array | `[T]` |
/// | object | `{ "k": T, "k2": T2 }` (listed keys are required and exhaustive) |
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    String,
    Number,
    Bool,
    Array(Box<Type>),
    /// An object's declared keys, in source order. Every key is required.
    Object(Vec<(String, Type)>),
}

/// A tool parameter: its name and declared [`Type`]. A bare header param (no
/// `:type` suffix) has type [`Type::String`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Param {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: Type,
}

/// One entry of a tool's `# exits:` header (see `docs/tools.md`): an exit status
/// `code`, a compilable `name`, and an optional human-readable `desc`. The
/// `name` is a DSL identifier so a later workflow stage can branch on it (see
/// `docs/workflow-dsl.md` §8.1); the `desc` is sugar for humans and the model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ExitCode {
    pub code: i32,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desc: Option<String>,
}

impl Type {
    /// Parse the §2 notation, e.g. `[{ "host": string, "ms": number }]`.
    /// Errors carry a human-readable reason; whitespace is insignificant.
    pub fn parse(s: &str) -> Result<Type, String> {
        let mut parser = Parser {
            chars: s.chars().collect(),
            pos: 0,
        };
        let ty = parser.parse_type()?;
        parser.skip_ws();
        if parser.pos != parser.chars.len() {
            return Err(format!("unexpected trailing characters in type `{s}`"));
        }
        Ok(ty)
    }

    /// Lower into JSON Schema (workflow-dsl §2.1): `string`/`number`/`bool` map
    /// to their scalar schema, `[T]` to an `array` with `items`, and an object
    /// to an `object` whose every declared key is `required`.
    pub fn to_schema(&self) -> serde_json::Value {
        match self {
            Type::String => serde_json::json!({ "type": "string" }),
            Type::Number => serde_json::json!({ "type": "number" }),
            Type::Bool => serde_json::json!({ "type": "boolean" }),
            Type::Array(inner) => serde_json::json!({
                "type": "array",
                "items": inner.to_schema(),
            }),
            Type::Object(fields) => {
                let properties: serde_json::Map<String, serde_json::Value> = fields
                    .iter()
                    .map(|(key, ty)| (key.clone(), ty.to_schema()))
                    .collect();
                let required: Vec<&String> = fields.iter().map(|(key, _)| key).collect();
                serde_json::json!({
                    "type": "object",
                    "properties": properties,
                    "required": required,
                })
            }
        }
    }
}

/// Renders the §2 notation, so a [`Type`] round-trips back to the text a header
/// would carry (and serializes as that string — see the [`Serialize`] impl).
impl std::fmt::Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Type::String => f.write_str("string"),
            Type::Number => f.write_str("number"),
            Type::Bool => f.write_str("bool"),
            Type::Array(inner) => write!(f, "[{inner}]"),
            Type::Object(fields) if fields.is_empty() => f.write_str("{}"),
            Type::Object(fields) => {
                f.write_str("{ ")?;
                for (i, (key, ty)) in fields.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "\"{key}\": {ty}")?;
                }
                f.write_str(" }")
            }
        }
    }
}

/// Serialized as its notation string, so `spawningpool show tool` renders a
/// type as the same text its header declares rather than a tagged enum.
impl Serialize for Type {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

/// A cursor over the notation's characters. Hand-rolled rather than pulling in a
/// parser dependency — the grammar is tiny.
struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(c) if c.is_whitespace()) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, want: char) -> Result<(), String> {
        match self.bump() {
            Some(got) if got == want => Ok(()),
            Some(got) => Err(format!("expected `{want}`, found `{got}`")),
            None => Err(format!("expected `{want}`, reached end of input")),
        }
    }

    fn parse_type(&mut self) -> Result<Type, String> {
        self.skip_ws();
        match self.peek() {
            Some('[') => self.parse_array(),
            Some('{') => self.parse_object(),
            Some(c) if c.is_ascii_alphabetic() => self.parse_scalar(),
            Some(c) => Err(format!("unexpected character `{c}` in type")),
            None => Err("expected a type but reached end of input".to_string()),
        }
    }

    fn parse_scalar(&mut self) -> Result<Type, String> {
        let start = self.pos;
        while matches!(self.peek(), Some(c) if c.is_ascii_alphabetic()) {
            self.pos += 1;
        }
        let word: String = self.chars[start..self.pos].iter().collect();
        match word.as_str() {
            "string" => Ok(Type::String),
            "number" => Ok(Type::Number),
            "bool" => Ok(Type::Bool),
            other => Err(format!("unknown type `{other}`")),
        }
    }

    fn parse_array(&mut self) -> Result<Type, String> {
        self.expect('[')?;
        let inner = self.parse_type()?;
        self.skip_ws();
        self.expect(']')?;
        Ok(Type::Array(Box::new(inner)))
    }

    fn parse_object(&mut self) -> Result<Type, String> {
        self.expect('{')?;
        self.skip_ws();
        let mut fields = Vec::new();
        if self.peek() == Some('}') {
            self.bump();
            return Ok(Type::Object(fields));
        }
        loop {
            self.skip_ws();
            let key = self.parse_string_key()?;
            self.skip_ws();
            self.expect(':')?;
            let ty = self.parse_type()?;
            fields.push((key, ty));
            self.skip_ws();
            match self.bump() {
                Some(',') => continue,
                Some('}') => break,
                Some(c) => return Err(format!("expected `,` or `}}` in object type, found `{c}`")),
                None => return Err("unterminated object type".to_string()),
            }
        }
        Ok(Type::Object(fields))
    }

    fn parse_string_key(&mut self) -> Result<String, String> {
        self.expect('"')?;
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c == '"' {
                let key: String = self.chars[start..self.pos].iter().collect();
                self.bump();
                return Ok(key);
            }
            self.pos += 1;
        }
        Err("unterminated string key in object type".to_string())
    }
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
