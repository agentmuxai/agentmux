// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Utility functions ported from Go's `pkg/util/utilfn/utilfn.go`.
//!
//! Includes null encoding/decoding, hashing, binary detection, star matching,
//! slice operations, string helpers, and atomic file operations.

pub mod hash;
pub mod nullenc;
pub mod strutil;
#[cfg(test)]
mod tests;

// ---- Re-exports ----

#[allow(unused_imports)]
pub use hash::{quick_hash_string, sha1_hash};
#[allow(unused_imports)]
pub use nullenc::{
    decode_string_array, decode_string_map, encode_string_array, encode_string_map,
    encoded_string_array_get_first_val, encoded_string_array_has_first_val, null_decode_str,
    null_encode_str,
};
#[allow(unused_imports)]
pub use strutil::{
    add_elem_uniq, atomic_rename_copy, atoi_no_err, combine_str_arrays, ellipsis_str,
    filter_valid_arch, get_first_line, get_line_col_from_offset, has_binary_data, indent_string,
    is_binary_content, longest_prefix, move_to_front, random_hex_string, remove_elem,
    shell_hex_escape, slice_idx, star_match_string, truncate_string, write_file_if_different,
    StrWithPos, NO_STR_POS,
};
