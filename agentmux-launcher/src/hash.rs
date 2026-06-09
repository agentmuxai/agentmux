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
/// data_dir path **combined with the build version string**.
///
/// Including the version ensures that two different release binaries
/// (e.g. 0.40.2 and 0.41.0) that share the same channel data dir
/// (`~/.agentmux/channels/stable/`) produce DISTINCT pipe names and
/// therefore satisfy the CLAUDE.md multi-version concurrency guarantee:
/// each version is an independent single-instance domain.
///
/// Without the version, both binaries hash to the same pipe name and
/// the second one silently forwards its "open window" request to the
/// first, activating the wrong version. `version` should be the semver
/// string from `CARGO_PKG_VERSION` (e.g. `"0.41.0"`). The `\x00`
/// separator never appears in a filesystem path, so path + version
/// are always unambiguously distinguishable.
pub fn data_dir_hash16(data_dir: &std::path::Path, version: &str) -> String {
    let canonical = data_dir
        .canonicalize()
        .unwrap_or_else(|_| data_dir.to_path_buf());
    let combined = format!("{}\x00{}", canonical.to_string_lossy().to_lowercase(), version);
    format!("{:016x}", fnv1a_64(combined.as_bytes()))
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
        assert_eq!(data_dir_hash16(&p, "0.41.0"), data_dir_hash16(&p, "0.41.0"));
        assert_eq!(data_dir_hash16(&p, "0.41.0").len(), 16);
    }

    #[test]
    fn data_dir_hash_case_insensitive() {
        // Windows paths shouldn't produce different hashes for
        // different casings of the same logical path.
        let lower = std::path::PathBuf::from("c:\\users\\test");
        let upper = std::path::PathBuf::from("C:\\Users\\Test");
        assert_eq!(data_dir_hash16(&lower, "0.41.0"), data_dir_hash16(&upper, "0.41.0"));
    }

    #[test]
    fn different_versions_same_dir_produce_different_hashes() {
        // Core invariant: same data dir + different version → different pipe name.
        // This is what prevents 0.40.2 and 0.41.0 from colliding on single-instance.
        let p = std::path::PathBuf::from("C:\\Users\\test\\.agentmux\\channels\\stable\\data");
        assert_ne!(data_dir_hash16(&p, "0.40.2"), data_dir_hash16(&p, "0.41.0"));
    }

    #[test]
    fn same_version_different_dirs_produce_different_hashes() {
        let a = std::path::PathBuf::from("C:\\Users\\test\\.agentmux\\channels\\stable\\data");
        let b = std::path::PathBuf::from("C:\\Users\\test\\.agentmux\\channels\\beta\\data");
        assert_ne!(data_dir_hash16(&a, "0.41.0"), data_dir_hash16(&b, "0.41.0"));
    }

    #[test]
    fn portable_and_installed_never_collide() {
        // The real 2026-06-03 scenario: a local v0.42.0 build launched
        // alongside an installed v0.41.0. Different channel dir AND different
        // version → distinct single-instance pipe → safe to run in parallel,
        // per CLAUDE.md "Multiple Instances Run in Parallel" and
        // SPEC_MULTI_INSTANCE_ISOLATION_HARDENING_2026_06_03.md.
        let portable = std::path::PathBuf::from(
            "C:\\Users\\test\\.agentmux\\channels\\local-main-b28b7a\\versions\\0.42.0\\data",
        );
        let installed = std::path::PathBuf::from(
            "C:\\Users\\test\\.agentmux\\channels\\stable\\versions\\0.41.0\\data",
        );
        assert_ne!(
            data_dir_hash16(&portable, "0.42.0"),
            data_dir_hash16(&installed, "0.41.0")
        );
    }

    #[test]
    fn successive_local_builds_produce_different_hashes() {
        // Two successive `task package` runs on the same branch at the same
        // semver get different AGENTMUX_BUILD_LABELs (different stamps).
        // The pipe key uses the label, so each build is its own single-instance
        // domain — the second launch starts a fresh window rather than joining
        // the first build's running instance. Regression guard for the
        // 2026-06-09 isolation bug (retro: retro-local-build-isolation-regression).
        let data_dir = std::path::PathBuf::from(
            "C:\\Users\\test\\.agentmux\\channels\\local-main-b28b7a\\versions\\0.43.1\\data",
        );
        let label_a = "0.43.1+gabc1234.20260609T1100.12345";
        let label_b = "0.43.1+gabc1234.20260609T1145.67890";
        assert_ne!(
            data_dir_hash16(&data_dir, label_a),
            data_dir_hash16(&data_dir, label_b)
        );
    }
}
