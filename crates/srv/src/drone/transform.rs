// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Transform node's evaluator: shape lists and objects with a restricted
//! expression language and no code execution
//! (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md §3.9).
//!
//! **Module A of the engine.** The types, the bounds and [`taint_out`] are
//! fixed here; [`evaluate`] is the module's job. Its acceptance tests are
//! `transform_acceptance_tests.rs`.
//!
//! # Expression grammar
//!
//! Lowest precedence first; all binary operators are left associative.
//!
//! ```text
//! expr    := or
//! or      := and ( "||" and )*
//! and     := not ( "&&" not )*
//! not     := "!" not | cmp
//! cmp     := sum ( ( "==" | "!=" | "<" | "<=" | ">" | ">=" | "contains" | "matches" | "in_set" ) sum )?
//! sum     := prod ( ( "+" | "-" ) prod )*
//! prod    := unary ( ( "*" | "/" | "%" ) unary )*
//! unary   := "-" unary | atom
//! atom    := number | string | "true" | "false" | "null" | list | call | path | "(" expr ")"
//! list    := "[" ( expr ( "," expr )* )? "]"
//! call    := name "(" ( expr ( "," expr )* )? ")"      // lower, trunc, default, exists
//! path    := name ( "." name )*                         // item, item.user.login, index
//! string  := 'text' or "text"                           // no escapes beyond \' \" \\
//! ```
//!
//! * `item` is the current element and `index` its position (0-based) inside
//!   `map`, `filter` and the key expressions (`by`). A missing path is `null`,
//!   never an error.
//! * `a contains b`: a string contains a substring, or a list holds an element.
//!   `a matches 're'`: the Rust `regex` crate (linear time) on a string.
//!   `a in_set [..]`: the list holds `a` (JSON equality).
//! * `lower(s)`, `trunc(s, n)` (at most `n` characters), `default(x, d)` (`d`
//!   when `x` is null), `exists(path)` (the path is not null). **No other
//!   function exists**, no variable, no assignment, no loop, no recursion, no
//!   call to the host and no network: an unknown name is [`TransformError::Expr`].
//! * Arithmetic is on numbers only; `/` by zero and `%` by zero are errors; an
//!   integer result stays an integer (`1 + 2` is `3`, not `3.0`).
//! * Comparing values of different types is `false` for `==`, `true` for `!=`,
//!   and an error for `<`, `<=`, `>`, `>=`.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};

use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};

use super::value::{join, Tv};

/// An input of at most this many items (§3.9).
pub const MAX_ITEMS: usize = 10_000;
/// An evaluation budget per node: one unit per element an operation visits and
/// per expression node evaluated (§3.9).
pub const MAX_OPERATIONS: u64 = 100_000;
/// An output of at most the state cap, in bytes of JSON (§3.2).
pub const MAX_OUTPUT_BYTES: usize = 256 * 1024;
/// A compiled `matches` pattern may be at most this big: a pattern that would
/// compile to more is an expression error, so a hostile one cannot make the
/// compiler work without bound.
const MAX_REGEX_BYTES: usize = 1024 * 1024;
/// How many compiled `matches` patterns one node keeps. A chain with more
/// distinct patterns compiles the rest per element, and the budget still bounds
/// how many can be evaluated.
const MAX_CACHED_PATTERNS: usize = 32;

/// One step of a chain. A chain reads top to bottom, each result feeding the next.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum TransformOp {
    /// A list in, a list out: `expr` per element.
    Map { expr: String },
    /// Keep the elements for which `expr` is true.
    Filter { expr: String },
    /// Keep only these top-level keys of an object, or of every object in a list.
    Pick { fields: Vec<String> },
    /// The opposite of `pick`.
    Omit { fields: Vec<String> },
    /// Stable sort of a list by the value of `by` (an expression from `item`, such as `item.age`).
    Sort {
        by: String,
        #[serde(default)]
        desc: bool,
    },
    /// An object from the key of `by` (as a string) to the list of elements with it, in input order.
    Group { by: String },
    /// Drop repeats, keeping the first: by whole value, or by the value of `by`.
    Unique {
        #[serde(default)]
        by: Option<String>,
    },
    /// `start..end` of a list (`end` absent: to the end); out-of-range ends are clamped.
    Slice {
        start: usize,
        #[serde(default)]
        end: Option<usize>,
    },
    /// The first element, or `null` for an empty list.
    First,
    /// The last element, or `null` for an empty list.
    Last,
    /// One level of lists flattened; non-list elements stay.
    Flatten,
    /// A list of strings or numbers as one string.
    Join { sep: String },
    /// A string as a list of strings.
    Split { sep: String },
    /// The input object with `with` merged over it (shallow; `with` wins).
    Merge { with: Value },
    /// The number of elements of a list, of keys of an object, or characters of a string.
    Count,
    /// The sum of a list of numbers, or of `by` over a list of objects.
    Sum {
        #[serde(default)]
        by: Option<String>,
    },
    /// The smallest, by the same rule as `sum`; `null` for an empty list.
    Min {
        #[serde(default)]
        by: Option<String>,
    },
    /// The largest, by the same rule as `sum`; `null` for an empty list.
    Max {
        #[serde(default)]
        by: Option<String>,
    },
    /// Whether a list has an element for which `expr` is true (any element when `expr` is absent), or an object has a non-null path `expr` (a dotted path from the object itself, with no `item`).
    Exists {
        #[serde(default)]
        expr: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub ops: Vec<TransformOp>,
}

/// Which bound a node went over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundKind {
    Items,
    Operations,
    Output,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransformError {
    /// A bound of §3.9 was passed. The node fails naming it.
    #[error("transform: over the limit of {limit} {kind:?}")]
    Bound { kind: BoundKind, limit: u64 },
    /// An operation could not be applied (a map on a number, a sum of text...). `index` is its place in the chain.
    #[error("transform: step {index}: {message}")]
    Op { index: usize, message: String },
    /// An expression did not parse or cannot be evaluated (an unknown function, a bad regex, a division by zero).
    #[error("transform: step {index}: expression: {message}")]
    Expr { index: usize, message: String },
}

/// What an operation does to the taint bits of the value it reads (the table
/// of §3.9 and the derived-value rule of §4.5). Returns `(tainted, derived)`.
///
/// * Operations that can carry text (everything but the next line) pass the
///   input's bits on: tainted in, tainted out.
/// * `count`, `sum`, `min`, `max` and `exists` cannot carry text: over tainted
///   input they give a clean value that is **`derived`**, so it is clean as
///   data and still attacker-chosen as control or as a numeric argument.
/// * Over clean input they stay clean; over derived input they stay derived.
pub fn taint_out(op: &TransformOp, input: &Tv) -> (bool, bool) {
    let carries_no_text = matches!(
        op,
        TransformOp::Count
            | TransformOp::Sum { .. }
            | TransformOp::Min { .. }
            | TransformOp::Max { .. }
            | TransformOp::Exists { .. }
    );
    let (tainted, derived) = join(&[input]);
    if carries_no_text && tainted {
        (false, true)
    } else {
        (tainted, derived)
    }
}

