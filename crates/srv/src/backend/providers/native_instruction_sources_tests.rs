// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

/// The write target must always be one of the paths the provider reads,
/// or AgentMux is writing a file nobody opens.
#[test]
fn every_write_target_is_also_a_read_source() {
    for (id, provider) in REGISTRY.iter() {
        let Some(target) = provider.startup_instructions_filename else {
            assert!(
                provider.native_instruction_sources.is_empty(),
                "{id}: no write target, so no confirmed native source either"
            );
            continue;
        };
        assert!(
            provider.native_instruction_sources.contains(&target),
            "{id}: writes {target} but does not list it as a native source"
        );
    }
}

/// Duplicates would make a resolved instruction set report the same file
/// twice.
#[test]
fn no_provider_lists_a_source_twice() {
    for (id, provider) in REGISTRY.iter() {
        let mut seen = std::collections::HashSet::new();
        for src in provider.native_instruction_sources {
            assert!(seen.insert(*src), "{id}: {src} listed more than once");
        }
    }
}

/// The two cases that motivated the field. Guarding them by name keeps a
/// future registry edit from quietly collapsing the set back to the write
/// target and re-hiding what those providers read.
#[test]
fn the_multi_source_providers_keep_their_extra_sources() {
    let copilot = get_provider("copilot").expect("copilot is registered");
    for expected in [
        "AGENTS.md",
        ".github/copilot-instructions.md",
        "CLAUDE.md",
        "GEMINI.md",
    ] {
        assert!(
            copilot.native_instruction_sources.contains(&expected),
            "copilot must still read {expected}"
        );
    }

    let pi = get_provider("pi").expect("pi is registered");
    assert!(
        pi.native_instruction_sources.contains(&".pi/SYSTEM.md"),
        "pi reads .pi/SYSTEM.md as well as the file AgentMux writes"
    );
}

/// Copilot's path-scoped instructions are a pattern, not a path, and a
/// resolver reading only the literal list would miss them entirely in any
/// repository that uses them (Codex, PR #3154).
#[test]
fn copilot_declares_its_path_scoped_instructions_directory() {
    let copilot = get_provider("copilot").expect("copilot is registered");
    let scan = copilot
        .native_instruction_dirs
        .iter()
        .find(|d| d.dir == ".github/instructions")
        .expect("copilot scans .github/instructions");
    assert_eq!(scan.suffix, ".instructions.md");
    assert!(scan.recursive, "the convention is `**`, so subdirectories count");
}

/// A scan entry that could never match anything is a silent hole.
#[test]
fn every_declared_scan_is_usable() {
    for (id, provider) in REGISTRY.iter() {
        for scan in provider.native_instruction_dirs {
            assert!(!scan.dir.trim().is_empty(), "{id}: scan with no directory");
            assert!(!scan.suffix.trim().is_empty(), "{id}: scan with no suffix");
            assert!(
                !scan.dir.starts_with('/') && !scan.dir.contains(".."),
                "{id}: scan dir must stay inside the working directory: {}",
                scan.dir
            );
        }
    }
}
