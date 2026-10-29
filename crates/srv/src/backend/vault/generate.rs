// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Strong values the Vault makes itself, so no person types them and no agent
//! sees them (docs/specs/SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §4.1).
//!
//! Characters are drawn from the OS generator (`ring`'s `SystemRandom`) with
//! rejection sampling, so every allowed character is equally likely. The rules
//! take the `passwordrules` format sites declare on their fields
//! (`minlength: 12; maxlength: 32; required: lower; required: digit; allowed: [-_]`).

use ring::rand::{SecureRandom, SystemRandom};
use zeroize::Zeroizing;

const LOWER: &str = "abcdefghijkmnopqrstuvwxyz";
const UPPER: &str = "ABCDEFGHJKLMNPQRSTUVWXYZ";
const DIGIT: &str = "23456789";
/// The symbols most sites accept.
const SPECIAL: &str = "-_.!@#%^*";
/// Look-alikes left out of every class, so a person can read a value if they ever must.
const LOOK_ALIKES: &str = "0O1lI";

pub const DEFAULT_LENGTH: usize = 20;
pub const MIN_LENGTH: usize = 8;
pub const MAX_LENGTH: usize = 128;
/// Below this a character value is refused: the rules leave too few choices.
pub const MIN_BITS: f64 = 80.0;
pub const DEFAULT_WORDS: usize = 6;
const MAX_WORDS: usize = 12;
/// Tries at drawing a value that meets every `required` class and
/// `max-consecutive`; with any rules that leave 80 bits, a few are enough.
const TRIES: usize = 200;

/// How a value is built.
#[derive(Debug, Clone, PartialEq)]
pub enum Style {
    /// Characters from the classes the rules allow.
    Chars,
    /// Words from the Vault's word list, joined by `-`: for a value a person
    /// must type on a TV or a phone.
    Words(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rules {
    pub style: Style,
    pub length: usize,
    /// Each set must give the value at least one character.
    pub required: Vec<String>,
    /// Every character the value may hold (the required sets included).
    pub allowed: String,
    pub max_consecutive: Option<usize>,
}

impl Default for Rules {
    fn default() -> Self {
        let required = vec![LOWER.to_string(), UPPER.to_string(), DIGIT.to_string(), SPECIAL.to_string()];
        Rules { style: Style::Chars, length: DEFAULT_LENGTH, allowed: required.concat(), required, max_consecutive: None }
    }
}

/// What a caller asks for: a site's `passwordrules`, a length, or words.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct RulesRequest {
    #[serde(default)]
    pub passwordrules: Option<String>,
    #[serde(default)]
    pub length: Option<usize>,
    #[serde(default)]
    pub words: Option<usize>,
    /// A field's `maxlength`, which caps the length whatever else says.
    #[serde(default)]
    pub maxlength: Option<usize>,
}

fn class(name: &str) -> Option<&'static str> {
    match name {
        "lower" => Some(LOWER),
        "upper" => Some(UPPER),
        "digit" => Some(DIGIT),
        "special" => Some(SPECIAL),
        _ => None,
    }
}

/// The characters of a `[...]` set, minus look-alikes, or an error naming it.
fn custom_set(set: &str) -> Result<String, String> {
    let inner = set.strip_prefix('[').and_then(|s| s.strip_suffix(']')).ok_or_else(|| format!("{set:?} isn't a class or a [set]"))?;
    let mut out = String::new();
    for c in inner.chars() {
        if c.is_ascii_graphic() && !LOOK_ALIKES.contains(c) && !out.contains(c) {
            out.push(c);
        }
    }
    if out.is_empty() {
        return Err(format!("the set {set:?} holds no usable character"));
    }
    Ok(out)
}

/// A value list in `passwordrules` (`lower, upper, [-_]`) as character sets.
fn sets(value: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for item in value.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        match class(item) {
            Some(c) => out.push(c.to_string()),
            None if item == "ascii-printable" || item == "unicode" => out.push([LOWER, UPPER, DIGIT, SPECIAL].concat()),
            None => out.push(custom_set(item)?),
        }
    }
    Ok(out)
}

fn merge(into: &mut String, set: &str) {
    for c in set.chars() {
        if !into.contains(c) {
            into.push(c);
        }
    }
}