/// The bits of a chain's output: [`taint_out`] applied step by step, starting
/// from the input's.
pub fn chain_taint(ops: &[TransformOp], input: &Tv) -> (bool, bool) {
    let mut bits = (input.tainted, input.derived);
    for op in ops {
        let probe = Tv {
            v: Value::Null,
            tainted: bits.0,
            derived: bits.1,
        };
        bits = taint_out(op, &probe);
    }
    bits
}

/// Run `t` over `input`. The output's bits are [`chain_taint`] of the chain.
/// Every bound of §3.9 is checked and named in [`TransformError::Bound`].
pub fn evaluate(t: &Transform, input: &Tv) -> Result<Tv, TransformError> {
    let mut value = input.v.clone();
    check_items(&value)?;
    let mut state = State::default();
    for (step, op) in t.ops.iter().enumerate() {
        value = apply(op, &value, step, &mut state)?;
        check_items(&value)?;
        check_output(&value, step)?;
    }
    let (tainted, derived) = chain_taint(&t.ops, input);
    Ok(Tv {
        v: value,
        tainted,
        derived,
    })
}

/// The item bound: an input list, or a list a step makes, of more than
/// [`MAX_ITEMS`].
fn check_items(value: &Value) -> Result<(), TransformError> {
    match value {
        Value::Array(items) if items.len() > MAX_ITEMS => Err(TransformError::Bound {
            kind: BoundKind::Items,
            limit: MAX_ITEMS as u64,
        }),
        _ => Ok(()),
    }
}

/// The output bound, checked after every step so a runaway intermediate stops
/// early rather than filling memory first.
fn check_output(value: &Value, step: usize) -> Result<(), TransformError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| op_step(step, &format!("the output cannot be measured: {error}")))?;
    if bytes.len() > MAX_OUTPUT_BYTES {
        Err(TransformError::Bound {
            kind: BoundKind::Output,
            limit: MAX_OUTPUT_BYTES as u64,
        })
    } else {
        Ok(())
    }
}

fn op_step(step: usize, message: &str) -> TransformError {
    TransformError::Op {
        index: step,
        message: message.to_string(),
    }
}

fn expr_step(step: usize, message: String) -> TransformError {
    TransformError::Expr {
        index: step,
        message,
    }
}

/// What one node's evaluation carries between elements: the budget it has
/// spent and the `matches` patterns it has already compiled, so a constant
/// pattern is compiled once rather than once per element.
#[derive(Default)]
struct State {
    budget: u64,
    regexes: HashMap<String, Regex>,
}

/// One unit of the evaluation budget; the total may not pass
/// [`MAX_OPERATIONS`].
fn charge(state: &mut State) -> Result<(), TransformError> {
    state.budget += 1;
    if state.budget > MAX_OPERATIONS {
        Err(TransformError::Bound {
            kind: BoundKind::Operations,
            limit: MAX_OPERATIONS,
        })
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// The tokenizer
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Bang,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Contains,
    Matches,
    InSet,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Dot,
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(Number),
    Text(String),
    Name(String),
    Op(Op),
}

fn tokenize(source: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = source.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() {
            let (number, next) = number_at(&chars, i)?;
            tokens.push(Token::Number(number));
            i = next;
        } else if c == '\'' || c == '"' {
            let (text, next) = text_at(&chars, i, c)?;
            tokens.push(Token::Text(text));
            i = next;
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            tokens.push(Token::Name(chars[start..i].iter().collect()));
        } else {
            let (op, width) = operator_at(&chars, i)?;
            tokens.push(Token::Op(op));
            i += width;
        }
    }
    Ok(tokens)
}

fn operator_at(chars: &[char], i: usize) -> Result<(Op, usize), String> {
    let next = chars.get(i + 1).copied();
    match (chars[i], next) {
        ('=', Some('=')) => Ok((Op::Eq, 2)),
        ('!', Some('=')) => Ok((Op::Ne, 2)),
        ('<', Some('=')) => Ok((Op::Le, 2)),
        ('>', Some('=')) => Ok((Op::Ge, 2)),
        ('&', Some('&')) => Ok((Op::And, 2)),
        ('|', Some('|')) => Ok((Op::Or, 2)),
        ('+', _) => Ok((Op::Plus, 1)),
        ('-', _) => Ok((Op::Minus, 1)),
        ('*', _) => Ok((Op::Star, 1)),
        ('/', _) => Ok((Op::Slash, 1)),
        ('%', _) => Ok((Op::Percent, 1)),
        ('!', _) => Ok((Op::Bang, 1)),
        ('<', _) => Ok((Op::Lt, 1)),
        ('>', _) => Ok((Op::Gt, 1)),
        ('(', _) => Ok((Op::LParen, 1)),
        (')', _) => Ok((Op::RParen, 1)),
        ('[', _) => Ok((Op::LBracket, 1)),
        (']', _) => Ok((Op::RBracket, 1)),
        (',', _) => Ok((Op::Comma, 1)),
        ('.', _) => Ok((Op::Dot, 1)),
        (other, _) => Err(format!("unexpected character {other:?}")),
    }
}

fn number_at(chars: &[char], start: usize) -> Result<(Number, usize), String> {
    let mut i = start;
    while i < chars.len() && chars[i].is_ascii_digit() {
        i += 1;
    }
    let mut float = false;
    if i < chars.len() && chars[i] == '.' {
        float = true;
        i += 1;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
        float = true;
        i += 1;
        if i < chars.len() && (chars[i] == '+' || chars[i] == '-') {
            i += 1;
        }
        let digits = i;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
        if i == digits {
            return Err("an exponent needs digits".to_string());
        }
    }
    let text: String = chars[start..i].iter().collect();
    if !float {
        if let Ok(value) = text.parse::<i64>() {
            return Ok((Number::from(value), i));
        }
    }
    let value: f64 = text
        .parse()
        .map_err(|_| format!("{text:?} is not a number"))?;
    let number = Number::from_f64(value).ok_or_else(|| format!("{text:?} is not finite"))?;
    Ok((number, i))
}

fn text_at(chars: &[char], start: usize, quote: char) -> Result<(String, usize), String> {
    let mut text = String::new();
    let mut i = start + 1;
    while i < chars.len() {
        let c = chars[i];
        if c == quote {
            return Ok((text, i + 1));
        }
        if c == '\\' {
            i += 1;
            let escaped = *chars
                .get(i)
                .ok_or_else(|| "a string ended after a backslash".to_string())?;
            match escaped {
                '\\' => text.push('\\'),
                '\'' => text.push('\''),
                '"' => text.push('"'),
                other => return Err(format!("unknown escape \\{other}")),
            }
            i += 1;
            continue;
        }
        text.push(c);
        i += 1;
    }
    Err("a string is not closed".to_string())
}

