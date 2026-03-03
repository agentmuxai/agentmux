// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

#![allow(dead_code)]

use std::io;
use std::path::Path;

// ---- Binary Detection ----

/// Check if data contains binary (non-text) bytes.
/// Returns true if any byte < 32 other than `\n`, `\r`, `\t`, `\f`, `\b`.
pub fn has_binary_data(data: &[u8]) -> bool {
    data.iter().any(|&b| b < 32 && b != b'\n' && b != b'\r' && b != b'\t' && b != 0x0C && b != 0x08)
}

/// Check if content is binary by examining up to 8192 bytes.
/// Checks for null byte ratio > 1% and UTF-8 validity.
pub fn is_binary_content(data: &[u8]) -> bool {
    if data.is_empty() {
        return false;
    }
    let sample_size = std::cmp::min(8192, data.len());
    let sample = &data[..sample_size];

    let null_count = sample.iter().filter(|&&b| b == 0).count();
    if null_count as f64 / sample.len() as f64 > 0.01 {
        return true;
    }

    if std::str::from_utf8(sample).is_err() {
        return true;
    }

    false
}

// ---- Star Matching ----

/// Match a delimited string with a pattern string.
/// `*` matches a single part, `**` matches the rest of the string (only valid at end).
pub fn star_match_string(pattern: &str, s: &str, delimiter: &str) -> bool {
    let pattern_parts: Vec<&str> = pattern.split(delimiter).collect();
    let string_parts: Vec<&str> = s.split(delimiter).collect();
    let p_len = pattern_parts.len();
    let s_len = string_parts.len();

    for i in 0..p_len {
        if pattern_parts[i] == "**" {
            return i == p_len - 1;
        }
        if i >= s_len {
            return false;
        }
        if pattern_parts[i] != "*" && pattern_parts[i] != string_parts[i] {
            return false;
        }
    }
    p_len == s_len
}

// ---- Slice Operations ----

/// Find the index of an element in a slice. Returns -1 if not found.
pub fn slice_idx<T: PartialEq>(arr: &[T], elem: &T) -> i32 {
    for (idx, e) in arr.iter().enumerate() {
        if e == elem {
            return idx as i32;
        }
    }
    -1
}

/// Remove an element from a vec. Returns a new vec without the element.
pub fn remove_elem<T: PartialEq + Clone>(arr: &[T], elem: &T) -> Vec<T> {
    let idx = slice_idx(arr, elem);
    if idx == -1 {
        return arr.to_vec();
    }
    let idx = idx as usize;
    let mut result = Vec::with_capacity(arr.len() - 1);
    result.extend_from_slice(&arr[..idx]);
    result.extend_from_slice(&arr[idx + 1..]);
    result
}

/// Add an element to a vec if it's not already present.
pub fn add_elem_uniq<T: PartialEq + Clone>(arr: &[T], elem: T) -> Vec<T> {
    if slice_idx(arr, &elem) != -1 {
        return arr.to_vec();
    }
    let mut result = arr.to_vec();
    result.push(elem);
    result
}

/// Move element at `idx` to the front of the slice. Returns a new vec.
pub fn move_to_front<T: Clone>(arr: &[T], idx: usize) -> Vec<T> {
    if idx == 0 || idx >= arr.len() {
        return arr.to_vec();
    }
    let mut rtn = Vec::with_capacity(arr.len());
    rtn.push(arr[idx].clone());
    rtn.extend_from_slice(&arr[..idx]);
    rtn.extend_from_slice(&arr[idx + 1..]);
    rtn
}

// ---- String Helpers ----

/// Truncate a string with "..." if it exceeds maxLen (char-safe).
pub fn ellipsis_str(s: &str, max_len: usize) -> String {
    let max_len = if max_len < 4 { 4 } else { max_len };
    let char_count = s.chars().count();
    if char_count > max_len {
        let truncated: String = s.chars().take(max_len - 3).collect();
        format!("{}...", truncated)
    } else {
        s.to_string()
    }
}

/// Truncate a string with "..." if it exceeds maxLen (char-safe).
pub fn truncate_string(s: &str, max_len: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_len {
        return s.to_string();
    }
    let max_len = if max_len < 4 { 4 } else { max_len };
    let truncated: String = s.chars().take(max_len - 3).collect();
    format!("{}...", truncated)
}

/// Get the first line of a string.
pub fn get_first_line(s: &str) -> &str {
    match s.find('\n') {
        Some(idx) => &s[..idx],
        None => s,
    }
}

/// Parse an integer from a string, returning 0 on error.
pub fn atoi_no_err(s: &str) -> i32 {
    s.parse::<i32>().unwrap_or(0)
}

/// Generate a random hex string of the given number of hex digits.
pub fn random_hex_string(num_hex_digits: usize) -> Result<String, io::Error> {
    use std::fmt::Write;
    let num_bytes = num_hex_digits.div_ceil(2);
    let mut bytes = vec![0u8; num_bytes];
    getrandom(&mut bytes)?;
    let mut hex = String::with_capacity(num_hex_digits);
    for b in &bytes {
        write!(hex, "{:02x}", b).unwrap();
    }
    hex.truncate(num_hex_digits);
    Ok(hex)
}

/// Platform-independent random bytes.
/// Uses /dev/urandom on Unix, hash-based randomness on Windows.
fn getrandom(buf: &mut [u8]) -> Result<(), io::Error> {
    #[cfg(unix)]
    {
        use std::fs::File;
        use std::io::Read;
        let mut f = File::open("/dev/urandom")?;
        f.read_exact(buf)?;
    }
    #[cfg(windows)]
    {
        use std::collections::hash_map::RandomState;
        use std::hash::{BuildHasher, Hasher};
        let mut offset = 0;
        while offset < buf.len() {
            let state = RandomState::new();
            let hash = state.build_hasher().finish();
            let bytes = hash.to_ne_bytes();
            let copy_len = std::cmp::min(bytes.len(), buf.len() - offset);
            buf[offset..offset + copy_len].copy_from_slice(&bytes[..copy_len]);
            offset += copy_len;
        }
    }
    Ok(())
}

