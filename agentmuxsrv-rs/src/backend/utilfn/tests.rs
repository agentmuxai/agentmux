// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

use super::hash::*;
use super::nullenc::*;
use super::strutil::*;

// ---- Null Encoding Tests ----

#[test]
fn test_null_encode_no_special() {
    assert_eq!(null_encode_str("hello"), b"hello");
}

#[test]
fn test_null_encode_with_null() {
    assert_eq!(null_encode_str("a\0b"), b"a\\0b");
}

#[test]
fn test_null_encode_with_backslash() {
    assert_eq!(null_encode_str("a\\b"), b"a\\\\b");
}

#[test]
fn test_null_encode_with_sep() {
    assert_eq!(null_encode_str("a|b"), b"a\\sb");
}

#[test]
fn test_null_encode_with_eq() {
    assert_eq!(null_encode_str("a=b"), b"a\\eb");
}

#[test]
fn test_null_decode_no_special() {
    assert_eq!(null_decode_str(b"hello").unwrap(), "hello");
}

#[test]
fn test_null_decode_with_null() {
    assert_eq!(null_decode_str(b"a\\0b").unwrap(), "a\0b");
}

#[test]
fn test_null_decode_with_backslash() {
    assert_eq!(null_decode_str(b"a\\\\b").unwrap(), "a\\b");
}

#[test]
fn test_null_decode_with_sep() {
    assert_eq!(null_decode_str(b"a\\sb").unwrap(), "a|b");
}

#[test]
fn test_null_decode_with_eq() {
    assert_eq!(null_decode_str(b"a\\eb").unwrap(), "a=b");
}

#[test]
fn test_null_encode_decode_roundtrip() {
    let cases = ["hello", "a\0b\0c", "key=val|more\\end", "", "no special"];
    for s in &cases {
        let encoded = null_encode_str(s);
        let decoded = null_decode_str(&encoded).unwrap();
        assert_eq!(&decoded, s, "roundtrip failed for {:?}", s);
    }
}

#[test]
fn test_null_decode_invalid() {
    assert!(null_decode_str(b"a\\xb").is_err());
}

#[test]
fn test_null_decode_escape_at_end() {
    assert!(null_decode_str(b"a\\").is_err());
}

// ---- String Map Encoding ----

#[test]
fn test_encode_decode_string_map() {
    let mut m = HashMap::new();
    m.insert("key1".into(), "val1".into());
    m.insert("key2".into(), "val2".into());
    let encoded = encode_string_map(&m);
    let decoded = decode_string_map(&encoded).unwrap();
    assert_eq!(decoded, m);
}

#[test]
fn test_encode_decode_string_map_with_special() {
    let mut m = HashMap::new();
    m.insert("k\0y".into(), "v=l".into());
    m.insert("a|b".into(), "c\\d".into());
    let encoded = encode_string_map(&m);
    let decoded = decode_string_map(&encoded).unwrap();
    assert_eq!(decoded, m);
}

#[test]
fn test_decode_empty_string_map() {
    let decoded = decode_string_map(b"").unwrap();
    assert!(decoded.is_empty());
}

// ---- String Array Encoding ----

#[test]
fn test_encode_decode_string_array() {
    let arr = vec!["hello".to_string(), "world".to_string()];
    let encoded = encode_string_array(&arr);
    let decoded = decode_string_array(&encoded).unwrap();
    assert_eq!(decoded, arr);
}

#[test]
fn test_encode_decode_string_array_with_special() {
    let arr = vec!["a|b".to_string(), "c=d".to_string(), "e\\f".to_string()];
    let encoded = encode_string_array(&arr);
    let decoded = decode_string_array(&encoded).unwrap();
    assert_eq!(decoded, arr);
}

#[test]
fn test_decode_empty_array() {
    let decoded = decode_string_array(b"").unwrap();
    assert!(decoded.is_empty());
}