// ---------------------------------------------------------------------------
// The parser
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Expr {
    Literal(Value),
    Path(Vec<String>),
    List(Vec<Expr>),
    Call {
        name: String,
        args: Vec<Expr>,
    },
    Not(Box<Expr>),
    Negate(Box<Expr>),
    Binary {
        op: Op,
        left: Box<Expr>,
        right: Box<Expr>,
    },
}

/// How many arguments each function name takes; anything else is unknown.
fn function_arity(name: &str) -> Option<usize> {
    match name {
        "lower" | "exists" => Some(1),
        "trunc" | "default" => Some(2),
        _ => None,
    }
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn parse(source: &str) -> Result<Expr, String> {
        let mut parser = Self {
            tokens: tokenize(source)?,
            pos: 0,
        };
        let expr = parser.expression()?;
        if parser.pos < parser.tokens.len() {
            return Err("a token is left over after the expression".to_string());
        }
        Ok(expr)
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn bump(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.pos).cloned();
        if token.is_some() {
            self.pos += 1;
        }
        token
    }

    fn eat(&mut self, op: Op) -> bool {
        if self.peek() == Some(&Token::Op(op)) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, op: Op, what: &str) -> Result<(), String> {
        if self.eat(op) {
            Ok(())
        } else {
            Err(format!("{what} is expected"))
        }
    }

    fn expression(&mut self) -> Result<Expr, String> {
        self.or()
    }

    fn or(&mut self) -> Result<Expr, String> {
        let mut expr = self.and()?;
        while self.eat(Op::Or) {
            expr = binary(Op::Or, expr, self.and()?);
        }
        Ok(expr)
    }

    fn and(&mut self) -> Result<Expr, String> {
        let mut expr = self.not()?;
        while self.eat(Op::And) {
            expr = binary(Op::And, expr, self.not()?);
        }
        Ok(expr)
    }

    fn not(&mut self) -> Result<Expr, String> {
        if self.eat(Op::Bang) {
            Ok(Expr::Not(Box::new(self.not()?)))
        } else {
            self.comparison()
        }
    }

    fn comparison(&mut self) -> Result<Expr, String> {
        let left = self.sum()?;
        let op = match self.peek() {
            Some(Token::Op(op @ (Op::Eq | Op::Ne | Op::Lt | Op::Le | Op::Gt | Op::Ge))) => *op,
            Some(Token::Name(name)) => match name.as_str() {
                "contains" => Op::Contains,
                "matches" => Op::Matches,
                "in_set" => Op::InSet,
                _ => return Ok(left),
            },
            _ => return Ok(left),
        };
        self.pos += 1;
        let right = self.sum()?;
        Ok(binary(op, left, right))
    }

    fn sum(&mut self) -> Result<Expr, String> {
        let mut expr = self.product()?;
        loop {
            let op = if self.eat(Op::Plus) {
                Op::Plus
            } else if self.eat(Op::Minus) {
                Op::Minus
            } else {
                return Ok(expr);
            };
            expr = binary(op, expr, self.product()?);
        }
    }

    fn product(&mut self) -> Result<Expr, String> {
        let mut expr = self.unary()?;
        loop {
            let op = if self.eat(Op::Star) {
                Op::Star
            } else if self.eat(Op::Slash) {
                Op::Slash
            } else if self.eat(Op::Percent) {
                Op::Percent
            } else {
                return Ok(expr);
            };
            expr = binary(op, expr, self.unary()?);
        }
    }

    fn unary(&mut self) -> Result<Expr, String> {
        if self.eat(Op::Minus) {
            Ok(Expr::Negate(Box::new(self.unary()?)))
        } else {
            self.atom()
        }
    }

    fn atom(&mut self) -> Result<Expr, String> {
        match self.bump() {
            Some(Token::Number(number)) => Ok(Expr::Literal(Value::Number(number))),
            Some(Token::Text(text)) => Ok(Expr::Literal(Value::String(text))),
            Some(Token::Name(name)) => self.name(name),
            Some(Token::Op(Op::LParen)) => {
                let expr = self.expression()?;
                self.expect(Op::RParen, "')'")?;
                Ok(expr)
            }
            Some(Token::Op(Op::LBracket)) => self.list(),
            Some(token) => Err(format!("{token:?} cannot start an expression")),
            None => Err("the expression ended early".to_string()),
        }
    }

    fn name(&mut self, name: String) -> Result<Expr, String> {
        if self.eat(Op::LParen) {
            return self.call(name);
        }
        match name.as_str() {
            "true" => return Ok(Expr::Literal(Value::Bool(true))),
            "false" => return Ok(Expr::Literal(Value::Bool(false))),
            "null" => return Ok(Expr::Literal(Value::Null)),
            _ => {}
        }
        let mut path = vec![name];
        while self.eat(Op::Dot) {
            match self.bump() {
                Some(Token::Name(segment)) => path.push(segment),
                _ => return Err("a field name is expected after '.'".to_string()),
            }
        }
        Ok(Expr::Path(path))
    }

    fn call(&mut self, name: String) -> Result<Expr, String> {
        let arity = function_arity(&name).ok_or_else(|| format!("unknown function {name:?}"))?;
        let mut args = Vec::new();
        if !self.eat(Op::RParen) {
            loop {
                args.push(self.expression()?);
                if self.eat(Op::Comma) {
                    continue;
                }
                self.expect(Op::RParen, "')'")?;
                break;
            }
        }
        if args.len() != arity {
            return Err(format!(
                "{name} takes {arity} argument(s), not {}",
                args.len()
            ));
        }
        Ok(Expr::Call { name, args })
    }

    fn list(&mut self) -> Result<Expr, String> {
        let mut items = Vec::new();
        if !self.eat(Op::RBracket) {
            loop {
                items.push(self.expression()?);
                if self.eat(Op::Comma) {
                    continue;
                }
                self.expect(Op::RBracket, "']'")?;
                break;
            }
        }
        Ok(Expr::List(items))
    }
}

fn binary(op: Op, left: Expr, right: Expr) -> Expr {
    Expr::Binary {
        op,
        left: Box::new(left),
        right: Box::new(right),
    }
}

/// A dotted path with no operators, for `exists` over an object.
fn parse_object_path(source: &str) -> Result<Vec<String>, String> {
    let tokens = tokenize(source)?;
    let mut segments = Vec::new();
    let mut i = 0;
    loop {
        match tokens.get(i) {
            Some(Token::Name(name)) => segments.push(name.clone()),
            _ => return Err("a dotted path is expected".to_string()),
        }
        i += 1;
        match tokens.get(i) {
            Some(Token::Op(Op::Dot)) => i += 1,
            Some(_) => return Err("only a dotted path is allowed here".to_string()),
            None => return Ok(segments),
        }
    }
}

