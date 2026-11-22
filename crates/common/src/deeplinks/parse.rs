// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! srv's and mcp's parser of deep links (docs/specs/SPEC_DEEP_LINKS_ONE_FRAMEWORK_2026_11_20.md §3.2, R6 to R9,
//! G7, G8). The app has the same parser in TypeScript (`frontend/app/deeplinks/parse.ts`); both are written from
//! the one grammar and the one registry, and both run `crates/common/deeplinks/conformance.json`. A change here is
//! a change there.
//!
//! The order of the checks is part of the contract, so both sides give the same reason for the same link:
//! 1. length, then the scheme;
//! 2. the shape: view, segments, query and anchor, no empty part;
//! 3. each part, left to right: its characters, its percent-encoding (decoded once), control characters, its one
//!    written form;
//! 4. the view (an alias is rewritten first), its build, the place, the parameters, the anchor.

use std::collections::BTreeMap;

use super::types::{Kind, Place, Registry, Segment, View};

pub const SCHEME: &str = "agentmux://";

/// Why a link is refused. `as_str` is the code the conformance file names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkError {
    TooLong,
    NotAgentmux,
    BadSyntax,
    BadEncoding,
    DoubleEncoded,
    ControlChar,
    NotCanonical,
    UnknownView,
    UnknownPlace,
    UnknownParam,
    UnknownAnchor,
    DuplicateParam,
    EmptyParam,
    BadValue,
    BadPath,
    MissingPlace,
    NotYet,
}

impl LinkError {
    pub fn as_str(self) -> &'static str {
        match self {
            LinkError::TooLong => "too_long",
            LinkError::NotAgentmux => "not_agentmux",
            LinkError::BadSyntax => "bad_syntax",
            LinkError::BadEncoding => "bad_encoding",
            LinkError::DoubleEncoded => "double_encoded",
            LinkError::ControlChar => "control_char",
            LinkError::NotCanonical => "not_canonical",
            LinkError::UnknownView => "unknown_view",
            LinkError::UnknownPlace => "unknown_place",
            LinkError::UnknownParam => "unknown_param",
            LinkError::UnknownAnchor => "unknown_anchor",
            LinkError::DuplicateParam => "duplicate_param",
            LinkError::EmptyParam => "empty_param",
            LinkError::BadValue => "bad_value",
            LinkError::BadPath => "bad_path",
            LinkError::MissingPlace => "missing_place",
            LinkError::NotYet => "not_yet",
        }
    }
}

/// A value of a var or a parameter: a number for an `int`, text for every other kind.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Value {
    Text(String),
    Int(i64),
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Text(s) => f.write_str(s),
            Value::Int(n) => write!(f, "{n}"),
        }
    }
}

/// A link read: its view, the place it matched, the values of the place's vars, its parameters and anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedLink {
    /// The registry view (`ext` for a widget).
    pub view: &'static View,
    /// The place it matched, or None for the view as it is.
    pub place: Option<&'static Place>,
    /// Each var of the place by name; for a widget, `package` too.
    pub values: BTreeMap<String, Value>,
    pub params: BTreeMap<String, Value>,
    pub anchor: Option<String>,
    /// The link's one written form (R7).
    pub canonical: String,
}

type R<T> = Result<T, LinkError>;

fn is_unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~')
}

/// C0 and C1 controls, DEL, the bidirectional marks, embeddings, overrides and isolates, zero-width characters (G8).
pub fn is_control(c: char) -> bool {
    let u = c as u32;
    u <= 0x1F
        || (0x7F..=0x9F).contains(&u)
        || u == 0x061C
        || (0x200B..=0x200F).contains(&u)
        || (0x202A..=0x202E).contains(&u)
        || (0x2060..=0x2069).contains(&u)
        || u == 0xFEFF
}