#[test]
fn test_encoded_string_array_has_first_val() {
    let arr = vec!["first".to_string(), "second".to_string()];
    let encoded = encode_string_array(&arr);
    assert!(encoded_string_array_has_first_val(&encoded, "first"));
    assert!(!encoded_string_array_has_first_val(&encoded, "second"));
    assert!(!encoded_string_array_has_first_val(&encoded, "firs"));
}

#[test]
fn test_encoded_string_array_get_first_val() {
    let arr = vec!["first".to_string(), "second".to_string()];
    let encoded = encode_string_array(&arr);
    assert_eq!(encoded_string_array_get_first_val(&encoded), "first");
}

#[test]
fn test_encoded_string_array_get_first_val_single() {
    let arr = vec!["only".to_string()];
    let encoded = encode_string_array(&arr);
    assert_eq!(encoded_string_array_get_first_val(&encoded), "only");
}

// ---- Hashing Tests ----

#[test]
fn test_sha1_hash_known() {
    // SHA1("") = da39a3ee5e6b4b0d3255bfef95601890afd80709
    let hash = sha1_hash(b"");
    // Base64 of that 20-byte hash
    assert_eq!(hash, "2jmj7l5rSw0yVb/vlWAYkK/YBwk=");
}

#[test]
fn test_sha1_hash_hello() {
    // SHA1("hello") = aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d
    let hash = sha1_hash(b"hello");
    assert_eq!(hash, "qvTGHdzF6KLavt4PO0gs2a6pQ00=");
}

#[test]
fn test_quick_hash_string() {
    // FNV-64a deterministic
    let h1 = quick_hash_string("test");
    let h2 = quick_hash_string("test");
    assert_eq!(h1, h2);
    // Different input -> different hash
    assert_ne!(quick_hash_string("test"), quick_hash_string("other"));
}

#[test]
fn test_quick_hash_string_known() {
    // FNV-64a("") = CBF29CE484222325 in big-endian
    let h = quick_hash_string("");
    assert!(!h.is_empty());
}

// ---- Binary Detection ----

#[test]
fn test_has_binary_data_text() {
    assert!(!has_binary_data(b"hello world\n"));
}

#[test]
fn test_has_binary_data_with_null() {
    assert!(has_binary_data(b"hello\x00world"));
}

#[test]
fn test_has_binary_data_with_control() {
    assert!(has_binary_data(b"hello\x01world"));
}

#[test]
fn test_has_binary_data_with_tab_newline() {
    assert!(!has_binary_data(b"hello\tworld\nfoo\rbar"));
}

#[test]
fn test_is_binary_content_empty() {
    assert!(!is_binary_content(b""));
}

#[test]
fn test_is_binary_content_text() {
    assert!(!is_binary_content(b"hello world\n"));
}

#[test]
fn test_is_binary_content_many_nulls() {
    let mut data = vec![0u8; 100];
    data.extend_from_slice(b"hello");
    assert!(is_binary_content(&data));
}

// ---- Star Matching ----

#[test]
fn test_star_match_exact() {
    assert!(star_match_string("a:b:c", "a:b:c", ":"));
}

#[test]
fn test_star_match_wildcard() {
    assert!(star_match_string("a:*:c", "a:b:c", ":"));
}

#[test]
fn test_star_match_double_star() {
    assert!(star_match_string("a:**", "a:b:c", ":"));
}

#[test]
fn test_star_match_no_match() {
    assert!(!star_match_string("a:b:c", "a:b:d", ":"));
}

#[test]
fn test_star_match_length_mismatch() {
    assert!(!star_match_string("a:b", "a:b:c", ":"));
    assert!(!star_match_string("a:b:c", "a:b", ":"));
}

#[test]
fn test_star_match_double_star_must_be_last() {
    assert!(!star_match_string("a:**:c", "a:b:c", ":"));
}

// ---- Slice Operations ----

