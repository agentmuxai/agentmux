// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The readers against hostile files (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §8.7).
//!
//! Every reader runs on a file somebody else made, so none may panic, hang or balloon on any bytes.
//! This is a mutation fuzzer that runs with `cargo test`: it takes real files of each format (the
//! KeePass fixtures, an export of each service), corrupts them in the ways a fuzzer does (flipped and
//! overwritten bytes, cut, repeated and spliced ranges, runaway nesting and runaway numbers), and
//! feeds the results to the reader, every time through [`read_keyed`] the way the app does, and
//! straight into the header check of a KeePass database. A panic, or a read that takes more than
//! [`SLOW`], fails the test with the seed and a copy of the input to replay.
//!
//! The same seed always makes the same inputs. `VAULT_FUZZ_ITERS` raises the count per seed
//! (default 60; a few hundred thousand is an overnight run), `VAULT_FUZZ_KDBX_EVERY` how often a
//! damaged database is read whole (default every 25th: each read stretches a password, which takes
//! seconds in a debug build), and `VAULT_FUZZ_SEED` starts from a different place. It is not coverage-guided: the readers sit in a binary crate, which a
//! `cargo-fuzz` target cannot link against, so splitting them into a library would be the next step
//! (spec §8.7).

use std::io::Write as _;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::{Duration, Instant};

use super::*;

/// The longest any one read may take (a database read whole is exempt, see [`feed`]).
const SLOW: Duration = Duration::from_secs(10);

/// The password of the KeePass fixtures, so a damaged database gets past its password and into the parser.
const PASS: &str = "correct horse battery";

/// xorshift64*: small, fast and the same everywhere.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// A number below `n` (`n > 0`).
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Text that makes parsers misbehave: nesting without end, huge and odd numbers, odd quotes.
const NASTY: &[&[u8]] = &[
    b"[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[",
    b"{\"a\":{\"a\":{\"a\":{\"a\":{\"a\":{\"a\":{\"a\":{\"a\":{\"a\":{\"a\":{\"a\":{\"a\":",
    b"99999999999999999999999999",
    b"-1",
    b"4294967295",
    b"18446744073709551615",
    b"\"\"\"\"\"\"",
    b"\xEF\xBB\xBF",
    b"\xFF\xFE\x00",
    b"\r\n\r\n\r\n",
    b"\0\0\0\0\0\0\0\0",
    b"=,=,=,=,=,=,=,=\n",
];

/// `seed` damaged in one to four ways. `others` are the other seeds, for splicing.
fn mutate(rng: &mut Rng, seed: &[u8], others: &[Vec<u8>]) -> Vec<u8> {
    let mut b = seed.to_vec();
    for _ in 0..=rng.below(4) {
        let len = b.len();
        match rng.below(9) {
            // A byte flipped, or set to an edge value.
            0 if len > 0 => {
                let i = rng.below(len);
                b[i] ^= 1 << rng.below(8);
            }
            1 if len > 0 => {
                let i = rng.below(len);
                b[i] = [0, 1, 0x7f, 0x80, 0xff, b'"', b'\\', b'\n', b','][rng.below(9)];
            }
            // A range removed.
            2 if len > 1 => {
                let from = rng.below(len);
                let to = (from + 1 + rng.below(len.min(64))).min(len);
                b.drain(from..to);
            }
            // A range repeated (growing a field, a row, a header).
            3 if len > 1 => {
                let from = rng.below(len);
                let to = (from + 1 + rng.below(len.min(256))).min(len);
                let piece = b[from..to].to_vec();
                let at = rng.below(len);
                for _ in 0..1 + rng.below(40) {
                    b.splice(at..at, piece.iter().copied());
                }
            }
            // Cut short.
            4 if len > 1 => b.truncate(rng.below(len)),
            // Something nasty put in.
            5 => {
                let at = if len == 0 { 0 } else { rng.below(len + 1) };
                let piece = NASTY[rng.below(NASTY.len())];
                for _ in 0..1 + rng.below(30) {
                    b.splice(at..at, piece.iter().copied());
                }
            }
            // Random bytes put in.
            6 => {
                let at = if len == 0 { 0 } else { rng.below(len + 1) };
                let junk: Vec<u8> = (0..1 + rng.below(32)).map(|_| rng.next() as u8).collect();
                b.splice(at..at, junk);
            }
            // Part of another seed spliced in.
            7 if !others.is_empty() => {
                let other = &others[rng.below(others.len())];
                if !other.is_empty() {
                    let from = rng.below(other.len());
                    let to = (from + 1 + rng.below(other.len().min(512))).min(other.len());
                    let at = if len == 0 { 0 } else { rng.below(len + 1) };
                    b.splice(at..at, other[from..to].iter().copied());
                }
            }
            // A run of one byte written over a range.
            8 if len > 1 => {
                let from = rng.below(len);
                let to = (from + 1 + rng.below(len.min(128))).min(len);
                let v = rng.next() as u8;
                b[from..to].fill(v);
            }
            _ => {}
        }
    }
    b
}

fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut out));
        for (name, bytes) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }
    out
}

/// A file of each format the readers take, with the name it would have.
fn seeds() -> Vec<(&'static str, Vec<u8>)> {
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/backend/vault/import/fixtures");
    let mut s: Vec<(&'static str, Vec<u8>)> = Vec::new();
    for f in ["pykeepass_plain.kdbx", "pykeepass_custom_fields.kdbx"] {
        s.push(("db.kdbx", std::fs::read(fixtures.join(f)).unwrap()));
    }
    s.push(("lastpass_export.csv", super::tests::LASTPASS.as_bytes().to_vec()));
    s.push(("dashlane_credentials.csv", super::second_wave_tests::DASHLANE_CREDENTIALS.as_bytes().to_vec()));
    s.push(("dashlane_payments.csv", super::second_wave_tests::DASHLANE_PAYMENTS.as_bytes().to_vec()));
    s.push(("dashlane_ids.csv", super::second_wave_tests::DASHLANE_IDS.as_bytes().to_vec()));
    s.push(("dashlane_personal.csv", super::second_wave_tests::DASHLANE_PERSONAL.as_bytes().to_vec()));
    s.push(("dashlane_notes.csv", super::second_wave_tests::DASHLANE_NOTES.as_bytes().to_vec()));
    s.push(("nordpass.csv", super::second_wave_tests::NORDPASS.as_bytes().to_vec()));
    s.push(("roboform.csv", super::second_wave_tests::ROBOFORM.as_bytes().to_vec()));
    s.push(("zoho.csv", super::second_wave_tests::ZOHO.as_bytes().to_vec()));
    s.push(("keeper.csv", super::second_wave_tests::KEEPER.as_bytes().to_vec()));
    s.push(("bitwarden.json", super::tests::BITWARDEN.as_bytes().to_vec()));
    s.push(("bitwarden.csv", b"folder,favorite,type,name,notes,fields,reprompt,login_uri,login_username,login_password,login_totp\nWork,0,login,GitHub,\"two\nlines\",,0,https://github.com,alex,\"pw, \"\"quoted\"\"\",JBSWY3DPEHPK3PXP\n,0,note,Wifi,ssid: home,,0,,,,\n".to_vec()));
    s.push(("chrome.csv", b"name,url,username,password,note\nExample,https://example.com/login,alex,hunter2,\nSecond,https://b.example,bob,\"p\"\"w\",hi\n".to_vec()));
    s.push((".env", b"# keys\nSTRIPE_API_KEY=sk_live_abcdefgh1234\nexport GITHUB_TOKEN='ghp_abcdefghij'\nEMPTY=\nQUOTED=\"a b c\"\n".to_vec()));
    s.push(("vault.json", br#"{"format":"agentmux-vault","version":1,"exported_at":1,"app_version":"x","entries":[{"kind":"ssn","label":"A","parts":{"value":"123-45-6789"},"fields":{"owner":"A"},"tags":["T"]}]}"#.to_vec()));
    let data = serde_json::json!({ "accounts": [{ "attrs": { "accountName": "Alex" }, "vaults": [{ "attrs": { "name": "Personal" }, "items": [{
        "categoryUuid": "001",
        "overview": { "title": "Example", "url": "https://example.com", "urls": [{ "url": "https://example.com" }] },
        "details": { "loginFields": [{ "designation": "password", "value": "hunter2" }], "notesPlain": "n", "sections": [{ "fields": [{ "id": "totp", "title": "one-time password", "value": { "totp": "otpauth://totp/E?secret=JBSWY3DPEHPK3PXP" } }] }] }
    }] }] }] });
    s.push(("export.1pux", zip_of(&[("export.attributes", b"{}"), ("export.data", data.to_string().as_bytes())])));
    s
}

/// What a failure prints to replay it: the seed, the name, and the input in hex (cut if long).
fn describe(seed: u64, name: &str, input: &[u8]) -> String {
    let shown: String = input.iter().take(256).map(|b| format!("{b:02x}")).collect();
    let more = if input.len() > 256 { format!("… ({} bytes)", input.len()) } else { String::new() };
    format!("seed {seed} on {name}: {shown}{more}")
}

/// Runs the ways the app reads `input`; the failures it caused. `header_only` runs just the KeePass
/// header check: reading a whole database stretches a password (a second or so each), which
/// a header check does not.
fn feed(seed: u64, file_name: &str, input: &[u8], header_only: bool, failures: &mut Vec<String>) {
    let started = Instant::now();
    if catch_unwind(AssertUnwindSafe(|| {
        let _ = kdbx::check_header(input);
    }))
    .is_err()
    {
        failures.push(format!("panic in check_header: {}", describe(seed, file_name, input)));
    } else if started.elapsed() > SLOW {
        failures.push(format!("check_header took {:?}: {}", started.elapsed(), describe(seed, file_name, input)));
    }
    if header_only {
        return;
    }
    // The reader is chosen by content, but `.env` goes by name too. A database is read once: its
    // name makes no difference to it, and each read stretches a password.
    let names: &[&str] = if input.starts_with(&kdbx::MAGIC) { &["x.kdbx"] } else { &[file_name, "x.csv", "x.json", "x.env", "x"] };
    for name in names {
        let started = Instant::now();
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let _ = read_keyed(input, name, None, Some(PASS), None);
            let _ = read_keyed(input, name, None, None, Some(b"key file bytes"));
        }));
        if outcome.is_err() {
            failures.push(format!("panic: {}", describe(seed, name, input)));
        } else if started.elapsed() > SLOW && !input.starts_with(&kdbx::MAGIC) {
            // A database is exempt: its header may ask, within the caps (spec §8.7), for a password
            // stretch of a gigabyte and a hundred rounds, which takes seconds and is the cap's design.
            failures.push(format!("took {:?}: {}", started.elapsed(), describe(seed, name, input)));
        }
    }
}

