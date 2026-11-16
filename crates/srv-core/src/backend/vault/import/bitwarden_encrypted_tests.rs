// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Bitwarden's password-protected export: the primitives against known answers from Bitwarden's
//! own source, and whole files written by independent libraries (Python's `cryptography` and
//! `argon2-cffi`, from the published algorithm).

use super::super::{read_with, ReadError, Source};
use super::*;

// ── Known answers from Bitwarden's `bitwarden-crypto` (sdk-internal) ───────────────────────────

#[test]
fn the_pbkdf2_key_matches_bitwardens_test_vector() {
    let key = master_key(b"67t9b5g67$%Dh89n", b"test_key", &Kdf::Pbkdf2 { rounds: 10_000 }).unwrap();
    let want: [u8; 32] = [31, 79, 104, 226, 150, 71, 177, 90, 194, 80, 172, 209, 17, 129, 132, 81, 138, 167, 69, 167, 254, 149, 2, 27, 39, 197, 64, 42, 22, 195, 86, 75];
    assert_eq!(&key[..], &want[..]);
}

#[test]
fn the_argon2id_key_matches_bitwardens_test_vector_with_the_salt_hashed() {
    let key = master_key(b"67t9b5g67$%Dh89n", b"test_key", &Kdf::Argon2id { memory_mib: 32, iterations: 4, lanes: 2 }).unwrap();
    let want: [u8; 32] = [207, 240, 225, 177, 162, 19, 163, 76, 98, 106, 179, 175, 224, 9, 17, 240, 20, 147, 237, 47, 246, 150, 141, 184, 62, 225, 131, 242, 51, 53, 225, 242];
    assert_eq!(&key[..], &want[..]);
}

#[test]
fn hkdf_expand_matches_bitwardens_test_vector() {
    let prk: [u8; 32] = [23, 152, 120, 41, 214, 16, 156, 133, 71, 226, 178, 135, 208, 255, 66, 101, 189, 70, 173, 30, 39, 215, 175, 236, 38, 180, 180, 62, 196, 4, 159, 70];
    let want: [u8; 64] = [
        6, 114, 42, 38, 87, 231, 30, 109, 30, 255, 104, 129, 255, 94, 92, 108, 124, 145, 215, 208, 17, 60, 135, 22, 70, 158, 40, 53, 45, 182, 8, 63, 65, 87, 239, 234, 185, 227, 153, 122, 115, 205, 144, 56,
        102, 149, 92, 139, 217, 102, 119, 57, 37, 57, 251, 178, 18, 52, 94, 77, 132, 215, 239, 100,
    ];
    let mut out = [0u8; 64];
    hkdf_expand(&prk, b"info", &mut out).unwrap();
    assert_eq!(out, want);
}

#[test]
fn a_type_2_enc_string_decrypts_bitwardens_test_vector_and_checks_its_mac() {
    let key: [u8; 64] = std::array::from_fn(|i| i as u8);
    let iv: [u8; 16] = [216, 218, 36, 0, 196, 186, 150, 85, 49, 147, 110, 168, 185, 227, 42, 172];
    let mac: [u8; 32] = [60, 78, 44, 111, 72, 233, 3, 6, 86, 250, 217, 242, 62, 229, 184, 221, 231, 150, 189, 44, 99, 189, 220, 55, 196, 194, 101, 60, 102, 195, 149, 130];
    let data: [u8; 32] = [234, 77, 16, 15, 189, 82, 36, 188, 182, 88, 64, 67, 145, 94, 30, 178, 36, 235, 130, 67, 255, 207, 183, 168, 73, 231, 82, 122, 193, 139, 25, 129];
    let b64 = base64::engine::general_purpose::STANDARD;
    let enc = format!("2.{}|{}|{}", b64.encode(iv), b64.encode(data), b64.encode(mac));
    assert_eq!(&decrypt_enc_string(&enc, &key).unwrap()[..], b"Bitwarden SDK test vector");
    // One bit of the MAC, one of the data: refused as a failed check, not decrypted.
    let mut bad_mac = mac;
    bad_mac[0] ^= 1;
    assert_eq!(decrypt_enc_string(&format!("2.{}|{}|{}", b64.encode(iv), b64.encode(data), b64.encode(bad_mac)), &key).unwrap_err(), Failed::Mac);
    let mut bad_data = data;
    bad_data[5] ^= 1;
    assert_eq!(decrypt_enc_string(&format!("2.{}|{}|{}", b64.encode(iv), b64.encode(bad_data), b64.encode(mac)), &key).unwrap_err(), Failed::Mac);
    // Not type 2, or not well formed.
    for bad in ["0.aaaa|bbbb", "7.abcd", "2.only|two", "2.a|b|c|d", "", "2.!!!|!!!|!!!"] {
        assert_eq!(decrypt_enc_string(bad, &key).unwrap_err(), Failed::Form, "{bad}");
    }
}

