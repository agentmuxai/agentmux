// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where the Claude Code CLI keeps a conversation under a `CLAUDE_CONFIG_DIR`.
//!
//! One implementation, because every reader has to agree with the CLI byte for
//! byte: a resume check that computes a different folder name than the CLI
//! reports a transcript that is sitting on disk as unreachable, and the agent
//! loses its native resume. Three copies of this rule used to disagree, and
//! the one on the resume path was wrong for any working directory holding an
//! underscore, a space or most other punctuation.

/// The CLI keeps this many characters of the sanitized path before it
/// appends a hash instead.
const PROJECT_DIR_NAME_MAX: usize = 200;

/// The folder under `<CLAUDE_CONFIG_DIR>/projects/` that holds the sessions
/// the CLI ran with `cwd` as its working directory.
///
/// The CLI's rule (its `RC()`, read from the binary AgentMux 0.57.0 bundles,
/// Claude Code v2.1.280):
///
/// ```js
/// function TQ(e){let r=0;for(let n=0;n<e.length;n++)r=(r<<5)-r+e.charCodeAt(n)|0;return r}
/// function RC(e){let n=e.replace(/[^a-zA-Z0-9]/g,"-");
///   if(n.length<=200)return n;return`${n.slice(0,200)}-${Math.abs(TQ(e)).toString(36)}`}
/// ```
///
/// JavaScript strings are UTF-16, so the rule works on UTF-16 code units: a
/// character outside the Basic Multilingual Plane (an emoji) becomes two
/// dashes, not one.
///
/// `cwd` must be the path as the CLI sees it — already expanded, not a
/// `~`-shorthand.
pub fn project_dir_name(cwd: &str) -> String {
    let name: String = cwd
        .encode_utf16()
        .map(|unit| match u8::try_from(unit) {
            Ok(b) if b.is_ascii_alphanumeric() => b as char,
            _ => '-',
        })
        .collect();
    if name.len() <= PROJECT_DIR_NAME_MAX {
        return name;
    }
    // Every character of `name` is ASCII, so slicing by bytes is slicing by
    // characters.
    format!(
        "{}-{}",
        &name[..PROJECT_DIR_NAME_MAX],
        radix_36(path_hash(cwd))
    )
}

/// `Math.abs(TQ(cwd))`: 31·h + c over UTF-16 code units, wrapping at 32 bits
/// (Java's `String.hashCode`). `i32::MIN`'s absolute value doesn't fit an
/// `i32`, and JavaScript numbers don't wrap, so the magnitude is returned
/// unsigned.
fn path_hash(s: &str) -> u32 {
    let mut hash: i32 = 0;
    for unit in s.encode_utf16() {
        hash = hash
            .wrapping_shl(5)
            .wrapping_sub(hash)
            .wrapping_add(i32::from(unit));
    }
    hash.unsigned_abs()
}

/// `Number.prototype.toString(36)` for a non-negative integer.
fn radix_36(mut n: u32) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".to_string();
    }
    let mut buf = Vec::new();
    while n > 0 {
        buf.push(DIGITS[(n % 36) as usize]);
        n /= 36;
    }
    buf.reverse();
    String::from_utf8(buf).expect("base-36 digits are ASCII")
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every expected value below was produced by the CLI's own `RC()`
    // (quoted on `project_dir_name`), run under Node — not by this module.

    #[test]
    fn a_windows_agent_directory() {
        assert_eq!(
            project_dir_name(r"C:\Users\asafe\.agentmux\agents\agent3-0630k"),
            "C--Users-asafe--agentmux-agents-agent3-0630k"
        );
    }

    #[test]
    fn a_posix_path_keeps_the_dashes_inside_a_segment() {
        assert_eq!(
            project_dir_name("/home/u/.agentmux/agents/foo-bar"),
            "-home-u--agentmux-agents-foo-bar"
        );
    }

    #[test]
    fn underscores_and_spaces_become_dashes() {
        assert_eq!(
            project_dir_name("/home/u/my_project dir"),
            "-home-u-my-project-dir"
        );
    }

    #[test]
    fn non_ascii_letters_and_punctuation_become_dashes() {
        assert_eq!(
            project_dir_name("C:\\code\\caf\u{e9} (old)@2~x"),
            "C--code-caf---old--2-x"
        );
    }

    #[test]
    fn a_character_outside_the_bmp_becomes_two_dashes() {
        assert_eq!(project_dir_name("/x/\u{1F600}y"), "-x---y");
    }

    #[test]
    fn a_long_path_is_cut_at_200_and_hashed() {
        let cwd = format!("/home/u/{}repo", "deeply_nested/".repeat(20));
        let got = project_dir_name(&cwd);
        assert_eq!(
            got,
            "-home-u-deeply-nested-deeply-nested-deeply-nested-deeply-nested-deeply-nested-\
             deeply-nested-deeply-nested-deeply-nested-deeply-nested-deeply-nested-deeply-nested-\
             deeply-nested-deeply-nested-deeply-nes-fygn57"
        );
        assert_eq!(got.len(), 207);
    }

    #[test]
    fn a_long_windows_path_is_cut_at_200_and_hashed() {
        let cwd = format!(r"C:\Users\asafe\{}", "a".repeat(230));
        assert_eq!(
            project_dir_name(&cwd),
            format!("C--Users-asafe-{}-py1bvd", "a".repeat(185))
        );
    }

    #[test]
    fn exactly_200_characters_is_not_hashed() {
        let cwd = "a".repeat(200);
        assert_eq!(project_dir_name(&cwd), cwd);
    }

    #[test]
    fn the_hash_magnitude_of_i32_min_does_not_wrap() {
        // JavaScript's Math.abs(-2147483648) is 2147483648; an i32 `abs`
        // would overflow.
        assert_eq!(radix_36(i32::MIN.unsigned_abs()), "zik0zk");
        assert_eq!(radix_36(0), "0");
    }
}