// ---------------------------------------------------------------------------
// The evaluator
// ---------------------------------------------------------------------------

struct Eval<'a> {
    item: &'a Value,
    index: usize,
    step: usize,
    state: &'a mut State,
}

impl<'a> Eval<'a> {
    fn fail(&self, message: impl Into<String>) -> TransformError {
        expr_step(self.step, message.into())
    }

    fn charge(&mut self) -> Result<(), TransformError> {
        charge(self.state)
    }

    fn eval(&mut self, expr: &Expr) -> Result<Value, TransformError> {
        self.charge()?;
        match expr {
            Expr::Literal(value) => Ok(value.clone()),
            Expr::Path(path) => self.path(path),
            Expr::List(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.eval(item)?);
                }
                Ok(Value::Array(out))
            }
            Expr::Call { name, args } => self.call(name, args),
            Expr::Not(inner) => Ok(Value::Bool(!self.eval_bool(inner)?)),
            Expr::Negate(inner) => {
                let value = self.eval(inner)?;
                negate(&value).map_err(|message| self.fail(message))
            }
            Expr::Binary { op, left, right } => self.binary(*op, left, right),
        }
    }

    fn eval_bool(&mut self, expr: &Expr) -> Result<bool, TransformError> {
        match self.eval(expr)? {
            Value::Bool(value) => Ok(value),
            other => Err(self.fail(format!("a true or false is expected, not {other}"))),
        }
    }

    fn path(&mut self, path: &[String]) -> Result<Value, TransformError> {
        let (root, rest) = path
            .split_first()
            .ok_or_else(|| self.fail("an empty path"))?;
        let start = match root.as_str() {
            "item" => self.item.clone(),
            "index" => Value::Number(Number::from(self.index as u64)),
            other => return Err(self.fail(format!("unknown name {other:?}"))),
        };
        Ok(lookup_path(&start, rest))
    }

    fn call(&mut self, name: &str, args: &[Expr]) -> Result<Value, TransformError> {
        match name {
            "lower" => {
                let [arg] = args else {
                    return Err(self.fail("lower takes one argument"));
                };
                let value = self.eval(arg)?;
                let text = value
                    .as_str()
                    .ok_or_else(|| self.fail("lower needs a string"))?;
                Ok(Value::String(text.to_lowercase()))
            }
            "trunc" => {
                let [text, count] = args else {
                    return Err(self.fail("trunc takes two arguments"));
                };
                let value = self.eval(text)?;
                let text = value
                    .as_str()
                    .ok_or_else(|| self.fail("trunc needs a string"))?;
                let count = self.eval(count)?;
                let count = count
                    .as_u64()
                    .ok_or_else(|| self.fail("trunc needs a count of at least 0"))?;
                Ok(Value::String(text.chars().take(count as usize).collect()))
            }
            "default" => {
                let [value, fallback] = args else {
                    return Err(self.fail("default takes two arguments"));
                };
                let value = self.eval(value)?;
                if value.is_null() {
                    self.eval(fallback)
                } else {
                    Ok(value)
                }
            }
            "exists" => {
                let [arg] = args else {
                    return Err(self.fail("exists takes one argument"));
                };
                let value = self.eval(arg)?;
                Ok(Value::Bool(!value.is_null()))
            }
            other => Err(self.fail(format!("unknown function {other:?}"))),
        }
    }

    /// Whether `text` matches `pattern`. A pattern is compiled at most once per
    /// node: without the cache, a chain over many elements would pay the same
    /// compile for every one of them.
    fn matches(&mut self, pattern: &str, text: &str) -> Result<bool, TransformError> {
        if let Some(regex) = self.state.regexes.get(pattern) {
            return Ok(regex.is_match(text));
        }
        let regex = RegexBuilder::new(pattern)
            .size_limit(MAX_REGEX_BYTES)
            .build()
            .map_err(|error| self.fail(format!("bad regular expression: {error}")))?;
        let matched = regex.is_match(text);
        if self.state.regexes.len() < MAX_CACHED_PATTERNS {
            self.state.regexes.insert(pattern.to_string(), regex);
        }
        Ok(matched)
    }

    fn binary(&mut self, op: Op, left: &Expr, right: &Expr) -> Result<Value, TransformError> {
        match op {
            Op::And => {
                if !self.eval_bool(left)? {
                    return Ok(Value::Bool(false));
                }
                Ok(Value::Bool(self.eval_bool(right)?))
            }
            Op::Or => {
                if self.eval_bool(left)? {
                    return Ok(Value::Bool(true));
                }
                Ok(Value::Bool(self.eval_bool(right)?))
            }
            Op::Eq => {
                let left = self.eval(left)?;
                let right = self.eval(right)?;
                Ok(Value::Bool(equal(&left, &right)))
            }
            Op::Ne => {
                let left = self.eval(left)?;
                let right = self.eval(right)?;
                Ok(Value::Bool(!equal(&left, &right)))
            }
            Op::Lt | Op::Le | Op::Gt | Op::Ge => {
                let left = self.eval(left)?;
                let right = self.eval(right)?;
                let ordering = compare(&left, &right).map_err(|message| self.fail(message))?;
                let answer = match op {
                    Op::Lt => ordering.is_lt(),
                    Op::Le => ordering.is_le(),
                    Op::Gt => ordering.is_gt(),
                    _ => ordering.is_ge(),
                };
                Ok(Value::Bool(answer))
            }
            Op::Contains => {
                let left = self.eval(left)?;
                let right = self.eval(right)?;
                contains(&left, &right)
                    .map(Value::Bool)
                    .map_err(|message| self.fail(message))
            }
            Op::InSet => {
                let left = self.eval(left)?;
                let right = self.eval(right)?;
                in_set(&left, &right)
                    .map(Value::Bool)
                    .map_err(|message| self.fail(message))
            }
            Op::Matches => {
                let value = self.eval(left)?;
                let pattern = self.eval(right)?;
                let pattern = pattern
                    .as_str()
                    .ok_or_else(|| self.fail("matches needs a pattern string"))?;
                let text = value
                    .as_str()
                    .ok_or_else(|| self.fail("matches needs a string"))?;
                let matched = self.matches(pattern, text)?;
                Ok(Value::Bool(matched))
            }
            Op::Plus | Op::Minus | Op::Star | Op::Slash | Op::Percent => {
                let left = self.eval(left)?;
                let right = self.eval(right)?;
                arithmetic(op, &left, &right).map_err(|message| self.fail(message))
            }
            op => Err(self.fail(format!("{op:?} is not an infix operator"))),
        }
    }
}

