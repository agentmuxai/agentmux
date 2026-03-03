// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! CSS style attribute parser.
//! Port of Go's `pkg/vdom/cssparser/cssparser.go`.
//!
//! Parses inline CSS style strings (e.g., `"color: red; font-size: 14px"`)
//! into a map of property name → value. Handles quoted strings, nested
//! parentheses, and escape sequences.

#![allow(dead_code)]

use std::collections::HashMap;

/// CSS style attribute parser.
pub struct CssParser {
    input: Vec<u8>,
    pos: usize,
    in_quote: bool,
    quote_char: u8,
    open_parens: usize,
}

impl CssParser {
    /// Create a new parser for the given CSS style string.
    pub fn new(input: &str) -> Self {
        Self {
            input: input.as_bytes().to_vec(),
            pos: 0,
            in_quote: false,
            quote_char: 0,
            open_parens: 0,
        }
    }

    /// Parse the style string into a map of property → value.
    pub fn parse(&mut self) -> Result<HashMap<String, String>, String> {
        let mut result = HashMap::new();
        let mut last_prop = String::new();

        loop {
            self.skip_whitespace();
            if self.eof() {
                break;
            }
            let prop_name = self.parse_identifier_colon(&last_prop)?;
            last_prop.clone_from(&prop_name);
            self.skip_whitespace();
            let value = self.parse_value(&prop_name)?;
            result.insert(prop_name, value);
            self.skip_whitespace();
            if self.eof() {
                break;
            }
            if !self.expect_char(b';') {
                break;
            }
        }

        self.skip_whitespace();
        if !self.eof() {
            return Err(format!(
                "bad style attribute, unexpected character {:?} at pos {}",
                self.input[self.pos] as char,
                self.pos + 1
            ));
        }

        Ok(result)
    }

    fn parse_identifier_colon(&mut self, last_prop: &str) -> Result<String, String> {
        let start = self.pos;
        while !self.eof() {
            let c = self.peek_char();
            if is_ident_char(c) || c == b'-' {
                self.advance();
            } else {
                break;
            }
        }
        let attr_name = std::str::from_utf8(&self.input[start..self.pos])
            .unwrap_or("")
            .to_string();
        self.skip_whitespace();
        if self.eof() {
            return Err(format!(
                "bad style attribute, expected colon after property {:?}, got EOF, at pos {}",
                attr_name,
                self.pos + 1
            ));
        }
        if attr_name.is_empty() {
            return Err(format!(
                "bad style attribute, invalid property name after property {:?}, at pos {}",
                last_prop,
                self.pos + 1
            ));
        }
        if !self.expect_char(b':') {
            return Err(format!(
                "bad style attribute, bad property name starting with {:?}, expected colon, got {:?}, at pos {}",
                attr_name,
                self.input[self.pos] as char,
                self.pos + 1
            ));
        }
        Ok(attr_name)
    }

    fn parse_value(&mut self, prop_name: &str) -> Result<String, String> {
        let start = self.pos;
        let mut quote_pos = 0usize;
        let mut paren_pos_stack: Vec<usize> = Vec::new();

        while !self.eof() {
            let c = self.peek_char();
            if self.in_quote {
                if c == self.quote_char {
                    self.in_quote = false;
                } else if c == b'\\' {
                    self.advance();
                }
            } else {
                if c == b'"' || c == b'\'' {
                    self.in_quote = true;
                    self.quote_char = c;
                    quote_pos = self.pos;
                } else if c == b'(' {
                    self.open_parens += 1;
                    paren_pos_stack.push(self.pos);
                } else if c == b')' {
                    if self.open_parens == 0 {
                        return Err(format!("unmatched ')' at pos {}", self.pos + 1));
                    }
                    self.open_parens -= 1;
                    paren_pos_stack.pop();
                } else if c == b';' && self.open_parens == 0 {
                    break;
                }
            }
            self.advance();
        }

        if self.eof() && self.in_quote {
            return Err(format!(
                "bad style attribute, while parsing attribute {:?}, unmatched quote at pos {}",
                prop_name,
                quote_pos + 1
            ));
        }
        if self.eof() && self.open_parens > 0 {
            return Err(format!(
                "bad style attribute, while parsing property {:?}, unmatched '(' at pos {}",
                prop_name,
                paren_pos_stack.last().unwrap() + 1
            ));
        }

        let raw = std::str::from_utf8(&self.input[start..self.pos]).unwrap_or("");
        Ok(raw.trim().to_string())
    }