/// The one percent-encoding of `text` (R7): unreserved characters as they are, every other byte as `%HH`, upper case.
pub fn encode_part(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for &b in text.as_bytes() {
        if is_unreserved(b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn has_pct_triplet(text: &str) -> bool {
    let b = text.as_bytes();
    b.windows(3).any(|w| w[0] == b'%' && w[1].is_ascii_hexdigit() && w[2].is_ascii_hexdigit())
}

/// `raw` decoded once, with every check of step 3.
fn decode_part(raw: &str) -> R<String> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'%' {
            let hex = bytes.get(i + 1..i + 3).ok_or(LinkError::BadEncoding)?;
            if !hex.iter().all(u8::is_ascii_hexdigit) {
                return Err(LinkError::BadEncoding);
            }
            let s = std::str::from_utf8(hex).map_err(|_| LinkError::BadEncoding)?;
            out.push(u8::from_str_radix(s, 16).map_err(|_| LinkError::BadEncoding)?);
            i += 3;
        } else if is_unreserved(b) {
            out.push(b);
            i += 1;
        } else {
            return Err(LinkError::BadSyntax);
        }
    }
    let text = String::from_utf8(out).map_err(|_| LinkError::BadEncoding)?;
    if has_pct_triplet(&text) {
        return Err(LinkError::DoubleEncoded);
    }
    if text.chars().any(is_control) {
        return Err(LinkError::ControlChar);
    }
    if encode_part(&text) != raw {
        return Err(LinkError::NotCanonical);
    }
    Ok(text)
}

fn all_chars(s: &str, first: impl Fn(char) -> bool, rest: impl Fn(char) -> bool, max: usize) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if first(c) => {}
        _ => return false,
    }
    s.chars().count() <= max && chars.all(rest)
}

fn lower_alnum(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit()
}

fn is_id(s: &str) -> bool {
    all_chars(s, lower_alnum, |c| lower_alnum(c) || matches!(c, '.' | '_' | '-'), 128)
}

fn is_slug(s: &str) -> bool {
    all_chars(s, lower_alnum, |c| lower_alnum(c) || c == '-', 24)
}

fn is_param_key(s: &str) -> bool {
    all_chars(s, |c| c.is_ascii_lowercase(), |c| lower_alnum(c) || c == '_', 32)
}

fn is_opaque(s: &str) -> bool {
    s.strip_prefix("e_").is_some_and(|r| (8..=32).contains(&r.len()) && r.chars().all(lower_alnum))
}

fn is_package(s: &str) -> bool {
    let part = |p: &str| all_chars(p, lower_alnum, |c| lower_alnum(c) || c == '-', usize::MAX);
    match s.split_once('.') {
        Some((a, b)) => part(a) && part(b),
        None => false,
    }
}

/// Windows device names, which open a device whatever folder or extension they are written with.
fn is_device(base: &str) -> bool {
    let l = base.to_lowercase();
    if matches!(l.as_str(), "con" | "prn" | "aux" | "nul" | "conin$" | "conout$") {
        return true;
    }
    let mut chars = l.chars();
    let head: String = chars.by_ref().take(3).collect();
    let tail: Vec<char> = chars.collect();
    (head == "com" || head == "lpt") && tail.len() == 1 && (tail[0].is_ascii_digit() || matches!(tail[0], '\u{b9}' | '\u{b2}' | '\u{b3}'))
}

/// Why `path` may not be opened by a link, as a structural check both sides make the same way (G7), or None. srv
/// applies the rest of G7 (what the caller may read, where a symbolic link leads) when the link is opened.
pub fn path_problem(path: &str) -> Option<LinkError> {
    if path.contains('\\') {
        return Some(LinkError::BadPath);
    }
    let b = path.as_bytes();
    let rest = if b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'/' {
        &path[3..]
    } else if path.starts_with('/') && !path.starts_with("//") {
        &path[1..]
    } else {
        // Relative, a drive-relative `C:x`, or a UNC `//server`.
        return Some(LinkError::BadPath);
    };
    if rest.is_empty() {
        return None;
    }
    for seg in rest.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." || seg.contains(':') || seg.ends_with('.') || seg.ends_with(' ') {
            return Some(LinkError::BadPath);
        }
        if is_device(seg.split('.').next().unwrap_or("")) {
            return Some(LinkError::BadPath);
        }
    }
    None
}