fn eval_at<'a>(
    ast: &Expr,
    item: &'a Value,
    index: usize,
    step: usize,
    state: &'a mut State,
) -> Result<Value, TransformError> {
    Eval {
        item,
        index,
        step,
        state,
    }
    .eval(ast)
}

fn lookup_path(root: &Value, segments: &[String]) -> Value {
    let mut current = root;
    for segment in segments {
        match current.get(segment.as_str()) {
            Some(next) => current = next,
            None => return Value::Null,
        }
    }
    current.clone()
}

fn negate(value: &Value) -> Result<Value, String> {
    if let Some(integer) = value.as_i64() {
        if let Some(negated) = integer.checked_neg() {
            return Ok(Value::Number(Number::from(negated)));
        }
    }
    let number = value
        .as_f64()
        .ok_or_else(|| "'-' needs a number".to_string())?;
    Number::from_f64(-number)
        .map(Value::Number)
        .ok_or_else(|| "a number that is not finite".to_string())
}

fn arithmetic(op: Op, left: &Value, right: &Value) -> Result<Value, String> {
    let (x, y) = match (left.as_f64(), right.as_f64()) {
        (Some(x), Some(y)) => (x, y),
        _ => return Err("arithmetic needs numbers".to_string()),
    };
    if matches!(op, Op::Slash | Op::Percent) && y == 0.0 {
        return Err("division by zero".to_string());
    }
    if let (Some(a), Some(b)) = (left.as_i64(), right.as_i64()) {
        let integer = match op {
            Op::Plus => a.checked_add(b),
            Op::Minus => a.checked_sub(b),
            Op::Star => a.checked_mul(b),
            Op::Slash => {
                if a % b == 0 {
                    Some(a / b)
                } else {
                    None
                }
            }
            Op::Percent => Some(a % b),
            _ => None,
        };
        if let Some(value) = integer {
            return Ok(Value::Number(Number::from(value)));
        }
    }
    let result = match op {
        Op::Plus => x + y,
        Op::Minus => x - y,
        Op::Star => x * y,
        Op::Slash => x / y,
        Op::Percent => x % y,
        _ => return Err("unknown arithmetic".to_string()),
    };
    Number::from_f64(result)
        .map(Value::Number)
        .ok_or_else(|| "a number that is not finite".to_string())
}

fn equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(_), Value::Number(_)) => match (left.as_f64(), right.as_f64()) {
            (Some(x), Some(y)) => x == y,
            _ => false,
        },
        _ => left == right,
    }
}

fn compare(left: &Value, right: &Value) -> Result<Ordering, String> {
    match (left, right) {
        (Value::Number(_), Value::Number(_)) => {
            let x = left.as_f64().ok_or_else(|| "a number".to_string())?;
            let y = right.as_f64().ok_or_else(|| "a number".to_string())?;
            x.partial_cmp(&y)
                .ok_or_else(|| "two numbers that cannot be ordered".to_string())
        }
        (Value::String(x), Value::String(y)) => Ok(x.cmp(y)),
        (Value::Bool(x), Value::Bool(y)) => Ok(x.cmp(y)),
        _ => Err("ordering needs two numbers or two strings".to_string()),
    }
}

fn contains(haystack: &Value, needle: &Value) -> Result<bool, String> {
    match haystack {
        Value::String(text) => {
            let needle = needle
                .as_str()
                .ok_or_else(|| "contains on a string needs a string".to_string())?;
            Ok(text.contains(needle))
        }
        Value::Array(items) => Ok(items.iter().any(|item| equal(item, needle))),
        _ => Err("contains needs a string or a list".to_string()),
    }
}

fn in_set(value: &Value, set: &Value) -> Result<bool, String> {
    let items = set
        .as_array()
        .ok_or_else(|| "in_set needs a list after it".to_string())?;
    Ok(items.iter().any(|item| equal(item, value)))
}

/// The type a sort key must share with every other key before ordering is
/// defined; ordering across types is an expression error.
fn value_class(value: &Value) -> &'static str {
    match value {
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Bool(_) => "bool",
        Value::Null => "null",
        Value::Array(_) => "list",
        Value::Object(_) => "object",
    }
}

fn cmp_same_class(left: &Value, right: &Value) -> Ordering {
    match (left, right) {
        (Value::Number(_), Value::Number(_)) => left
            .as_f64()
            .and_then(|x| right.as_f64().and_then(|y| x.partial_cmp(&y)))
            .unwrap_or(Ordering::Equal),
        (Value::String(x), Value::String(y)) => x.cmp(y),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        _ => Ordering::Equal,
    }
}

fn as_list<'a>(input: &'a Value, step: usize, what: &str) -> Result<&'a [Value], TransformError> {
    input
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| op_step(step, &format!("{what} needs a list")))
}

fn parse_or(expr: &str, step: usize) -> Result<Expr, TransformError> {
    Parser::parse(expr).map_err(|message| expr_step(step, message))
}

fn parse_opt(by: Option<&str>, step: usize) -> Result<Option<Expr>, TransformError> {
    match by {
        Some(source) => Ok(Some(parse_or(source, step)?)),
        None => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// The operations
// ---------------------------------------------------------------------------

fn apply(
    op: &TransformOp,
    input: &Value,
    step: usize,
    state: &mut State,
) -> Result<Value, TransformError> {
    match op {
        TransformOp::Map { expr } => op_map(expr, input, step, state),
        TransformOp::Filter { expr } => op_filter(expr, input, step, state),
        TransformOp::Pick { fields } => op_select(fields, false, input, step, state),
        TransformOp::Omit { fields } => op_select(fields, true, input, step, state),
        TransformOp::Sort { by, desc } => op_sort(by, *desc, input, step, state),
        TransformOp::Group { by } => op_group(by, input, step, state),
        TransformOp::Unique { by } => op_unique(by.as_deref(), input, step, state),
        TransformOp::Slice { start, end } => op_slice(*start, *end, input, step),
        TransformOp::First => op_first(false, input, step, state),
        TransformOp::Last => op_first(true, input, step, state),
        TransformOp::Flatten => op_flatten(input, step, state),
        TransformOp::Join { sep } => op_join(sep, input, step, state),
        TransformOp::Split { sep } => op_split(sep, input, step),
        TransformOp::Merge { with } => op_merge(with, input, step),
        TransformOp::Count => op_count(input, step, state),
        TransformOp::Sum { by } => op_sum(by.as_deref(), input, step, state),
        TransformOp::Min { by } => op_extreme(by.as_deref(), false, input, step, state),
        TransformOp::Max { by } => op_extreme(by.as_deref(), true, input, step, state),
        TransformOp::Exists { expr } => op_exists(expr.as_deref(), input, step, state),
    }
}

fn op_map(
    source: &str,
    input: &Value,
    step: usize,
    state: &mut State,
) -> Result<Value, TransformError> {
    let items = as_list(input, step, "map")?;
    let ast = parse_or(source, step)?;
    let mut out = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        charge(state)?;
        out.push(eval_at(&ast, item, index, step, state)?);
    }
    Ok(Value::Array(out))
}

fn op_filter(
    source: &str,
    input: &Value,
    step: usize,
    state: &mut State,
) -> Result<Value, TransformError> {
    let items = as_list(input, step, "filter")?;
    let ast = parse_or(source, step)?;
    let mut out = Vec::new();
    for (index, item) in items.iter().enumerate() {
        charge(state)?;
        match eval_at(&ast, item, index, step, state)? {
            Value::Bool(true) => out.push(item.clone()),
            Value::Bool(false) => {}
            other => {
                return Err(expr_step(
                    step,
                    format!("filter needs true or false, not {other}"),
                ))
            }
        }
    }
    Ok(Value::Array(out))
}

fn op_select(
    fields: &[String],
    drop: bool,
    input: &Value,
    step: usize,
    state: &mut State,
) -> Result<Value, TransformError> {
    match input {
        Value::Object(map) => Ok(Value::Object(select_fields(map, fields, drop))),
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                charge(state)?;
                match item {
                    Value::Object(map) => out.push(Value::Object(select_fields(map, fields, drop))),
                    _ => return Err(op_step(step, "pick and omit need objects in the list")),
                }
            }
            Ok(Value::Array(out))
        }
        _ => Err(op_step(
            step,
            "pick and omit need an object or a list of objects",
        )),
    }
}