/// Builds the rules from what was asked. A site's `passwordrules` replaces the
/// default classes; `length` picks a length within its bounds.
pub fn rules(req: &RulesRequest) -> Result<Rules, String> {
    if let Some(n) = req.words {
        if !(DEFAULT_WORDS..=MAX_WORDS).contains(&n) {
            return Err(format!("words is {DEFAULT_WORDS} to {MAX_WORDS}"));
        }
        return Ok(Rules { style: Style::Words(n), length: 0, required: Vec::new(), allowed: String::new(), max_consecutive: None });
    }
    let mut r = Rules::default();
    let (mut min, mut max) = (MIN_LENGTH, MAX_LENGTH);
    if let Some(text) = req.passwordrules.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        if text.len() > 1000 {
            return Err("passwordrules is longer than 1000 characters".into());
        }
        let (mut required, mut allowed) = (Vec::new(), String::new());
        for rule in text.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            let (name, value) = rule.split_once(':').ok_or_else(|| format!("{rule:?} isn't name: value"))?;
            let (name, value) = (name.trim().to_ascii_lowercase(), value.trim());
            let number = || value.parse::<usize>().map_err(|_| format!("{name} needs a number, not {value:?}"));
            match name.as_str() {
                "minlength" => min = min.max(number()?),
                "maxlength" => max = max.min(number()?),
                "max-consecutive" => r.max_consecutive = Some(number()?.max(1)),
                "required" => {
                    // `required: lower, upper` means one character from either.
                    let union = sets(value)?.concat();
                    merge(&mut allowed, &union);
                    required.push(union);
                }
                "allowed" => {
                    for s in sets(value)? {
                        merge(&mut allowed, &s);
                    }
                }
                // Unknown rules are ignored, as browsers do.
                _ => {}
            }
        }
        if !allowed.is_empty() {
            r.allowed = allowed;
            r.required = required;
        }
    }
    if let Some(m) = req.maxlength.filter(|m| *m > 0) {
        max = max.min(m);
    }
    if min > max {
        return Err(format!("the rules ask for at least {min} and at most {max} characters"));
    }
    r.length = req.length.unwrap_or(DEFAULT_LENGTH).clamp(min, max);
    if r.required.len() > r.length {
        return Err(format!("{} required classes don't fit in {} characters", r.required.len(), r.length));
    }
    let bits = bits(&r);
    if bits < MIN_BITS {
        return Err(format!(
            "these rules leave {bits:.0} bits ({} characters from {} choices); at least {MIN_BITS:.0} are needed: allow a longer value or more characters",
            r.length,
            r.allowed.chars().count()
        ));
    }
    Ok(r)
}

/// How many bits of choice a value under these rules carries.
pub fn bits(r: &Rules) -> f64 {
    match r.style {
        Style::Words(n) => n as f64 * (super::wordlist::WORDS.len() as f64).log2(),
        Style::Chars => r.length as f64 * (r.allowed.chars().count().max(1) as f64).log2(),
    }
}

/// A uniform index below `n` (at most 256), by rejection sampling.
fn index(rng: &SystemRandom, n: usize) -> Result<usize, String> {
    debug_assert!(n > 0 && n <= 256);
    let limit = 256 - (256 % n);
    loop {
        let mut b = [0u8; 1];
        rng.fill(&mut b).map_err(|_| "the system random generator failed".to_string())?;
        if (b[0] as usize) < limit {
            return Ok(b[0] as usize % n);
        }
    }
}

fn meets(value: &[char], r: &Rules) -> bool {
    if !r.required.iter().all(|set| value.iter().any(|c| set.contains(*c))) {
        return false;
    }
    if let Some(max) = r.max_consecutive {
        let mut run = 1;
        for w in value.windows(2) {
            run = if w[0] == w[1] { run + 1 } else { 1 };
            if run > max {
                return false;
            }
        }
    }
    true
}

/// A new value under the rules. It lives only in the returned buffer.
pub fn generate(r: &Rules) -> Result<Zeroizing<String>, String> {
    let rng = SystemRandom::new();
    if let Style::Words(n) = r.style {
        let words = super::wordlist::WORDS;
        let mut out = Zeroizing::new(String::new());
        for i in 0..n {
            if i > 0 {
                out.push('-');
            }
            out.push_str(words[index(&rng, words.len())?]);
        }
        return Ok(out);
    }
    let alphabet: Vec<char> = r.allowed.chars().collect();
    if alphabet.is_empty() || alphabet.len() > 256 {
        return Err("the rules allow no characters".into());
    }
    for _ in 0..TRIES {
        let mut chars = Zeroizing::new(Vec::with_capacity(r.length));
        for _ in 0..r.length {
            chars.push(alphabet[index(&rng, alphabet.len())?]);
        }
        if meets(&chars, r) {
            return Ok(Zeroizing::new(chars.iter().collect()));
        }
    }
    Err("couldn't make a value that meets every rule: they may contradict each other".into())
}

/// What the rules were, in a line an agent or the pane can show.
pub fn describe(r: &Rules) -> String {
    match r.style {
        Style::Words(n) => format!("{n} words, {:.0} bits", bits(r)),
        Style::Chars => format!("{} characters from {} choices, {:.0} bits", r.length, r.allowed.chars().count(), bits(r)),
    }
}

#[cfg(test)]
#[path = "generate_tests.rs"]
mod tests;