fn name_problem(name: &str) -> bool {
    name.chars().count() > 255
        || name == "."
        || name == ".."
        || name.contains(['/', '\\', ':'])
        || is_device(name.split('.').next().unwrap_or(""))
        || name.ends_with(' ')
        || name.ends_with('.')
}

struct Typed<'a> {
    kind: Kind,
    values: &'a [super::types::EnumValue],
    min: Option<i64>,
    max: Option<i64>,
}

/// `text` read as a value of `spec`'s kind. An enum value that isn't one is the caller's to name.
fn typed_value(spec: &Typed<'_>, text: &str, enum_miss: LinkError) -> R<Value> {
    let lower = text.to_lowercase();
    match spec.kind {
        Kind::Enum => {
            if spec.values.iter().any(|v| v.id == text) {
                Ok(Value::Text(text.to_string()))
            } else if spec.values.iter().any(|v| v.id == lower) {
                Err(LinkError::NotCanonical)
            } else {
                Err(enum_miss)
            }
        }
        Kind::Id | Kind::Slug => {
            let check = if spec.kind == Kind::Id { is_id } else { is_slug };
            if check(text) {
                Ok(Value::Text(text.to_string()))
            } else if check(&lower) {
                Err(LinkError::NotCanonical)
            } else {
                Err(LinkError::BadValue)
            }
        }
        Kind::Int => {
            let digits = !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
            if digits && text.len() > 1 && text.starts_with('0') {
                return Err(LinkError::NotCanonical);
            }
            if !digits || text.len() > 10 {
                return Err(LinkError::BadValue);
            }
            let n: i64 = text.parse().map_err(|_| LinkError::BadValue)?;
            if n < spec.min.unwrap_or(0) || n > spec.max.unwrap_or(2_147_483_647) {
                return Err(LinkError::BadValue);
            }
            Ok(Value::Int(n))
        }
        Kind::Opaque => is_opaque(text).then(|| Value::Text(text.to_string())).ok_or(LinkError::BadValue),
        Kind::Path => match path_problem(text) {
            Some(e) => Err(e),
            None => Ok(Value::Text(text.to_string())),
        },
        Kind::Name => (!name_problem(text)).then(|| Value::Text(text.to_string())).ok_or(LinkError::BadValue),
    }
}

struct Shape {
    head: String,
    segments: Vec<String>,
    params: Vec<(String, String)>,
    anchor: Option<String>,
}

/// Step 2 and 3: the link cut into its parts, each decoded once.
fn shape_of(link: &str, reg: &Registry) -> R<Shape> {
    if link.len() > reg.max_bytes {
        return Err(LinkError::TooLong);
    }
    let mut rest = link.strip_prefix(SCHEME).ok_or(LinkError::NotAgentmux)?;
    let mut anchor_raw = None;
    if let Some((before, after)) = rest.split_once('#') {
        if after.is_empty() || after.contains('#') {
            return Err(LinkError::BadSyntax);
        }
        anchor_raw = Some(after);
        rest = before;
    }
    let mut query_raw = None;
    if let Some((before, after)) = rest.split_once('?') {
        if after.is_empty() || after.contains('?') {
            return Err(LinkError::BadSyntax);
        }
        query_raw = Some(after);
        rest = before;
    }
    let raw_segments: Vec<&str> = rest.split('/').collect();
    if raw_segments.iter().any(|s| s.is_empty()) {
        return Err(LinkError::BadSyntax);
    }
    let mut raw_params = Vec::new();
    if let Some(q) = query_raw {
        for piece in q.split('&') {
            let (k, v) = piece.split_once('=').ok_or(LinkError::BadSyntax)?;
            raw_params.push((k, v));
        }
    }
    let mut decoded = raw_segments.iter().map(|s| decode_part(s)).collect::<R<Vec<_>>>()?;
    let head = decoded.remove(0);
    let mut params = Vec::with_capacity(raw_params.len());
    for (k, v) in raw_params {
        if !is_param_key(k) {
            return Err(if is_param_key(&k.to_lowercase()) { LinkError::NotCanonical } else { LinkError::BadSyntax });
        }
        if v.is_empty() {
            return Err(LinkError::EmptyParam);
        }
        params.push((k.to_string(), decode_part(v)?));
    }
    let anchor = anchor_raw.map(decode_part).transpose()?;
    Ok(Shape { head, segments: decoded, params, anchor })
}

