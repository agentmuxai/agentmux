// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Stable 64-bit hash for deriving per-data-dir IPC names. Used to
// build the named-pipe path `\\.\pipe\agentmux-{hash16}\command`
// so each AgentMux instance (per CLAUDE.md, multiple parallel
// instances are supported per-data-dir) gets a distinct IPC
// surface, kernel-isolated from siblings.
//
// We hand-roll FNV-1a because:
//   * Adding `sha2` for ~64 bits of stable hash is overkill (3+ MB
//     deps) when this is non-cryptographic.
//   * `std::collections::hash_map::DefaultHasher` is explicitly
//     NOT documented as stable across runs / Rust versions; we
//     need stability so the same launcher binary always picks the
//     same pipe name for the same data dir.
//   * FNV-1a is deterministic, well-known, and ~20 lines of code.
//
// Collisions are non-cryptographic but adequate for this scope:
// the hash inputs are filesystem paths, the keyspace is tiny, and
// a collision just means two installs at different paths share a
// pipe name — they'd need to be running simultaneously AND have
// the SAME data dir hash, which is astronomically unlikely with
// 16 hex chars (64 bits) of namespace.

const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// 64-bit FNV-1a hash of bytes. Stable across runs.
pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// First 16 hex chars of FNV-1a-64 over the canonical-lowercase
/// representation of the data_dir path. Used as the per-instance
/// IPC namespace component.
pub fn data_dir_hash16(data_dir: &std::path::Path) -> String {
    // Canonicalize if possible (resolves `..`, mixed casing on
    // Windows), but fall back to the raw path if canonicalize fails
    // (e.g. data_dir doesn't exist yet during early startup).
    let canonical = data_dir
        .canonicalize()
        .unwrap_or_else(|_| data_dir.to_path_buf());
    let s = canonical.to_string_lossy().to_lowercase();
    format!("{:016x}", fnv1a_64(s.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a_known_vector() {
        // Standard test vector for FNV-1a-64 on empty input.
        assert_eq!(fnv1a_64(b""), FNV_OFFSET_BASIS);
        // From http://www.isthe.com/chongo/tech/comp/fnv/test_vectors.html
        // ("foobar" → 0x85944171f73967e8)
        assert_eq!(fnv1a_64(b"foobar"), 0x85944171f73967e8);
    }

    #[test]
    fn data_dir_hash_stable_across_calls() {
        let p = std::path::PathBuf::from("C:\\Users\\test\\AgentMux");
        assert_eq!(data_dir_hash16(&p), data_dir_hash16(&p));
        assert_eq!(data_dir_hash16(&p).len(), 16);
    }

    #[test]
    fn data_dir_hash_case_insensitive() {
        // Windows paths shouldn't produce different hashes for
        // different casings of the same logical path.
        let lower = std::path::PathBuf::from("c:\\users\\test");
        let upper = std::path::PathBuf::from("C:\\Users\\Test");
        assert_eq!(data_dir_hash16(&lower), data_dir_hash16(&upper));
    }
}