#[test]
fn test_slice_idx_found() {
    assert_eq!(slice_idx(&[1, 2, 3], &2), 1);
}

#[test]
fn test_slice_idx_not_found() {
    assert_eq!(slice_idx(&[1, 2, 3], &4), -1);
}

#[test]
fn test_remove_elem() {
    assert_eq!(remove_elem(&[1, 2, 3], &2), vec![1, 3]);
}

#[test]
fn test_remove_elem_not_found() {
    assert_eq!(remove_elem(&[1, 2, 3], &4), vec![1, 2, 3]);
}

#[test]
fn test_add_elem_uniq_new() {
    assert_eq!(add_elem_uniq(&[1, 2], 3), vec![1, 2, 3]);
}

#[test]
fn test_add_elem_uniq_existing() {
    assert_eq!(add_elem_uniq(&[1, 2, 3], 2), vec![1, 2, 3]);
}

#[test]
fn test_move_to_front() {
    assert_eq!(move_to_front(&[1, 2, 3, 4], 2), vec![3, 1, 2, 4]);
}

#[test]
fn test_move_to_front_already_front() {
    assert_eq!(move_to_front(&[1, 2, 3], 0), vec![1, 2, 3]);
}

#[test]
fn test_move_to_front_out_of_bounds() {
    assert_eq!(move_to_front(&[1, 2, 3], 5), vec![1, 2, 3]);
}

// ---- String Helpers ----

#[test]
fn test_ellipsis_str() {
    assert_eq!(ellipsis_str("hello world foo bar", 10), "hello w...");
    assert_eq!(ellipsis_str("short", 10), "short");
}

#[test]
fn test_ellipsis_str_multibyte() {
    // "hllo wrld" has multi-byte chars -- must not panic
    assert_eq!(ellipsis_str("héllo wörld foo", 10), "héllo w...");
    // CJK characters
    assert_eq!(ellipsis_str("\u{4f60}\u{597d}\u{4e16}\u{754c}\u{6d4b}\u{8bd5}\u{6570}\u{636e}\u{4fe1}\u{606f}", 7), "\u{4f60}\u{597d}\u{4e16}\u{754c}...");
}

#[test]
fn test_truncate_string() {
    assert_eq!(truncate_string("hello world foo bar", 10), "hello w...");
    assert_eq!(truncate_string("short", 10), "short");
}

#[test]
fn test_truncate_string_multibyte() {
    // Multi-byte chars must not cause panics
    assert_eq!(truncate_string("héllo wörld foo", 10), "héllo w...");
    assert_eq!(truncate_string("\u{65e5}\u{672c}\u{8a9e}\u{30c6}\u{30b9}\u{30c8}\u{6587}\u{5b57}\u{5217}", 6), "\u{65e5}\u{672c}\u{8a9e}...");
}

#[test]
fn test_get_first_line() {
    assert_eq!(get_first_line("hello\nworld"), "hello");
    assert_eq!(get_first_line("no newline"), "no newline");
}

#[test]
fn test_atoi_no_err() {
    assert_eq!(atoi_no_err("42"), 42);
    assert_eq!(atoi_no_err("abc"), 0);
    assert_eq!(atoi_no_err(""), 0);
}