/// An alias rewritten to the form it stands for (R7).
fn unalias(shape: Shape, reg: &Registry) -> Shape {
    let parts: Vec<&str> = std::iter::once(shape.head.as_str()).chain(shape.segments.iter().map(String::as_str)).collect();
    for alias in reg.aliases {
        if alias.from.len() != parts.len() || alias.from[0] != parts[0] {
            continue;
        }
        let mut bound: BTreeMap<&str, &str> = BTreeMap::new();
        let fits = alias.from.iter().zip(&parts).all(|(f, p)| {
            if f.starts_with('{') && f.ends_with('}') {
                bound.insert(*f, *p);
                true
            } else {
                f == p
            }
        });
        if !fits {
            continue;
        }
        let to: Vec<String> = alias.to.iter().map(|t| bound.get(*t).copied().unwrap_or(*t).to_string()).collect();
        return Shape { head: to[0].clone(), segments: to[1..].to_vec(), params: shape.params, anchor: shape.anchor };
    }
    shape
}

fn find_view(head: &str, reg: &'static Registry) -> R<(&'static View, Option<String>)> {
    if let Some(v) = reg.views.iter().find(|v| !v.prefix && v.id == head) {
        return Ok((v, None));
    }
    for v in reg.views.iter().filter(|v| v.prefix) {
        if let Some(pkg) = head.strip_prefix(v.id).and_then(|r| r.strip_prefix('.')) {
            if is_package(pkg) {
                return Ok((v, Some(pkg.to_string())));
            }
        }
    }
    let lower = head.to_lowercase();
    if head != lower && reg.views.iter().any(|v| v.id == lower) {
        return Err(LinkError::NotCanonical);
    }
    Err(LinkError::UnknownView)
}

/// The place `segments` names, with its vars' values.
fn match_place(view: &'static View, segments: &[String]) -> R<(&'static Place, BTreeMap<String, Value>)> {
    let lit_fits = |p: &Place, exact: bool| {
        p.segments.len() == segments.len()
            && p.segments.iter().zip(segments).all(|(s, given)| match s {
                Segment::Lit(l) => *l == given.as_str() || (!exact && *l == given.to_lowercase()),
                Segment::Var(_) => true,
            })
    };
    if !view.places.iter().any(|p| lit_fits(p, false)) {
        return Err(LinkError::UnknownPlace);
    }
    let exact: Vec<&'static Place> = view.places.iter().filter(|p| lit_fits(p, true)).collect();
    if exact.is_empty() {
        return Err(LinkError::NotCanonical);
    }
    let mut first = None;
    for place in exact {
        let mut values = BTreeMap::new();
        let mut failed = None;
        for (s, given) in place.segments.iter().zip(segments) {
            if let Segment::Var(v) = s {
                let spec = Typed { kind: v.kind, values: v.values, min: v.min, max: v.max };
                match typed_value(&spec, given, LinkError::UnknownPlace) {
                    Ok(val) => {
                        values.insert(v.name.to_string(), val);
                    }
                    Err(e) => {
                        failed = Some(e);
                        break;
                    }
                }
            }
        }
        match failed {
            None => return Ok((place, values)),
            Some(e) => {
                first.get_or_insert(e);
            }
        }
    }
    Err(first.unwrap_or(LinkError::UnknownPlace))
}

