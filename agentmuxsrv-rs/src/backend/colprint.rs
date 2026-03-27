// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Column printing utility for terminal output.
//! Port of Go's `pkg/util/colprint/colprint.go`.
//!
//! Prints values in fixed-width columns to a writer. Samples the first N
//! values to determine column width, then prints all values using that width.
//! Handles values that span multiple columns when they are too wide.

#![allow(dead_code)]

use std::io::{self, Write};

/// Print an iterator of values in columns.
///
/// - `values`: iterator yielding items
/// - `num_cols`: number of columns per row
/// - `sample_size`: how many items to read initially to determine column width
/// - `format_fn`: converts an item to a display string
/// - `writer`: output destination
pub fn print_columns<T, I, F>(
    values: I,
    num_cols: usize,
    sample_size: usize,
    format_fn: F,
    writer: &mut dyn Write,
) -> io::Result<()>
where
    I: IntoIterator<Item = T>,
    F: Fn(&T) -> io::Result<String>,
{
    let mut iter = values.into_iter();
    let mut max_len = 0;
    let mut samples = Vec::new();

    // Sample phase: determine max width
    for item in iter.by_ref() {
        let s = format_fn(&item)?;
        if s.len() > max_len {
            max_len = s.len();
        }
        samples.push(item);
        if samples.len() >= sample_size {
            break;
        }
    }

    let col_width = (max_len + 2).max(1);
    let mut col = 0;

    // Print sampled items
    for item in &samples {
        let s = format_fn(item)?;
        print_col_helper(&s, col_width, &mut col, num_cols, writer)?;
    }

    // Print remaining items
    for item in iter {
        let s = format_fn(&item)?;
        print_col_helper(&s, col_width, &mut col, num_cols, writer)?;
    }

    if col > 0 {
        writeln!(writer)?;
    }

    Ok(())
}

/// Print an iterator of values that produce multiple strings per item.
///
/// Each item may produce multiple display strings, all printed in columns.
pub fn print_columns_array<T, I, F>(
    values: I,
    num_cols: usize,
    sample_size: usize,
    format_fn: F,
    writer: &mut dyn Write,
) -> io::Result<()>
where
    I: IntoIterator<Item = T>,
    F: Fn(&T) -> io::Result<Vec<String>>,
{
    let mut iter = values.into_iter();
    let mut max_len = 0;
    let mut samples = Vec::new();

    // Sample phase
    for item in iter.by_ref() {
        let strings = format_fn(&item)?;
        for s in &strings {
            if s.len() > max_len {
                max_len = s.len();
            }
        }
        samples.push(item);
        if samples.len() >= sample_size {
            break;
        }
    }

    let col_width = (max_len + 2).max(1);
    let mut col = 0;

    // Print sampled items
    for item in &samples {
        let strings = format_fn(item)?;
        for s in &strings {
            print_col_helper(s, col_width, &mut col, num_cols, writer)?;
        }
    }

    // Print remaining items
    for item in iter {
        let strings = format_fn(&item)?;
        for s in &strings {
            print_col_helper(s, col_width, &mut col, num_cols, writer)?;
        }
    }

    if col > 0 {
        writeln!(writer)?;
    }

    Ok(())
}

/// Helper to print a single value in a column, handling multi-column spans.
fn print_col_helper(
    s: &str,
    col_width: usize,
    col: &mut usize,
    num_cols: usize,
    writer: &mut dyn Write,
) -> io::Result<()> {
    let mut name_col_span = (s.len() + 1) / col_width;
    if (s.len() + 1) % col_width != 0 {
        name_col_span += 1;
    }

    if *col + name_col_span > num_cols {
        writeln!(writer)?;
        *col = 0;
    }

    write!(writer, "{:<width$}", s, width = name_col_span * col_width)?;
    *col += name_col_span;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_column() {
        let items = vec!["alpha", "beta", "gamma"];
        let mut buf = Vec::new();
        print_columns(
            items.iter(),
            1,
            10,
            |item| Ok(item.to_string()),
            &mut buf,
        )
        .unwrap();
        let output = String::from_utf8(buf).unwrap();
        // Each item on its own line
        assert!(output.contains("alpha"));
        assert!(output.contains("beta"));
        assert!(output.contains("gamma"));
    }

    #[test]
    fn test_multiple_columns() {
        let items = vec!["a", "b", "c", "d"];
        let mut buf = Vec::new();
        print_columns(
            items.iter(),
            4,
            10,
            |item| Ok(item.to_string()),
            &mut buf,
        )
        .unwrap();
        let output = String::from_utf8(buf).unwrap();
        // All items should be on one line (4 items, 4 cols)
        let lines: Vec<&str> = output.trim().split('\n').collect();
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn test_wrapping() {
        let items = vec!["aa", "bb", "cc", "dd", "ee"];
        let mut buf = Vec::new();
        print_columns(
            items.iter(),
            2,
            10,
            |item| Ok(item.to_string()),
            &mut buf,
        )
        .unwrap();
        let output = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = output.trim().split('\n').collect();
        // 5 items, 2 cols = 3 lines (2+2+1)
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn test_wide_value_spans_columns() {
        let items = vec!["short", "a-very-long-value-that-spans"];
        let mut buf = Vec::new();
        print_columns(
            items.iter(),
            3,
            10,
            |item| Ok(item.to_string()),
            &mut buf,
        )
        .unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("short"));
        assert!(output.contains("a-very-long-value-that-spans"));
    }

    #[test]
    fn test_empty_input() {
        let items: Vec<&str> = vec![];
        let mut buf = Vec::new();
        print_columns(
            items.iter(),
            3,
            10,
            |item| Ok(item.to_string()),
            &mut buf,
        )
        .unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(output.is_empty());
    }

    #[test]
    fn test_print_columns_array() {
        let items = vec![vec!["a1", "a2"], vec!["b1", "b2"]];
        let mut buf = Vec::new();
        print_columns_array(
            items.iter(),
            4,
            10,
            |item| Ok(item.iter().map(|s| s.to_string()).collect()),
            &mut buf,
        )
        .unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("a1"));
        assert!(output.contains("a2"));
        assert!(output.contains("b1"));
        assert!(output.contains("b2"));
    }

    #[test]
    fn test_sample_size_smaller_than_input() {
        let items: Vec<String> = (0..20).map(|i| format!("item{}", i)).collect();
        let mut buf = Vec::new();
        print_columns(
            items.iter(),
            3,
            5, // Only sample first 5
            |item| Ok(item.to_string()),
            &mut buf,
        )
        .unwrap();
        let output = String::from_utf8(buf).unwrap();
        // All 20 items should appear
        assert!(output.contains("item0"));
        assert!(output.contains("item19"));
    }
}