// ── Whole files ────────────────────────────────────────────────────────────────────────────────

const PBKDF2_FILE: &str = include_str!("fixtures/bitwarden_pbkdf2.json");
const ARGON2_FILE: &str = include_str!("fixtures/bitwarden_argon2id.json");
/// The fixtures' password, with its spaces.
const PASSWORD: &str = " pass phrase ";

fn imported(file: &str, password: &str) -> super::super::Imported {
    match read_with(file.as_bytes(), "bitwarden_encrypted_export.json", None, Some(password)) {
        Ok(i) => i,
        Err(ReadError::Other(e)) => panic!("{e}"),
        Err(_) => panic!("unexpected"),
    }
}

#[test]
fn a_password_protected_export_opens_with_its_password_whichever_kdf_it_uses() {
    for (name, file) in [("pbkdf2", PBKDF2_FILE), ("argon2id", ARGON2_FILE)] {
        let i = imported(file, PASSWORD);
        assert_eq!(i.source, Source::Bitwarden, "{name}");
        assert!(!i.plaintext, "{name}: it was encrypted, so the pane doesn't offer to delete it as a plaintext file");
        let by = |label: &str| i.entries.iter().find(|e| e.label == label).unwrap_or_else(|| panic!("{name}: no {label}: {:?}", i.entries.iter().map(|e| &e.label).collect::<Vec<_>>()));
        let login = by("Example");
        assert_eq!((login.kind.as_str(), login.fields["username"].as_str()), ("login", "alex"), "{name}");
        assert_eq!(login.parts["password"], "  hunter2 with spaces  ", "{name}: a password keeps its spaces");
        assert_eq!(login.parts["totp"], "JBSWY3DPEHPK3PXP");
        assert_eq!(login.tags, vec!["Shopping".to_string()]);
        assert!(by("Example (notes)").parts["text"].contains("recovery 4455") && by("Example (notes)").parts["text"].contains("PIN: 9876"));
        assert_eq!((by("Visa").kind.as_str(), by("Visa").parts["number"].as_str(), by("Visa").parts["expiry"].as_str()), ("card", "4111111111111111", "12/2030"));
        assert_eq!(by("Wifi").kind, "note");
        assert_eq!(by("Me (Social Security number)").parts["value"], "123-45-6789");
    }
}

#[test]
fn it_asks_for_the_password_first_and_says_when_it_is_wrong() {
    for file in [PBKDF2_FILE, ARGON2_FILE] {
        match read_with(file.as_bytes(), "x.json", None, None) {
            Err(ReadError::NeedsPassphrase) => {}
            _ => panic!("a password is asked for"),
        }
        match read_with(file.as_bytes(), "x.json", None, Some("")) {
            Err(ReadError::NeedsPassphrase) => {}
            _ => panic!("an empty one is no password"),
        }
        // Trimming it would open the file: the password is its exact bytes.
        for wrong in ["pass phrase", "wrong", " pass phrase", "pass phrase "] {
            match read_with(file.as_bytes(), "x.json", None, Some(wrong)) {
                Err(ReadError::Other(e)) => assert!(e.contains("didn't open"), "{wrong:?}: {e}"),
                _ => panic!("{wrong:?} refused"),
            }
        }
    }
}