fn read_params(
    place: Option<&'static Place>,
    values: &BTreeMap<String, Value>,
    given: &[(String, String)],
) -> R<BTreeMap<String, Value>> {
    let declared = place.map(|p| p.params).unwrap_or(&[]);
    let mut out = BTreeMap::new();
    let mut last: Option<usize> = None;
    for (key, text) in given {
        if out.contains_key(key) {
            return Err(LinkError::DuplicateParam);
        }
        let index = declared.iter().position(|p| p.name == key.as_str()).ok_or(LinkError::UnknownParam)?;
        let spec = &declared[index];
        if let Some((var, allowed)) = spec.only_with {
            let holds = values.get(var).map(Value::to_string);
            if !holds.is_some_and(|h| allowed.contains(&h.as_str())) {
                return Err(LinkError::UnknownParam);
            }
        }
        if last.is_some_and(|l| index < l) {
            return Err(LinkError::NotCanonical);
        }
        last = Some(index);
        let typed = Typed { kind: spec.kind, values: spec.values, min: spec.min, max: spec.max };
        out.insert(key.clone(), typed_value(&typed, text, LinkError::BadValue)?);
    }
    Ok(out)
}

/// The one written form of a destination (R7). `values` holds `package` for a widget.
pub fn format_link(
    view: &View,
    place: Option<&Place>,
    values: &BTreeMap<String, Value>,
    params: &BTreeMap<String, Value>,
    anchor: Option<&str>,
) -> String {
    let head = if view.prefix {
        format!("{}.{}", view.id, values.get("package").map(Value::to_string).unwrap_or_default())
    } else {
        view.id.to_string()
    };
    let mut parts = vec![head];
    if let Some(p) = place {
        for s in p.segments {
            parts.push(match s {
                Segment::Lit(l) => l.to_string(),
                Segment::Var(v) => values.get(v.name).map(Value::to_string).unwrap_or_default(),
            });
        }
    }
    let mut link = format!("{SCHEME}{}", parts.iter().map(|p| encode_part(p)).collect::<Vec<_>>().join("/"));
    let written: Vec<String> = place
        .map(|p| p.params)
        .unwrap_or(&[])
        .iter()
        .filter_map(|p| params.get(p.name).map(|v| format!("{}={}", p.name, encode_part(&v.to_string()))))
        .collect();
    if !written.is_empty() {
        link.push('?');
        link.push_str(&written.join("&"));
    }
    if let Some(a) = anchor {
        link.push('#');
        link.push_str(&encode_part(a));
    }
    link
}

/// Reads `link` against `reg`: the destination it names, or why it is refused (R8).
pub fn parse_link_in(link: &str, reg: &'static Registry) -> R<ParsedLink> {
    let shape = unalias(shape_of(link, reg)?, reg);
    let (view, pkg) = find_view(&shape.head, reg)?;
    if view.build > reg.build {
        return Err(LinkError::NotYet);
    }
    let (place, mut values) = if shape.segments.is_empty() {
        if view.requires_place {
            return Err(LinkError::MissingPlace);
        }
        (None, BTreeMap::new())
    } else {
        let (p, v) = match_place(view, &shape.segments)?;
        (Some(p), v)
    };
    if let Some(pkg) = pkg {
        values.insert("package".to_string(), Value::Text(pkg));
    }
    let params = read_params(place, &values, &shape.params)?;
    if let Some(a) = &shape.anchor {
        if place.and_then(|p| p.anchor).is_none() {
            return Err(LinkError::UnknownAnchor);
        }
        typed_value(&Typed { kind: Kind::Id, values: &[], min: None, max: None }, a, LinkError::BadValue)?;
    }
    let canonical = format_link(view, place, &values, &params, shape.anchor.as_deref());
    Ok(ParsedLink { view, place, values, params, anchor: shape.anchor, canonical })
}

/// Reads `link` against the registry.
pub fn parse_link(link: &str) -> R<ParsedLink> {
    parse_link_in(link, &super::registry::REGISTRY)
}