fn select_fields(map: &Map<String, Value>, fields: &[String], drop: bool) -> Map<String, Value> {
    map.iter()
        .filter(|(key, _)| fields.iter().any(|field| field == *key) != drop)
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn op_sort(
    by: &str,
    desc: bool,
    input: &Value,
    step: usize,
    state: &mut State,
) -> Result<Value, TransformError> {
    let items = as_list(input, step, "sort")?;
    let ast = parse_or(by, step)?;
    let mut keyed = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        charge(state)?;
        let key = eval_at(&ast, item, index, step, state)?;
        keyed.push((key, item.clone()));
    }
    if let Some((first, _)) = keyed.first() {
        let class = value_class(first);
        let orderable = matches!(class, "number" | "string" | "bool" | "null");
        if !orderable || keyed.iter().any(|(key, _)| value_class(key) != class) {
            return Err(expr_step(
                step,
                "sort needs keys of one orderable type".to_string(),
            ));
        }
    }
    // The keys are of one class by now, so the comparison cannot fail and the
    // sort stays stable for equal keys.
    keyed.sort_by(|(left, _), (right, _)| {
        let ordering = cmp_same_class(left, right);
        if desc {
            ordering.reverse()
        } else {
            ordering
        }
    });
    Ok(Value::Array(
        keyed.into_iter().map(|(_, item)| item).collect(),
    ))
}

fn op_group(
    by: &str,
    input: &Value,
    step: usize,
    state: &mut State,
) -> Result<Value, TransformError> {
    let items = as_list(input, step, "group")?;
    let ast = parse_or(by, step)?;
    let mut groups: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for (index, item) in items.iter().enumerate() {
        charge(state)?;
        let key = eval_at(&ast, item, index, step, state)?;
        let key = key_as_string(&key).map_err(|message| expr_step(step, message))?;
        groups.entry(key).or_default().push(item.clone());
    }
    let mut out = Map::new();
    for (key, group) in groups {
        out.insert(key, Value::Array(group));
    }
    Ok(Value::Object(out))
}

fn key_as_string(value: &Value) -> Result<String, String> {
    match value {
        Value::String(text) => Ok(text.clone()),
        Value::Number(number) => Ok(number.to_string()),
        Value::Bool(flag) => Ok(flag.to_string()),
        Value::Null => Ok("null".to_string()),
        _ => Err("a group key must be a string, a number or a boolean".to_string()),
    }
}

fn op_unique(
    by: Option<&str>,
    input: &Value,
    step: usize,
    state: &mut State,
) -> Result<Value, TransformError> {
    let items = as_list(input, step, "unique")?;
    let ast = parse_opt(by, step)?;
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for (index, item) in items.iter().enumerate() {
        charge(state)?;
        let key = match &ast {
            Some(ast) => eval_at(ast, item, index, step, state)?,
            None => item.clone(),
        };
        let canonical = serde_json::to_string(&key)
            .map_err(|error| op_step(step, &format!("a value that cannot be compared: {error}")))?;
        if seen.insert(canonical) {
            out.push(item.clone());
        }
    }
    Ok(Value::Array(out))
}

fn op_slice(
    start: usize,
    end: Option<usize>,
    input: &Value,
    step: usize,
) -> Result<Value, TransformError> {
    let items = as_list(input, step, "slice")?;
    let end = end.unwrap_or(items.len()).min(items.len());
    let start = start.min(end);
    Ok(Value::Array(items[start..end].to_vec()))
}

fn op_first(
    last: bool,
    input: &Value,
    step: usize,
    state: &mut State,
) -> Result<Value, TransformError> {
    let items = as_list(input, step, if last { "last" } else { "first" })?;
    charge(state)?;
    let found = if last { items.last() } else { items.first() };
    Ok(found.cloned().unwrap_or(Value::Null))
}

fn op_flatten(input: &Value, step: usize, state: &mut State) -> Result<Value, TransformError> {
    let items = as_list(input, step, "flatten")?;
    let mut out = Vec::new();
    for item in items {
        charge(state)?;
        match item {
            Value::Array(inner) => out.extend(inner.iter().cloned()),
            other => out.push(other.clone()),
        }
    }
    Ok(Value::Array(out))
}

fn op_join(
    sep: &str,
    input: &Value,
    step: usize,
    state: &mut State,
) -> Result<Value, TransformError> {
    let items = as_list(input, step, "join")?;
    let mut parts = Vec::with_capacity(items.len());
    for item in items {
        charge(state)?;
        match item {
            Value::String(text) => parts.push(text.clone()),
            Value::Number(number) => parts.push(number.to_string()),
            _ => return Err(op_step(step, "join needs strings or numbers")),
        }
    }
    Ok(Value::String(parts.join(sep)))
}

fn op_split(sep: &str, input: &Value, step: usize) -> Result<Value, TransformError> {
    let text = input
        .as_str()
        .ok_or_else(|| op_step(step, "split needs a string"))?;
    if sep.is_empty() {
        return Err(op_step(
            step,
            "split needs a separator of at least one character",
        ));
    }
    Ok(Value::Array(
        text.split(sep)
            .map(|part| Value::String(part.to_string()))
            .collect(),
    ))
}