#[test]
fn a_file_asking_for_too_much_key_stretching_is_refused_before_a_password_is_tried() {
    let with = |patch: &dyn Fn(&mut serde_json::Value)| {
        let mut v: serde_json::Value = serde_json::from_str(ARGON2_FILE).unwrap();
        patch(&mut v);
        v.to_string()
    };
    for (what, file) in [
        ("memory", with(&|v| v["kdfMemory"] = (MAX_ARGON_MEMORY_MIB + 1).into())),
        ("iterations", with(&|v| v["kdfIterations"] = (MAX_ARGON_ITERATIONS + 1).into())),
        ("lanes", with(&|v| v["kdfParallelism"] = (MAX_ARGON_LANES + 1).into())),
        ("zero memory", with(&|v| v["kdfMemory"] = 0.into())),
        ("pbkdf2 rounds", { let mut v: serde_json::Value = serde_json::from_str(PBKDF2_FILE).unwrap(); v["kdfIterations"] = (MAX_PBKDF2_ROUNDS + 1).into(); v.to_string() }),
    ] {
        // No password at all: the cost is refused before anything asks for one.
        match read_with(file.as_bytes(), "x.json", None, None) {
            Err(ReadError::Other(e)) => assert!(e.contains("more key-stretching"), "{what}: {e}"),
            _ => panic!("{what} refused"),
        }
    }
    // Argon2 without its parameters, and an unknown kdf.
    let missing = with(&|v| {
        v.as_object_mut().unwrap().remove("kdfMemory");
    });
    assert!(matches!(read_with(missing.as_bytes(), "x.json", None, Some(PASSWORD)), Err(ReadError::Other(_))));
    let unknown = with(&|v| v["kdfType"] = 9.into());
    assert!(matches!(read_with(unknown.as_bytes(), "x.json", None, Some(PASSWORD)), Err(ReadError::Other(e)) if e.contains("key derivation")));
}

#[test]
fn a_changed_or_unsupported_file_is_refused_without_decrypting_it() {
    // The data changed: its check fails and nothing is read.
    let mut v: serde_json::Value = serde_json::from_str(PBKDF2_FILE).unwrap();
    let data = v["data"].as_str().unwrap().to_string();
    let flipped = data.replacen("|", "|AA", 1);
    v["data"] = flipped.into();
    match read_with(v.to_string().as_bytes(), "x.json", None, Some(PASSWORD)) {
        Err(ReadError::Other(e)) => assert!(e.contains("damaged") || e.contains("isn't one"), "{e}"),
        _ => panic!("refused"),
    }
    // An encryption type a newer Bitwarden may write.
    let mut v: serde_json::Value = serde_json::from_str(PBKDF2_FILE).unwrap();
    v["encKeyValidation_DO_NOT_EDIT"] = "7.pQEBAlgQ".into();
    match read_with(v.to_string().as_bytes(), "x.json", None, Some(PASSWORD)) {
        Err(ReadError::Other(e)) => assert!(e.contains("doesn't read yet"), "{e}"),
        _ => panic!("refused"),
    }
    // Encrypted with the account's own key (no password protection) can't be opened here.
    match read_with(br#"{"encrypted": true, "encKeyValidation_DO_NOT_EDIT": "2.a|b|c", "data": "2.a|b|c"}"#, "x.json", None, Some(PASSWORD)) {
        Err(ReadError::Other(e)) => assert!(e.contains("account's own key"), "{e}"),
        _ => panic!("refused"),
    }
}
