// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The file that keeps a headless srv's browser sessions across a restart (`sessions.bin`, in the data root).
//! docs/specs/SPEC_WINDOWS_HEADLESS_AS_A_SERVICE_2026_11_20.md D.2.
//!
//! What it holds: for each live session the SHA-256 of its cookie and of its per-tab token (the same digests the store keeps
//! in memory, never a cookie or a token), when it was confirmed and when it was last used, as wall-clock seconds. One-time
//! codes and pending sessions are never written. The file is written by a temp file and a rename, owner-only, when a session is
//! confirmed or ended and (at most every [`SAVE_EVERY`]) when one is used.
//!
//! What it refuses: a file that cannot be trusted is ignored, every session in it is cleared, and the reason is logged once
//! (never a digest). That is a file whose folder or own access list lets anyone but the owner, the system and the
//! administrators open it, one that is truncated, has extra bytes, does not parse or fails its check value, one saved at a time
//! after now (the clock has moved back, so the time that has passed is unknown), one with a time that cannot be (a session
//! used before it was confirmed, or after the file was saved), and one made under a different sign-in key when the key is one
//! the person provided. A key srv generated is new at every start, so a session cannot depend on it: the check value of such a
//! file is a plain SHA-256, and of a file made under a provided key an HMAC under that key.

use std::path::{Path, PathBuf};
use std::time::Duration;

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

pub(crate) type Digest32 = [u8; 32];

/// The file's name in the data root.
pub(crate) const FILE_NAME: &str = "sessions.bin";
/// A use of a session is written at most this often; the saved time of last use is then at most this stale, which makes a
/// session older after a restart, never younger.
pub(crate) const SAVE_EVERY: Duration = Duration::from_secs(5 * 60);

const MAGIC: &[u8; 8] = b"AMXSESS1";
const HEADER_LEN: usize = 8 + 1 + 8 + 4;
const RECORD_LEN: usize = 32 + 32 + 8 + 8;
const TRAILER_LEN: usize = 32;
/// Most sessions the file holds or accepts.
pub(crate) const MAX_SESSIONS: usize = 4096;

/// What the file is bound to: nothing (the key was generated at this start), or the key the person provided.
#[derive(Clone)]
pub(crate) enum KeyBinding {
    Generated,
    Provided(Vec<u8>),
}

impl KeyBinding {
    fn mode(&self) -> u8 {
        match self {
            KeyBinding::Generated => 0,
            KeyBinding::Provided(_) => 1,
        }
    }

    fn check(&self, body: &[u8]) -> Digest32 {
        match self {
            KeyBinding::Generated => Sha256::digest(body).into(),
            KeyBinding::Provided(key) => {
                let mut m = Hmac::<Sha256>::new_from_slice(key).expect("hmac takes any key length");
                m.update(b"amx-sessions-file-v1");
                m.update(body);
                m.finalize().into_bytes().into()
            }
        }
    }
}

/// One saved session: the digests and the wall-clock seconds of its confirmation and last use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Record {
    pub cookie: Digest32,
    pub token: Digest32,
    pub born_secs: u64,
    pub last_seen_secs: u64,
}

/// Why the file was not used. A fixed text: it names no value from the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refused {
    /// Someone besides the owner, the system and the administrators can open the file or its folder (or it could not be asked).
    TooOpen,
    /// Truncated, too long, not this format or its check value does not match (also: the file was made under a provided key
    /// and the key now is another, or the other way round).
    Damaged,
    /// Saved at a time after now: the clock has moved back.
    ClockMovedBack,
    /// A time in it that cannot be.
    Inconsistent,
}

impl Refused {
    pub(crate) fn why(self) -> &'static str {
        match self {
            Refused::TooOpen => "the file or its folder can be opened by someone else",
            Refused::Damaged => "the file is damaged, or the sign-in key it was made under has changed",
            Refused::ClockMovedBack => "the clock is earlier than when the file was saved",
            Refused::Inconsistent => "the file holds a time that cannot be",
        }
    }
}

/// The bytes of a file of `records`, saved at `saved_secs`.
pub(crate) fn encode(binding: &KeyBinding, saved_secs: u64, records: &[Record]) -> Vec<u8> {
    let records = &records[..records.len().min(MAX_SESSIONS)];
    let mut out = Vec::with_capacity(HEADER_LEN + records.len() * RECORD_LEN + TRAILER_LEN);
    out.extend_from_slice(MAGIC);
    out.push(binding.mode());
    out.extend_from_slice(&saved_secs.to_be_bytes());
    out.extend_from_slice(&(records.len() as u32).to_be_bytes());
    for r in records {
        out.extend_from_slice(&r.cookie);
        out.extend_from_slice(&r.token);
        out.extend_from_slice(&r.born_secs.to_be_bytes());
        out.extend_from_slice(&r.last_seen_secs.to_be_bytes());
    }
    let check = binding.check(&out);
    out.extend_from_slice(&check);
    out
}