fn op_merge(with: &Value, input: &Value, step: usize) -> Result<Value, TransformError> {
    let base = input
        .as_object()
        .ok_or_else(|| op_step(step, "merge needs an object"))?;
    let extra = with
        .as_object()
        .ok_or_else(|| op_step(step, "merge needs an object to merge"))?;
    let mut out = base.clone();
    for (key, value) in extra {
        out.insert(key.clone(), value.clone());
    }
    Ok(Value::Object(out))
}

fn op_count(input: &Value, step: usize, state: &mut State) -> Result<Value, TransformError> {
    match input {
        Value::Array(items) => {
            for _ in items {
                charge(state)?;
            }
            Ok(Value::Number(Number::from(items.len() as u64)))
        }
        Value::Object(map) => {
            for _ in map {
                charge(state)?;
            }
            Ok(Value::Number(Number::from(map.len() as u64)))
        }
        Value::String(text) => Ok(Value::Number(Number::from(text.chars().count() as u64))),
        _ => Err(op_step(step, "count needs a list, an object or a string")),
    }
}

fn op_sum(
    by: Option<&str>,
    input: &Value,
    step: usize,
    state: &mut State,
) -> Result<Value, TransformError> {
    let items = as_list(input, step, "sum")?;
    let ast = parse_opt(by, step)?;
    let mut whole: i128 = 0;
    let mut decimal = 0.0f64;
    let mut all_whole = true;
    for (index, item) in items.iter().enumerate() {
        charge(state)?;
        let value = match &ast {
            Some(ast) => eval_at(ast, item, index, step, state)?,
            None => item.clone(),
        };
        let number = match &value {
            Value::Number(number) => number,
            _ => return Err(op_step(step, "sum needs numbers")),
        };
        match number.as_i64() {
            Some(integer) => {
                whole += integer as i128;
                decimal += integer as f64;
            }
            None => {
                all_whole = false;
                decimal += number
                    .as_f64()
                    .ok_or_else(|| op_step(step, "sum needs numbers"))?;
            }
        }
    }
    if all_whole && whole >= i64::MIN as i128 && whole <= i64::MAX as i128 {
        Ok(Value::Number(Number::from(whole as i64)))
    } else {
        Number::from_f64(decimal)
            .map(Value::Number)
            .ok_or_else(|| op_step(step, "the sum is not finite"))
    }
}

fn op_extreme(
    by: Option<&str>,
    largest: bool,
    input: &Value,
    step: usize,
    state: &mut State,
) -> Result<Value, TransformError> {
    let items = as_list(input, step, if largest { "max" } else { "min" })?;
    let ast = parse_opt(by, step)?;
    let mut best: Option<(f64, Value)> = None;
    for (index, item) in items.iter().enumerate() {
        charge(state)?;
        let value = match &ast {
            Some(ast) => eval_at(ast, item, index, step, state)?,
            None => item.clone(),
        };
        let number = value
            .as_f64()
            .ok_or_else(|| op_step(step, "min and max need numbers"))?;
        let better = match &best {
            None => true,
            Some((chosen, _)) => {
                if largest {
                    number > *chosen
                } else {
                    number < *chosen
                }
            }
        };
        if better {
            best = Some((number, value));
        }
    }
    Ok(best.map(|(_, value)| value).unwrap_or(Value::Null))
}

fn op_exists(
    source: Option<&str>,
    input: &Value,
    step: usize,
    state: &mut State,
) -> Result<Value, TransformError> {
    match input {
        Value::Array(items) => match source {
            None => Ok(Value::Bool(!items.is_empty())),
            Some(source) => {
                let ast = parse_or(source, step)?;
                for (index, item) in items.iter().enumerate() {
                    charge(state)?;
                    match eval_at(&ast, item, index, step, state)? {
                        Value::Bool(true) => return Ok(Value::Bool(true)),
                        Value::Bool(false) => {}
                        other => {
                            return Err(expr_step(
                                step,
                                format!("exists needs true or false, not {other}"),
                            ))
                        }
                    }
                }
                Ok(Value::Bool(false))
            }
        },
        Value::Object(_) => {
            let source =
                source.ok_or_else(|| op_step(step, "exists needs a path for an object"))?;
            let path = parse_object_path(source).map_err(|message| expr_step(step, message))?;
            Ok(Value::Bool(
                walk_path(input, &path).is_some_and(|value| !value.is_null()),
            ))
        }
        _ => Err(op_step(step, "exists needs a list or an object")),
    }
}

fn walk_path<'a>(root: &'a Value, segments: &[String]) -> Option<&'a Value> {
    let mut current = root;
    for segment in segments {
        current = current.get(segment.as_str())?;
    }
    Some(current)
}

/// The expressions an op carries, for the validator and the canvas.
pub fn expressions_of(op: &TransformOp) -> Vec<&str> {
    match op {
        TransformOp::Map { expr } | TransformOp::Filter { expr } => vec![expr.as_str()],
        TransformOp::Sort { by, .. } | TransformOp::Group { by } => vec![by.as_str()],
        TransformOp::Unique { by }
        | TransformOp::Sum { by }
        | TransformOp::Min { by }
        | TransformOp::Max { by } => by.iter().map(String::as_str).collect(),
        TransformOp::Exists { expr } => expr.iter().map(String::as_str).collect(),
        _ => Vec::new(),
    }
}