/// Convert known architecture names to standard patterns.
pub fn filter_valid_arch(arch: &str) -> Result<&'static str, String> {
    match arch.trim().to_lowercase().as_str() {
        "amd64" | "x86_64" | "x64" => Ok("x64"),
        "arm64" | "aarch64" => Ok("arm64"),
        other => Err(format!("unknown architecture: {}", other)),
    }
}

// ---- Atomic File Operations ----

/// Atomically copy a file by writing to a temp file then renaming.
/// On Unix, sets file permissions to `perms`. On Windows, permissions are ignored.
pub fn atomic_rename_copy(dst_path: &Path, src_path: &Path, _perms: u32) -> io::Result<()> {
    use std::fs;

    let temp_name = format!("{}.new", dst_path.display());
    let temp_path = Path::new(&temp_name);
    fs::copy(src_path, temp_path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(temp_path, fs::Permissions::from_mode(_perms))?;
    }
    fs::rename(temp_path, dst_path)?;
    Ok(())
}

/// Write file only if contents differ. Returns true if file was written.
pub fn write_file_if_different(file_name: &Path, contents: &[u8]) -> io::Result<bool> {
    use std::fs;
    if let Ok(old_contents) = fs::read(file_name) {
        if old_contents == contents {
            return Ok(false);
        }
    }
    fs::write(file_name, contents)?;
    Ok(true)
}

// ---- Misc ----

/// Get line and column from a byte offset in content.
pub fn get_line_col_from_offset(data: &[u8], offset: usize) -> (usize, usize) {
    let mut line = 1;
    let mut col = 1;
    for &byte in data.iter().take(std::cmp::min(offset, data.len())) {
        if byte == b'\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// Find the longest common prefix of a set of strings, bounded by a root.
pub fn longest_prefix(root: &str, strs: &[&str]) -> String {
    if strs.is_empty() {
        return root.to_string();
    }
    if strs.len() == 1 {
        let comp = strs[0];
        if comp.len() >= root.len() && comp.starts_with(root) {
            return comp.to_string();
        }
    }
    let mut lcp = strs[0].to_string();
    for s in &strs[1..] {
        let mut end = 0;
        for (j, (a, b)) in lcp.chars().zip(s.chars()).enumerate() {
            if a != b {
                break;
            }
            end = j + a.len_utf8();
        }
        lcp.truncate(end);
    }
    if lcp.len() < root.len() || !lcp.starts_with(root) {
        return root.to_string();
    }
    lcp
}

/// Indent each non-empty line of a string.
pub fn indent_string(indent: &str, s: &str) -> String {
    let mut result = String::new();
    for line in s.split('\n') {
        if line.is_empty() {
            result.push('\n');
        } else {
            result.push_str(indent);
            result.push_str(line);
            result.push('\n');
        }
    }
    result
}

/// Shell hex escape a string (each byte as `\xNN`).
pub fn shell_hex_escape(s: &str) -> String {
    let mut result = String::new();
    for b in s.bytes() {
        result.push_str(&format!("\\x{:02x}", b));
    }
    result
}

/// Combine two string arrays, removing duplicates while preserving order.
pub fn combine_str_arrays(arr1: &[String], arr2: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for s in arr1.iter().chain(arr2.iter()) {
        if seen.insert(s.clone()) {
            result.push(s.clone());
        }
    }
    result
}

// ---- Sentinel / Helper Types ----

/// Sentinel value for StrWithPos.pos to indicate no position.
pub const NO_STR_POS: i32 = -1;

/// A string with a cursor position (rune-based, not byte-based).
#[derive(Debug, Clone, PartialEq)]
pub struct StrWithPos {
    pub str_val: String,
    pub pos: i32,
}

impl StrWithPos {
    pub fn new(s: String, pos: i32) -> Self {
        Self { str_val: s, pos }
    }

    /// Parse a string with `[*]` cursor marker.
    pub fn parse(s: &str) -> Self {
        match s.find("[*]") {
            None => Self { str_val: s.to_string(), pos: NO_STR_POS },
            Some(idx) => {
                let before = &s[..idx];
                let after = &s[idx + 3..];
                let pos = before.chars().count() as i32;
                Self {
                    str_val: format!("{}{}", before, after),
                    pos,
                }
            }
        }
    }

    pub fn prepend(&self, prefix: &str) -> Self {
        Self {
            str_val: format!("{}{}", prefix, self.str_val),
            pos: prefix.chars().count() as i32 + self.pos,
        }
    }

    pub fn append(&self, suffix: &str) -> Self {
        Self {
            str_val: format!("{}{}", self.str_val, suffix),
            pos: self.pos,
        }
    }
}

impl std::fmt::Display for StrWithPos {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.pos == NO_STR_POS {
            write!(f, "{}", self.str_val)
        } else if self.pos < 0 {
            write!(f, "[*]_{}", self.str_val)
        } else {
            let pos = self.pos as usize;
            let mut chars: Vec<char> = Vec::new();
            for (i, ch) in self.str_val.chars().enumerate() {
                if i == pos {
                    chars.extend(['[', '*', ']']);
                }
                chars.push(ch);
            }
            if pos >= self.str_val.chars().count() {
                chars.extend(['[', '*', ']']);
            }
            write!(f, "{}", chars.iter().collect::<String>())
        }
    }
}