    fn skip_whitespace(&mut self) {
        while !self.eof() && (self.peek_char() as char).is_whitespace() {
            self.advance();
        }
    }

    fn expect_char(&mut self, expected: u8) -> bool {
        if !self.eof() && self.peek_char() == expected {
            self.advance();
            return true;
        }
        false
    }

    fn peek_char(&self) -> u8 {
        if self.pos >= self.input.len() {
            return 0;
        }
        self.input[self.pos]
    }

    fn advance(&mut self) {
        self.pos += 1;
    }

    fn eof(&self) -> bool {
        self.pos >= self.input.len()
    }
}

/// Check if a byte is a valid identifier character (letter or digit).
fn is_ident_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c > 127 // ASCII ident + non-ASCII
}

/// Parse a CSS style attribute string into a map of property → value.
///
/// Convenience wrapper around `CssParser::new(input).parse()`.
pub fn parse_style(input: &str) -> Result<HashMap<String, String>, String> {
    CssParser::new(input).parse()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_property() {
        let result = parse_style("color: red").unwrap();
        assert_eq!(result.get("color").unwrap(), "red");
    }

    #[test]
    fn test_multiple_properties() {
        let result = parse_style("color: red; font-size: 14px; display: flex").unwrap();
        assert_eq!(result.get("color").unwrap(), "red");
        assert_eq!(result.get("font-size").unwrap(), "14px");
        assert_eq!(result.get("display").unwrap(), "flex");
    }

    #[test]
    fn test_trailing_semicolon() {
        let result = parse_style("color: red;").unwrap();
        assert_eq!(result.get("color").unwrap(), "red");
    }

    #[test]
    fn test_whitespace_handling() {
        let result = parse_style("  color :  red  ;  font-size : 14px  ").unwrap();
        assert_eq!(result.get("color").unwrap(), "red");
        assert_eq!(result.get("font-size").unwrap(), "14px");
    }

    #[test]
    fn test_quoted_values() {
        let result = parse_style(r#"font-family: "Courier New"; content: 'hello'"#).unwrap();
        assert_eq!(result.get("font-family").unwrap(), r#""Courier New""#);
        assert_eq!(result.get("content").unwrap(), "'hello'");
    }

    #[test]
    fn test_parentheses_in_value() {
        let result = parse_style("background: url(image.png); transform: rotate(45deg)").unwrap();
        assert_eq!(result.get("background").unwrap(), "url(image.png)");
        assert_eq!(result.get("transform").unwrap(), "rotate(45deg)");
    }

    #[test]
    fn test_nested_parentheses() {
        let result = parse_style("background: calc(100% - var(--gap))").unwrap();
        assert_eq!(result.get("background").unwrap(), "calc(100% - var(--gap))");
    }

    #[test]
    fn test_semicolon_in_parens() {
        // Semicolons inside parens should not split
        let result = parse_style("content: url(a;b)").unwrap();
        assert_eq!(result.get("content").unwrap(), "url(a;b)");
    }

    #[test]
    fn test_escaped_quote() {
        let result = parse_style(r#"content: "hello \"world\"""#).unwrap();
        assert_eq!(result.get("content").unwrap(), r#""hello \"world\"""#);
    }

    #[test]
    fn test_empty_input() {
        let result = parse_style("").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_whitespace_only() {
        let result = parse_style("   ").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_unmatched_quote_error() {
        let result = parse_style(r#"color: "red"#);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unmatched quote"));
    }

    #[test]
    fn test_unmatched_paren_error() {
        let result = parse_style("background: url(image.png");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unmatched '('"));
    }

    #[test]
    fn test_unmatched_close_paren_error() {
        let result = parse_style("background: )");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unmatched ')'"));
    }

    #[test]
    fn test_missing_colon_error() {
        let result = parse_style("color red");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("expected colon"));
    }

    #[test]
    fn test_vendor_prefix() {
        let result = parse_style("-webkit-transform: rotate(45deg)").unwrap();
        assert_eq!(result.get("-webkit-transform").unwrap(), "rotate(45deg)");
    }

    #[test]
    fn test_complex_value() {
        let result =
            parse_style("border: 1px solid rgba(0, 0, 0, 0.5); margin: 0 auto").unwrap();
        assert_eq!(
            result.get("border").unwrap(),
            "1px solid rgba(0, 0, 0, 0.5)"
        );
        assert_eq!(result.get("margin").unwrap(), "0 auto");
    }
}