/// Every op as it is written in a graph, with a few words for the canvas.
pub fn op_names() -> BTreeMap<&'static str, &'static str> {
    [
        ("map", "an expression per element"),
        ("filter", "keep where true"),
        ("pick", "keep fields"),
        ("omit", "drop fields"),
        ("sort", "by an expression"),
        ("group", "by an expression"),
        ("unique", "drop repeats"),
        ("slice", "start to end"),
        ("first", "first element"),
        ("last", "last element"),
        ("flatten", "one level"),
        ("join", "list to string"),
        ("split", "string to list"),
        ("merge", "object over object"),
        ("count", "how many"),
        ("sum", "total"),
        ("min", "smallest"),
        ("max", "largest"),
        ("exists", "any, or has a path"),
    ]
    .into_iter()
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn after(op: &TransformOp, input: &Tv) -> (bool, bool) {
        taint_out(op, input)
    }

    /// One expression over one item, as a `map` would read it.
    fn one(expr: &str, item: Value) -> Result<Value, TransformError> {
        let t: Transform =
            serde_json::from_value(json!({ "ops": [{ "op": "map", "expr": expr }] }))
                .expect("a valid chain");
        let out = evaluate(&t, &Tv::clean(json!([item])))?;
        Ok(out.v[0].clone())
    }

    fn value(expr: &str, item: Value) -> Value {
        one(expr, item).expect("the expression is valid")
    }

    /// Precedence and associativity as the module docs list them.
    #[test]
    fn the_parser_follows_the_precedence_of_the_grammar() {
        assert_eq!(value("1 + 2 * 3", Value::Null), json!(7));
        assert_eq!(value("(1 + 2) * 3", Value::Null), json!(9));
        assert_eq!(
            value("10 - 3 - 2", Value::Null),
            json!(5),
            "left associative"
        );
        assert_eq!(value("2 * 3 % 4", Value::Null), json!(2));
        assert_eq!(value("-3 * -2", Value::Null), json!(6));
        assert_eq!(value("!false", Value::Null), json!(true));
        assert_eq!(
            value("1 + 1 == 2", Value::Null),
            json!(true),
            "sum before comparison"
        );
        assert_eq!(value("'a' in_set ['a', 'b']", Value::Null), json!(true));
        assert_eq!(value("lower('AB')", Value::Null), json!("ab"));
    }

    #[test]
    fn a_parse_error_stops_the_chain_and_names_its_step() {
        assert!(matches!(
            one("item +", Value::Null),
            Err(TransformError::Expr { index: 0, .. })
        ));
        assert!(matches!(
            one("1 2", Value::Null),
            Err(TransformError::Expr { index: 0, .. })
        ));
        assert!(matches!(
            one("1 < 2 < 3", Value::Null),
            Err(TransformError::Expr { index: 0, .. })
        ));
        assert!(matches!(
            one("lower(1, 2)", Value::Null),
            Err(TransformError::Expr { index: 0, .. })
        ));
        assert!(matches!(
            one("item in_set", Value::Null),
            Err(TransformError::Expr { index: 0, .. })
        ));
        assert!(matches!(
            one("item.", Value::Null),
            Err(TransformError::Expr { index: 0, .. })
        ));
        let later: Transform = serde_json::from_value(json!({ "ops": [
            { "op": "filter", "expr": "true" },
            { "op": "map", "expr": "1 / 0" }
        ] }))
        .expect("a valid chain");
        let error = evaluate(&later, &Tv::clean(json!([1]))).expect_err("the second step fails");
        assert!(
            matches!(error, TransformError::Expr { index: 1, .. }),
            "{error:?}"
        );
    }

    #[test]
    fn a_missing_path_is_null_and_a_stray_name_is_an_error() {
        assert_eq!(value("item.a.b", json!({})), Value::Null);
        assert_eq!(value("item.a.b", json!({ "a": { "b": 2 } })), json!(2));
        assert_eq!(value("index", json!("x")), json!(0));
        assert!(matches!(
            one("other", Value::Null),
            Err(TransformError::Expr { index: 0, .. })
        ));
        assert_eq!(
            value("exists(item.a.b)", json!({ "a": { "b": null } })),
            json!(false)
        );
    }

    /// The taint table of §3.9 and the derived-value rule of §4.5.
    #[test]
    fn the_taint_table() {
        let tainted_list = Tv::tainted(json!(["a", "b"]));
        let clean_list = Tv::clean(json!([1, 2]));
        let derived_num = Tv::derived(json!(2));
        // Results that cannot carry text are clean over tainted input, and derived.
        for op in [
            TransformOp::Count,
            TransformOp::Sum { by: None },
            TransformOp::Min { by: None },
            TransformOp::Max { by: None },
            TransformOp::Exists { expr: None },
        ] {
            assert_eq!(
                after(&op, &tainted_list),
                (false, true),
                "{op:?} over tainted"
            );
            assert_eq!(after(&op, &clean_list), (false, false), "{op:?} over clean");
            assert_eq!(
                after(&op, &derived_num),
                (false, true),
                "{op:?} over derived"
            );
        }
        // Everything else carries the input's bits.
        for op in [
            TransformOp::Map {
                expr: "item".into(),
            },
            TransformOp::Filter {
                expr: "true".into(),
            },
            TransformOp::Pick { fields: vec![] },
            TransformOp::Omit { fields: vec![] },
            TransformOp::Sort {
                by: "item".into(),
                desc: false,
            },
            TransformOp::Group { by: "item".into() },
            TransformOp::Unique { by: None },
            TransformOp::Slice {
                start: 0,
                end: None,
            },
            TransformOp::First,
            TransformOp::Last,
            TransformOp::Flatten,
            TransformOp::Join { sep: ",".into() },
            TransformOp::Split { sep: ",".into() },
            TransformOp::Merge { with: json!({}) },
        ] {
            assert_eq!(
                after(&op, &tainted_list),
                (true, false),
                "{op:?} over tainted"
            );
            assert_eq!(after(&op, &clean_list), (false, false), "{op:?} over clean");
            assert_eq!(
                after(&op, &derived_num),
                (false, true),
                "{op:?} over derived"
            );
        }
    }

    #[test]
    fn a_chain_keeps_the_derivation_after_a_count() {
        let tainted = Tv::tainted(json!(["a", "b"]));
        let ops = [
            TransformOp::Filter {
                expr: "true".into(),
            },
            TransformOp::Count,
        ];
        assert_eq!(chain_taint(&ops, &tainted), (false, true));
        // A sum of a count is still derived; text never comes back.
        let more = [TransformOp::Count, TransformOp::Sum { by: None }];
        assert_eq!(chain_taint(&more, &tainted), (false, true));
        assert_eq!(chain_taint(&ops, &Tv::clean(json!([1]))), (false, false));
        assert_eq!(chain_taint(&[], &tainted), (true, false));
    }

    #[test]
    fn a_chain_reads_from_and_writes_to_json() {
        let t: Transform = serde_json::from_value(json!({ "ops": [
            { "op": "filter", "expr": "item.n > 1" },
            { "op": "sort", "by": "item.n", "desc": true },
            { "op": "slice", "start": 0, "end": 2 },
            { "op": "count" }
        ] }))
        .unwrap();
        assert_eq!(t.ops.len(), 4);
        assert_eq!(
            serde_json::from_value::<Transform>(serde_json::to_value(&t).unwrap()).unwrap(),
            t
        );
        assert!(
            serde_json::from_value::<Transform>(json!({ "ops": [{ "op": "eval", "expr": "1" }] }))
                .is_err(),
            "no op beyond the list"
        );
    }

    #[test]
    fn every_op_has_a_name_and_the_expressions_of_a_chain_can_be_listed() {
        assert_eq!(op_names().len(), 19);
        let ops = vec![
            TransformOp::Map {
                expr: "item.a".into(),
            },
            TransformOp::Sort {
                by: "item.b".into(),
                desc: false,
            },
            TransformOp::Count,
        ];
        let exprs: Vec<&str> = ops.iter().flat_map(expressions_of).collect();
        assert_eq!(exprs, ["item.a", "item.b"]);
    }
}
