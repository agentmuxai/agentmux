// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Structured output of the Agent block (SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md §4.1, §6.3).
//!
//! A node may declare `output_schema`: the few short fields its reply must carry, so the next node branches on
//! a field (`{{<id>.output.confidence}}`) and not on prose. The block adds the reply format to the prompt, takes the
//! JSON object out of the reply, and checks it against the schema; a reply that does not fit fails the node.
//!
//! The schema is an object of `field: spec`. A spec is one of:
//!   * `"string"`, `"number"`, `"boolean"` (a string is cut to [`DEFAULT_MAX`] characters);
//!   * `{"type": "string", "max": N}`: a string cut to N characters (N is at most [`LIMIT_MAX`]);
//!   * `{"enum": ["ok", "findings", "failed"]}`: one of a closed set, which is what makes a value clean to branch on.
//!
//! Every declared field is required, and a field the schema does not declare is dropped. A string is one line:
//! its line breaks and control characters become spaces, so a field stays a short label, never a block of
//! prose carrying someone else's words onward.

use serde_json::{json, Map, Value};

/// How many characters a string field keeps unless its spec says less or more.
pub const DEFAULT_MAX: usize = 200;
/// The most characters any string field may keep: the fields are short by design.
pub const LIMIT_MAX: usize = 500;
/// The most fields a schema may declare.
pub const MAX_FIELDS: usize = 12;

#[derive(Debug, Clone, PartialEq)]
enum Kind {
    Text(usize),
    Number,
    Bool,
    OneOf(Vec<String>),
}

#[derive(Debug, Clone, PartialEq)]
struct Field {
    name: String,
    kind: Kind,
}

/// A parsed `output_schema`.
#[derive(Debug, Clone, PartialEq)]
pub struct Schema {
    fields: Vec<Field>,
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase() || c == '_')
        && name.len() <= 32
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn parse_kind(name: &str, spec: &Value) -> Result<Kind, String> {
    match spec {
        Value::String(t) => match t.as_str() {
            "string" => Ok(Kind::Text(DEFAULT_MAX)),
            "number" => Ok(Kind::Number),
            "boolean" => Ok(Kind::Bool),
            other => Err(format!("field `{name}`: unknown type `{other}` (string, number or boolean)")),
        },
        Value::Object(o) => {
            if let Some(values) = o.get("enum") {
                let list = values.as_array().filter(|a| !a.is_empty() && a.len() <= 20);
                let list = list.ok_or_else(|| format!("field `{name}`: `enum` is a list of 1 to 20 values"))?;
                let strings: Option<Vec<String>> = list.iter().map(|v| v.as_str().filter(|s| !s.is_empty()).map(str::to_string)).collect();
                return strings.map(Kind::OneOf).ok_or_else(|| format!("field `{name}`: `enum` holds non-empty strings"));
            }
            match o.get("type").and_then(Value::as_str) {
                Some("string") => {
                    let max = match o.get("max") {
                        None => DEFAULT_MAX,
                        Some(m) => m
                            .as_u64()
                            .filter(|n| (1..=LIMIT_MAX as u64).contains(n))
                            .ok_or_else(|| format!("field `{name}`: `max` is 1 to {LIMIT_MAX}"))? as usize,
                    };
                    Ok(Kind::Text(max))
                }
                Some(other) => parse_kind(name, &Value::String(other.to_string())),
                None => Err(format!("field `{name}`: a spec has `type` or `enum`")),
            }
        }
        _ => Err(format!("field `{name}`: a spec is a type name or an object")),
    }
}

impl Schema {
    /// Reads a node's `output_schema`. An error names the field at fault, so it can be shown beside the node.
    pub fn parse(declared: &Value) -> Result<Schema, String> {
        let map = declared.as_object().ok_or("`output_schema` is an object of field: type")?;
        if map.is_empty() {
            return Err("`output_schema` declares no field".to_string());
        }
        if map.len() > MAX_FIELDS {
            return Err(format!("`output_schema` declares {} fields; at most {MAX_FIELDS}", map.len()));
        }
        let mut fields = Vec::with_capacity(map.len());
        for (name, spec) in map {
            if !valid_name(name) {
                return Err(format!("field `{name}`: a name is lowercase letters, digits and `_`, starting with a letter or `_`, 32 at most"));
            }
            fields.push(Field { name: name.clone(), kind: parse_kind(name, spec)? });
        }
        Ok(Schema { fields })
    }

