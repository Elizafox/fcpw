//! RFC 8949 diagnostic notation for dynamic values.

use alloc::{format, string::String};
use core::fmt::Write;

use crate::{BorrowedValue, Error, ErrorKind, Result, Value};

/// Formats one encoded CBOR item using diagnostic notation.
pub fn format(input: &[u8]) -> Result<String> {
    let value: Value = BorrowedValue::decode(input)?.into();
    let mut output = String::new();
    write_value(&value, &mut output);
    Ok(output)
}

fn write_value(value: &Value, output: &mut String) {
    match value {
        Value::Unsigned(value) => write!(output, "{value}").unwrap(),
        Value::Negative(value) => write!(output, "{value}").unwrap(),
        Value::Bytes(bytes) => {
            output.push_str("h'");
            for byte in bytes {
                write!(output, "{byte:02x}").unwrap();
            }
            output.push('\'');
        }
        Value::Text(text) => {
            output.push('"');
            for character in text.chars() {
                match character {
                    '"' => output.push_str("\\\""),
                    '\\' => output.push_str("\\\\"),
                    '\n' => output.push_str("\\n"),
                    '\r' => output.push_str("\\r"),
                    '\t' => output.push_str("\\t"),
                    value => output.push(value),
                }
            }
            output.push('"');
        }
        Value::Array(values) => {
            output.push('[');
            separated(values.iter(), output, |value, output| {
                write_value(value, output)
            });
            output.push(']');
        }
        Value::Map(entries) => {
            output.push('{');
            separated(entries.iter(), output, |(key, value), output| {
                write_value(key, output);
                output.push_str(": ");
                write_value(value, output);
            });
            output.push('}');
        }
        Value::Tag(tag, value) => {
            write!(output, "{tag}(").unwrap();
            write_value(value, output);
            output.push(')');
        }
        Value::Simple(value) => write!(output, "simple({value})").unwrap(),
        Value::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        Value::Null => output.push_str("null"),
        Value::Undefined => output.push_str("undefined"),
        Value::Float(value) => output.push_str(&format!("{value:?}")),
    }
}

fn separated<T>(
    values: impl Iterator<Item = T>,
    output: &mut String,
    mut write: impl FnMut(T, &mut String),
) {
    for (index, value) in values.enumerate() {
        if index != 0 {
            output.push_str(", ");
        }
        write(value, output);
    }
}

/// Parses the JSON-compatible subset of diagnostic notation.
///
/// Integers, floating-point values, quoted strings, arrays, maps, booleans,
/// `null`, and `undefined` are accepted.
pub fn parse(input: &str) -> Result<Value> {
    let mut parser = DiagnosticParser {
        bytes: input.as_bytes(),
        position: 0,
    };
    let value = parser.value()?;
    parser.whitespace();
    if parser.position != parser.bytes.len() {
        return Err(parser.error("trailing diagnostic input"));
    }

    Ok(value)
}

struct DiagnosticParser<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl DiagnosticParser<'_> {
    fn error(&self, _message: &'static str) -> Error {
        Error::new(ErrorKind::Message, self.position)
    }

    fn whitespace(&mut self) {
        while self
            .bytes
            .get(self.position)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.position += 1;
        }
    }

    fn take(&mut self, byte: u8) -> bool {
        self.whitespace();
        if self.bytes.get(self.position) == Some(&byte) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn value(&mut self) -> Result<Value> {
        self.whitespace();
        match self.bytes.get(self.position).copied() {
            Some(b'"') => self.string().map(Value::Text),
            Some(b'[') => self.array(),
            Some(b'{') => self.map(),
            Some(b't') if self.keyword(b"true") => Ok(Value::Bool(true)),
            Some(b'f') if self.keyword(b"false") => Ok(Value::Bool(false)),
            Some(b'n') if self.keyword(b"null") => Ok(Value::Null),
            Some(b'u') if self.keyword(b"undefined") => Ok(Value::Undefined),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(self.error("expected diagnostic value")),
        }
    }

    fn keyword(&mut self, keyword: &[u8]) -> bool {
        if self.bytes.get(self.position..self.position + keyword.len()) == Some(keyword) {
            self.position += keyword.len();
            true
        } else {
            false
        }
    }

    fn string(&mut self) -> Result<String> {
        self.position += 1;
        let mut result = String::new();
        loop {
            let byte = *self
                .bytes
                .get(self.position)
                .ok_or(self.error("unterminated string"))?;
            self.position += 1;
            match byte {
                b'"' => return Ok(result),
                b'\\' => {
                    let escaped = *self
                        .bytes
                        .get(self.position)
                        .ok_or(self.error("unterminated escape"))?;
                    self.position += 1;
                    result.push(match escaped {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        _ => return Err(self.error("unsupported escape")),
                    });
                }
                0..=0x7f => result.push(byte as char),
                _ => return Err(self.error("non-ASCII diagnostic string")),
            }
        }
    }

    fn array(&mut self) -> Result<Value> {
        self.position += 1;
        let mut values = alloc::vec::Vec::new();
        if self.take(b']') {
            return Ok(Value::Array(values));
        }
        loop {
            values.push(self.value()?);
            if self.take(b']') {
                break;
            }
            if !self.take(b',') {
                return Err(self.error("expected comma"));
            }
        }
        Ok(Value::Array(values))
    }

    fn map(&mut self) -> Result<Value> {
        self.position += 1;
        let mut entries = alloc::vec::Vec::new();
        if self.take(b'}') {
            return Ok(Value::Map(entries));
        }
        loop {
            let key = self.value()?;
            if !self.take(b':') {
                return Err(self.error("expected colon"));
            }
            entries.push((key, self.value()?));
            if self.take(b'}') {
                break;
            }
            if !self.take(b',') {
                return Err(self.error("expected comma"));
            }
        }
        Ok(Value::Map(entries))
    }

    fn number(&mut self) -> Result<Value> {
        let start = self.position;
        while self
            .bytes
            .get(self.position)
            .is_some_and(|byte| matches!(byte, b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9'))
        {
            self.position += 1;
        }
        let text = core::str::from_utf8(&self.bytes[start..self.position]).unwrap();
        if text.contains(['.', 'e', 'E']) {
            text.parse()
                .map(Value::Float)
                .map_err(|_| self.error("invalid float"))
        } else {
            let value: i128 = text.parse().map_err(|_| self.error("invalid integer"))?;
            if value >= 0 {
                u64::try_from(value)
                    .map(Value::Unsigned)
                    .map_err(|_| self.error("integer overflow"))
            } else {
                Ok(Value::Negative(value))
            }
        }
    }
}