/// The sessions in `bytes`, if it is a whole file made under `binding` and saved at or before `now_secs`.
pub(crate) fn decode(binding: &KeyBinding, now_secs: u64, bytes: &[u8]) -> Result<Vec<Record>, Refused> {
    if bytes.len() < HEADER_LEN + TRAILER_LEN || &bytes[..8] != MAGIC {
        return Err(Refused::Damaged);
    }
    let count = u32::from_be_bytes(bytes[17..21].try_into().expect("four bytes")) as usize;
    if count > MAX_SESSIONS || bytes.len() != HEADER_LEN + count * RECORD_LEN + TRAILER_LEN {
        return Err(Refused::Damaged);
    }
    let (body, trailer) = bytes.split_at(bytes.len() - TRAILER_LEN);
    if bytes[8] != binding.mode() || !agentmux_common::secret_eq::secret_eq(&binding.check(body), trailer) {
        return Err(Refused::Damaged);
    }
    let saved = u64::from_be_bytes(bytes[9..17].try_into().expect("eight bytes"));
    if saved > now_secs {
        return Err(Refused::ClockMovedBack);
    }
    let mut out = Vec::with_capacity(count);
    for chunk in body[HEADER_LEN..].chunks_exact(RECORD_LEN) {
        let r = Record {
            cookie: chunk[..32].try_into().expect("32 bytes"),
            token: chunk[32..64].try_into().expect("32 bytes"),
            born_secs: u64::from_be_bytes(chunk[64..72].try_into().expect("eight bytes")),
            last_seen_secs: u64::from_be_bytes(chunk[72..80].try_into().expect("eight bytes")),
        };
        if r.born_secs > r.last_seen_secs || r.last_seen_secs > saved {
            return Err(Refused::Inconsistent);
        }
        out.push(r);
    }
    Ok(out)
}

/// Whether the file and its folder can be opened by nobody but the owner, the system and the administrators. A failure to
/// find out is a refusal.
pub(crate) fn private_enough(path: &Path) -> Result<(), Refused> {
    #[cfg(windows)]
    {
        let folder = path.parent().ok_or(Refused::TooOpen)?;
        for p in [path, folder] {
            match agentmux_common::owner_only::others_who_can_open(p) {
                Ok(others) if others.is_empty() => {}
                _ => return Err(Refused::TooOpen),
            }
        }
        Ok(())
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let file = std::fs::metadata(path).map_err(|_| Refused::TooOpen)?;
        let folder = std::fs::metadata(path.parent().ok_or(Refused::TooOpen)?).map_err(|_| Refused::TooOpen)?;
        // The file is the owner's alone; the folder may not be written by anyone else.
        if file.permissions().mode() & 0o077 != 0 || folder.permissions().mode() & 0o022 != 0 {
            return Err(Refused::TooOpen);
        }
        Ok(())
    }
}

/// Read the file: `Ok(None)` when there is none, the sessions when it can be trusted. On any refusal the file is deleted, so
/// a later start does not meet it again.
pub(crate) fn load(path: &Path, binding: &KeyBinding, now_secs: u64) -> Result<Option<Vec<Record>>, Refused> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => {
            let _ = clear(path, binding, now_secs);
            return Err(Refused::TooOpen);
        }
    };
    let result = private_enough(path).and_then(|()| decode(binding, now_secs, &bytes));
    if result.is_err() {
        let _ = clear(path, binding, now_secs);
    }
    result.map(Some)
}

/// Write the file: a temp file in the same folder (owner-only before it holds anything on unix, and with its list set on
/// Windows) and a rename over the old one.
pub(crate) fn save(path: &Path, binding: &KeyBinding, saved_secs: u64, records: &[Record]) -> std::io::Result<()> {
    let folder = path.parent().ok_or_else(|| std::io::Error::other("the sessions file has no folder"))?;
    let mut tmp: PathBuf = folder.to_path_buf();
    tmp.push(format!(".{FILE_NAME}.{}.tmp", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    use std::io::Write;
    let written = opts.open(&tmp).and_then(|mut f| {
        f.write_all(&encode(binding, saved_secs, records))?;
        f.sync_all()
    });
    #[cfg(windows)]
    let written = written.and_then(|()| agentmux_common::owner_only::restrict_to_user(&tmp).map_err(std::io::Error::other));
    let renamed = written.and_then(|()| std::fs::rename(&tmp, path));
    if renamed.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    renamed
}

/// Delete the file (a logout, or a refusal). Nothing is wrong if it is not there. When it cannot be deleted (a sharing
/// violation, a scanner holding it) it is replaced by a file with no sessions; only when that fails too is the error returned,
/// and the file, with whatever sessions it holds, is still there.
pub(crate) fn clear(path: &Path, binding: &KeyBinding, now_secs: u64) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => save(path, binding, now_secs, &[]),
    }
}

#[cfg(test)]
#[path = "tests/local_session_file_tests.rs"]
mod tests;