fn iterations() -> u64 {
    std::env::var("VAULT_FUZZ_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(60)
}

/// How often a damaged database is read whole rather than only header-checked.
fn kdbx_every() -> u64 {
    std::env::var("VAULT_FUZZ_KDBX_EVERY").ok().and_then(|v| v.parse().ok()).filter(|n| *n > 0).unwrap_or(25)
}

fn first_seed() -> u64 {
    std::env::var("VAULT_FUZZ_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(1)
}

/// Mutated copies of one seed file, from `first` on.
fn fuzz(name: &str, seeds_of_kind: &[(&'static str, Vec<u8>)], others: &[Vec<u8>], failures: &mut Vec<String>) {
    let (file_name, base) = seeds_of_kind.iter().find(|(n, _)| *n == name).map(|(n, b)| (*n, b.clone())).expect("a seed");
    let first = first_seed();
    for k in 0..iterations() {
        let seed = first + k;
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let input = mutate(&mut rng, &base, others);
        // Every database goes through the header check; one in `kdbx_every` is read whole.
        feed(seed, file_name, &input, base.starts_with(&kdbx::MAGIC) && k % kdbx_every() != 0, failures);
    }
}

#[test]
fn no_reader_panics_or_hangs_on_a_damaged_export_of_any_format() {
    let all = seeds();
    let bodies: Vec<Vec<u8>> = all.iter().map(|(_, b)| b.clone()).collect();
    let mut failures = Vec::new();
    for (name, _) in &all {
        fuzz(name, &all, &bodies, &mut failures);
    }
    failures.dedup();
    assert!(failures.is_empty(), "{} failure(s), first ones:\n{}", failures.len(), failures.iter().take(5).cloned().collect::<Vec<_>>().join("\n"));
}

#[test]
fn the_unmodified_seeds_read_without_a_panic() {
    // A check on the harness itself: if a seed stopped parsing, the fuzzing above would be of nothing.
    let mut failures = Vec::new();
    let mut readable = 0;
    for (name, bytes) in seeds() {
        feed(0, name, &bytes, false, &mut failures);
        if read_keyed(&bytes, name, None, Some(PASS), None).is_ok() || read_keyed(&bytes, name, None, None, None).is_ok() {
            readable += 1;
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
    assert!(readable >= 7, "only {readable} of the seeds read: the corpus is broken");
}

#[test]
fn the_same_seed_makes_the_same_input() {
    let all = seeds();
    let bodies: Vec<Vec<u8>> = all.iter().map(|(_, b)| b.clone()).collect();
    let make = |seed: u64| mutate(&mut Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1), &all[2].1, &bodies);
    assert_eq!(make(7), make(7));
    assert_ne!(make(7), make(8));
}
