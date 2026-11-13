// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Transform expression language: the tokenizer, the parser and the
//! evaluator (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md §3.9).

use std::cmp::Ordering;
use std::collections::HashMap;

use regex::{Regex, RegexBuilder};
use serde_json::{Number, Value};

use super::{
    expr_step, op_step, BoundKind, TransformError, MAX_CACHED_PATTERNS, MAX_OPERATIONS,
    MAX_REGEX_BYTES,
};

/// An expression's source may be at most this long. The grammar is small, and
/// a bound keeps the tokenizer and parser work finite; the size of a `matches`
/// pattern is bounded separately, where the pattern is compiled.
const MAX_EXPRESSION_BYTES: usize = 64 * 1024;
/// How deeply a source may nest `(`, `[`, `!` and `-`. The parser is
/// recursive, so a source nested further is an expression error rather than a
/// stack overflow.
const MAX_NESTING_DEPTH: usize = 64;
/// How deep an expression tree may be before evaluation stops: a chain such as
/// `1 + 1 + ...` is not nested in the source but is deep in the tree.
const MAX_EVAL_DEPTH: usize = 256;

/// What one node's evaluation carries between elements: the budget it has
/// spent and the `matches` patterns it has already compiled, so a constant
/// pattern is compiled once rather than once per element.
#[derive(Default)]
pub(super) struct State {
    budget: u64,
    regexes: HashMap<String, Regex>,
}

/// One unit of the evaluation budget; the total may not pass
/// [`MAX_OPERATIONS`].
pub(super) fn charge(state: &mut State) -> Result<(), TransformError> {
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
pub(super) enum Op {
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
pub(super) enum Expr {
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
    depth: usize,
}

impl Parser {
    fn parse(source: &str) -> Result<Expr, String> {
        if source.len() > MAX_EXPRESSION_BYTES {
            return Err(format!(
                "an expression may be at most {MAX_EXPRESSION_BYTES} bytes"
            ));
        }
        let mut parser = Self {
            tokens: tokenize(source)?,
            pos: 0,
            depth: 0,
        };
        let expr = parser.expression()?;
        if parser.pos < parser.tokens.len() {
            return Err("a token is left over after the expression".to_string());
        }
        Ok(expr)
    }

    /// Count one level of nesting; past the limit the source is refused rather
    /// than parsed until the stack runs out. An error abandons the parse, so a
    /// depth is not given back on that path.
    fn enter(&mut self) -> Result<(), String> {
        self.depth += 1;
        if self.depth > MAX_NESTING_DEPTH {
            return Err(format!(
                "an expression may nest at most {MAX_NESTING_DEPTH} levels"
            ));
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
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
        self.enter()?;
        let expr = self.or()?;
        self.leave();
        Ok(expr)
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
            self.enter()?;
            let inner = self.not();
            self.leave();
            Ok(Expr::Not(Box::new(inner?)))
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
            self.enter()?;
            let inner = self.unary();
            self.leave();
            Ok(Expr::Negate(Box::new(inner?)))
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
pub(super) fn parse_object_path(source: &str) -> Result<Vec<String>, String> {
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
    depth: usize,
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
        self.depth += 1;
        if self.depth > MAX_EVAL_DEPTH {
            return Err(self.fail(format!(
                "an expression may be evaluated at most {MAX_EVAL_DEPTH} levels deep"
            )));
        }
        let answer = self.eval_node(expr);
        self.depth -= 1;
        answer
    }

    fn eval_node(&mut self, expr: &Expr) -> Result<Value, TransformError> {
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

pub(super) fn eval_at<'a>(
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
        depth: 0,
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
        // `i64::MIN / -1` and `i64::MIN % -1` overflow; checked division
        // returns `None` for them and the float path below handles the value.
        let integer = match op {
            Op::Plus => a.checked_add(b),
            Op::Minus => a.checked_sub(b),
            Op::Star => a.checked_mul(b),
            Op::Slash => match a.checked_rem(b) {
                Some(0) => a.checked_div(b),
                _ => None,
            },
            Op::Percent => a.checked_rem(b),
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
pub(super) fn value_class(value: &Value) -> &'static str {
    match value {
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Bool(_) => "bool",
        Value::Null => "null",
        Value::Array(_) => "list",
        Value::Object(_) => "object",
    }
}

pub(super) fn cmp_same_class(left: &Value, right: &Value) -> Ordering {
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

pub(super) fn as_list<'a>(
    input: &'a Value,
    step: usize,
    what: &str,
) -> Result<&'a [Value], TransformError> {
    input
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| op_step(step, &format!("{what} needs a list")))
}

pub(super) fn parse_or(expr: &str, step: usize) -> Result<Expr, TransformError> {
    Parser::parse(expr).map_err(|message| expr_step(step, message))
}

pub(super) fn parse_opt(by: Option<&str>, step: usize) -> Result<Option<Expr>, TransformError> {
    match by {
        Some(source) => Ok(Some(parse_or(source, step)?)),
        None => Ok(None),
    }
}