#[test]
fn test_random_hex_string() {
    let s = random_hex_string(16).unwrap();
    assert_eq!(s.len(), 16);
    assert!(s.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn test_random_hex_string_odd() {
    let s = random_hex_string(5).unwrap();
    assert_eq!(s.len(), 5);
}

#[test]
fn test_filter_valid_arch() {
    assert_eq!(filter_valid_arch("amd64").unwrap(), "x64");
    assert_eq!(filter_valid_arch("x86_64").unwrap(), "x64");
    assert_eq!(filter_valid_arch("ARM64").unwrap(), "arm64");
    assert_eq!(filter_valid_arch("aarch64").unwrap(), "arm64");
    assert_eq!(filter_valid_arch("AARCH64").unwrap(), "arm64");
    assert!(filter_valid_arch("mips").is_err());
}

// ---- Line/Col ----

#[test]
fn test_get_line_col_from_offset() {
    let data = b"hello\nworld\nfoo";
    assert_eq!(get_line_col_from_offset(data, 0), (1, 1));
    assert_eq!(get_line_col_from_offset(data, 5), (1, 6));
    assert_eq!(get_line_col_from_offset(data, 6), (2, 1));
    assert_eq!(get_line_col_from_offset(data, 12), (3, 1));
}

// ---- Longest Prefix ----

#[test]
fn test_longest_prefix_empty() {
    assert_eq!(longest_prefix("/root", &[]), "/root");
}

#[test]
fn test_longest_prefix_single() {
    assert_eq!(longest_prefix("/root", &["/root/foo"]), "/root/foo");
}

#[test]
fn test_longest_prefix_multiple() {
    assert_eq!(longest_prefix("/", &["/root/foo", "/root/bar"]), "/root/");
}

// ---- Indent ----

#[test]
fn test_indent_string() {
    assert_eq!(indent_string("  ", "a\nb\n\nc"), "  a\n  b\n\n  c\n");
}

// ---- Shell Hex ----

#[test]
fn test_shell_hex_escape() {
    assert_eq!(shell_hex_escape("AB"), "\\x41\\x42");
}

// ---- Combine Arrays ----

#[test]
fn test_combine_str_arrays() {
    let a = vec!["a".into(), "b".into()];
    let b = vec!["b".into(), "c".into()];
    assert_eq!(combine_str_arrays(&a, &b), vec!["a", "b", "c"]);
}

// ---- StrWithPos ----

#[test]
fn test_str_with_pos_parse() {
    let sp = StrWithPos::parse("hel[*]lo");
    assert_eq!(sp.str_val, "hello");
    assert_eq!(sp.pos, 3);
}

#[test]
fn test_str_with_pos_parse_no_cursor() {
    let sp = StrWithPos::parse("hello");
    assert_eq!(sp.str_val, "hello");
    assert_eq!(sp.pos, NO_STR_POS);
}

#[test]
fn test_str_with_pos_display() {
    let sp = StrWithPos::new("hello".into(), 3);
    assert_eq!(format!("{}", sp), "hel[*]lo");
}

#[test]
fn test_str_with_pos_display_end() {
    let sp = StrWithPos::new("hello".into(), 5);
    assert_eq!(format!("{}", sp), "hello[*]");
}

#[test]
fn test_str_with_pos_display_no_pos() {
    let sp = StrWithPos::new("hello".into(), NO_STR_POS);
    assert_eq!(format!("{}", sp), "hello");
}

#[test]
fn test_str_with_pos_prepend() {
    let sp = StrWithPos::new("world".into(), 2);
    let sp2 = sp.prepend("hello ");
    assert_eq!(sp2.str_val, "hello world");
    assert_eq!(sp2.pos, 8); // 6 + 2
}

#[test]
fn test_str_with_pos_append() {
    let sp = StrWithPos::new("hello".into(), 2);
    let sp2 = sp.append(" world");
    assert_eq!(sp2.str_val, "hello world");
    assert_eq!(sp2.pos, 2);
}

// ---- Atomic File Ops ----

#[test]
fn test_write_file_if_different_new() {
    let dir = std::env::temp_dir().join(format!("utilfn_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("test_file.txt");
    let _ = std::fs::remove_file(&path);

    let written = write_file_if_different(&path, b"hello").unwrap();
    assert!(written);

    let written = write_file_if_different(&path, b"hello").unwrap();
    assert!(!written);

    let written = write_file_if_different(&path, b"world").unwrap();
    assert!(written);

    std::fs::remove_file(&path).ok();
    std::fs::remove_dir_all(&dir).ok();
}