    /// What the prompt adds so the reply is the object the schema wants.
    pub fn instructions(&self) -> String {
        let mut out = String::from(
            "\n\nEnd your reply with one JSON object and nothing after it. It has exactly these fields, all required:\n",
        );
        for f in &self.fields {
            let what = match &f.kind {
                Kind::Text(max) => format!("a short text, one line, at most {max} characters"),
                Kind::Number => "a number".to_string(),
                Kind::Bool => "true or false".to_string(),
                Kind::OneOf(values) => format!("one of {}", values.iter().map(|v| format!("{v:?}")).collect::<Vec<_>>().join(", ")),
            };
            out.push_str(&format!("- {}: {what}\n", f.name));
        }
        out
    }

    /// The reply's last JSON object that fits the schema, as the fields the schema declares.
    /// The error says which field was wrong and how, never what the reply said: that is the agent's text.
    pub fn extract(&self, reply: &str) -> Result<Value, String> {
        let candidates = json_objects(reply);
        if candidates.is_empty() {
            return Err("the reply holds no JSON object".to_string());
        }
        let mut first_problem = None;
        for object in candidates.iter().rev() {
            match self.check(object) {
                Ok(fields) => return Ok(Value::Object(fields)),
                Err(problem) => {
                    first_problem.get_or_insert(problem);
                }
            }
        }
        Err(first_problem.unwrap_or_default())
    }

    fn check(&self, object: &Map<String, Value>) -> Result<Map<String, Value>, String> {
        let mut out = Map::new();
        for f in &self.fields {
            let value = object.get(&f.name).ok_or_else(|| format!("field `{}` is missing", f.name))?;
            let checked = match (&f.kind, value) {
                (Kind::Text(max), Value::String(s)) => json!(one_line(s, *max)),
                (Kind::Number, Value::Number(n)) if n.as_f64().is_some_and(f64::is_finite) => value.clone(),
                (Kind::Bool, Value::Bool(_)) => value.clone(),
                (Kind::OneOf(values), Value::String(s)) if values.iter().any(|v| v == s) => value.clone(),
                (Kind::OneOf(_), Value::String(_)) => return Err(format!("field `{}` is not one of its allowed values", f.name)),
                (kind, _) => return Err(format!("field `{}` is not {}", f.name, article(kind))),
            };
            out.insert(f.name.clone(), checked);
        }
        Ok(out)
    }
}

fn article(kind: &Kind) -> &'static str {
    match kind {
        Kind::Text(_) => "a string",
        Kind::Number => "a number",
        Kind::Bool => "a boolean",
        Kind::OneOf(_) => "a string",
    }
}

/// One line of at most `max` characters: breaks and control characters become spaces, runs of spaces one space.
fn one_line(s: &str, max: usize) -> String {
    let mut out = String::new();
    let mut space = true;
    for c in s.chars() {
        let c = if c.is_control() { ' ' } else { c };
        if c == ' ' {
            if !space {
                out.push(' ');
            }
            space = true;
        } else {
            out.push(c);
            space = false;
        }
    }
    let trimmed = out.trim_end();
    trimmed.chars().take(max).collect::<String>().trim_end().to_string()
}

/// The most of a reply's end that is searched for the object, and the most places in it tried.
const SEARCH_BYTES: usize = 64 * 1024;
const MAX_TRIES: usize = 400;

/// Every JSON object in the end of the text, in order: each of the last `MAX_TRIES` `{` that starts one that parses
/// (the text after it is ignored), skipping the ones inside an object already found. A stray `{` in prose costs one try.
fn json_objects(text: &str) -> Vec<Map<String, Value>> {
    let mut from = text.len().saturating_sub(SEARCH_BYTES);
    while !text.is_char_boundary(from) {
        from += 1;
    }
    let text = &text[from..];
    let opens: Vec<usize> = text.match_indices('{').map(|(i, _)| i).collect();
    let mut found = Vec::new();
    let mut skip_to = 0;
    for &i in &opens[opens.len().saturating_sub(MAX_TRIES)..] {
        if i < skip_to {
            continue;
        }
        let mut stream = serde_json::Deserializer::from_str(&text[i..]).into_iter::<Value>();
        if let Some(Ok(Value::Object(o))) = stream.next() {
            skip_to = i + stream.byte_offset();
            found.push(o);
        }
    }
    found
}

#[cfg(test)]
#[path = "agent_output_tests.rs"]
mod tests;
