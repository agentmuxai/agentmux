// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The end of a provider session (`SPEC_HISTORY_COMPACT_FORM_ONLY_2026_11_23.md` section 5.2 to 5.4): prove that the
//! store holds what the provider's files hold, import what it does not, and only then remove the files, by session id.
//!
//! Nothing here decides *when* a session ends or *which* ids AgentMux started; the caller passes one session's files.
//! The proof is [`super::provider`]'s: a file's records matched against the store's, in order. What does not match is
//! written to the zone as it is (`import/<session>/<file>`), a provider file that is not line records (a spill file)
//! is imported whole, and the fields a matched record has that the store's line lacks go to `output.provider`.
//! After the import the written data is read back and compared; **a file is removed only when every record and every
//! file of the session is matched or read back equal.** A failure removes nothing and reports counts only.

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::provider::{record, sidecar_fields, Matcher, Record, StoreIndex};
use super::withhold;
use crate::backend::agent_session::OUTPUT_FILE;

/// The file in a zone that keeps what the provider's matched records carry beyond the store's lines.
pub const SIDECAR_FILE: &str = "output.provider";

/// How much of a file is read or written at a time.
const CHUNK: u64 = 4 << 20;

/// A zone of the store, as the proof and the import use it.
pub trait Zone {
    fn size(&self, name: &str) -> Result<u64, String>;
    fn chunk(&self, name: &str, offset: u64, len: u64) -> Result<Vec<u8>, String>;
    /// Append, creating the file when it is not there.
    fn append(&self, name: &str, bytes: &[u8]) -> Result<(), String>;
    fn delete(&self, name: &str) -> Result<(), String>;

    fn read_all(&self, name: &str) -> Result<Vec<u8>, String> {
        let size = self.size(name)?;
        let mut out = Vec::with_capacity(size as usize);
        let mut at = 0;
        while at < size {
            let part = self.chunk(name, at, CHUNK.min(size - at))?;
            if part.is_empty() {
                return Err(format!("short read of {name}"));
            }
            at += part.len() as u64;
            out.extend_from_slice(&part);
        }
        Ok(out)
    }
}

/// One zone of a [`FileStore`](crate::backend::storage::filestore::FileStore).
pub struct FsZone<'a> {
    pub fs: &'a crate::backend::storage::filestore::FileStore,
    pub zone: &'a str,
}

impl Zone for FsZone<'_> {
    fn size(&self, name: &str) -> Result<u64, String> {
        Ok(self.fs.stat(self.zone, name).map_err(|e| e.to_string())?.map_or(0, |f| f.size.max(0) as u64))
    }

    fn chunk(&self, name: &str, offset: u64, len: u64) -> Result<Vec<u8>, String> {
        self.fs.read_bytes_db(self.zone, name, offset as i64, len as i64).map_err(|e| e.to_string())
    }

    fn append(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        use crate::backend::storage::error::StoreError;
        use crate::backend::storage::filestore::FileOpts;
        let put = || self.fs.append_data(self.zone, name, bytes);
        match put() {
            Err(StoreError::NotFound) => match self.fs.make_file(self.zone, name, Default::default(), FileOpts::default()) {
                Ok(()) | Err(StoreError::AlreadyExists) => put().map_err(|e| e.to_string()),
                Err(e) => Err(e.to_string()),
            },
            other => other.map_err(|e| e.to_string()),
        }
    }

    fn delete(&self, name: &str) -> Result<(), String> {
        match self.fs.delete_file(self.zone, name) {
            Ok(()) | Err(crate::backend::storage::error::StoreError::NotFound) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// The store's items for a session, read from each zone's `output` in turn (the oldest conversation first, so
/// the order the proof matches in is the order the conversation was written in).
pub fn index_store(zones: &[&dyn Zone], sid: &str) -> Result<StoreIndex, String> {
    let mut index = StoreIndex::new();
    for zone in zones {
        index_zone(*zone, sid, &mut index)?;
    }
    Ok(index)
}

/// Add one zone's `output`, read a chunk at a time.
fn index_zone(zone: &dyn Zone, sid: &str, index: &mut StoreIndex) -> Result<(), String> {
    let size = zone.size(OUTPUT_FILE)?;
    let mut carry: Vec<u8> = Vec::new();
    let mut at = 0;
    while at < size {
        let part = zone.chunk(OUTPUT_FILE, at, CHUNK.min(size - at))?;
        if part.is_empty() {
            return Err("short read of output".to_string());
        }
        at += part.len() as u64;
        carry.extend_from_slice(&part);
        let mut start = 0;
        while let Some(i) = carry[start..].iter().position(|b| *b == b'\n') {
            index.add_line(sid, &String::from_utf8_lossy(&carry[start..start + i]));
            start += i + 1;
        }
        carry.drain(..start);
    }
    if !carry.is_empty() {
        index.add_line(sid, &String::from_utf8_lossy(&carry));
    }
    Ok(())
}

/// When anything the session holds was last written, as ms since the epoch: the newest of its session file and every
/// file under its folder. `None` when it has no file.
pub fn newest_modified_ms(files: &SessionFiles) -> Option<i64> {
    let stamp = |p: &Path| -> Option<i64> {
        let t = std::fs::metadata(p).ok()?.modified().ok()?;
        Some(t.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as i64)
    };
    members(files).ok()?.iter().filter_map(|m| stamp(&m.path)).max()
}

/// A session id that names a file safely: letters, digits, `-` and `_`, and no longer than a path part needs.
fn valid_sid(sid: &str) -> bool {
    (8..=80).contains(&sid.len()) && sid.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// One session's files under a project folder: `<sid>.jsonl` and the `<sid>/` folder beside it.
pub struct SessionFiles {
    pub sid: String,
    pub jsonl: PathBuf,
    pub folder: PathBuf,
}

impl SessionFiles {
    /// `None` when `sid` is not a plain id: nothing is ever named from a string with a separator in it.
    pub fn at(project_dir: &Path, sid: &str) -> Option<Self> {
        valid_sid(sid).then(|| Self { sid: sid.to_string(), jsonl: project_dir.join(format!("{sid}.jsonl")), folder: project_dir.join(sid) })
    }
}

/// A regular file of a session: where it is and its name inside the session (`/`-separated).
struct Member {
    rel: String,
    path: PathBuf,
}

/// Every file of the session, the session file first. A link or anything that is not a regular file or folder is an
/// error, and so is a missing session: the caller removes nothing.
fn members(files: &SessionFiles) -> Result<Vec<Member>, String> {
    let mut out = Vec::new();
    match std::fs::symlink_metadata(&files.jsonl) {
        Ok(m) if m.is_file() => out.push(Member { rel: format!("{}.jsonl", files.sid), path: files.jsonl.clone() }),
        Ok(_) => return Err("the session file is not a regular file".to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("the session file: {e}")),
    }
    match std::fs::symlink_metadata(&files.folder) {
        Ok(m) if m.is_dir() => walk(&files.folder, "", &mut out)?,
        Ok(_) => return Err("the session folder is not a folder".to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("the session folder: {e}")),
    }
    Ok(out)
}

fn walk(dir: &Path, prefix: &str, out: &mut Vec<Member>) -> Result<(), String> {
    let mut entries: Vec<_> = std::fs::read_dir(dir).map_err(|e| e.to_string())?.collect::<Result<_, _>>().map_err(|e: std::io::Error| e.to_string())?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().into_owned();
        let rel = if prefix.is_empty() { format!("session/{name}") } else { format!("{prefix}/{name}") };
        let m = std::fs::symlink_metadata(e.path()).map_err(|e| e.to_string())?;
        if m.is_dir() {
            walk(&e.path(), &rel, out)?;
        } else if m.is_file() {
            out.push(Member { rel, path: e.path() });
        } else {
            return Err("the session folder holds a link or a special file".to_string());
        }
    }
    Ok(())
}

/// What the proof found in one file.
struct FileProof {
    rel: String,
    path: PathBuf,
    records: usize,
    matched: usize,
    /// The lines to import, verbatim (a line-record file only).
    unmatched: Vec<String>,
    /// For matched records: (line number, fields).
    sidecar: Vec<(usize, Value)>,
    /// The file is not UTF-8 line records: it is imported whole.
    whole: bool,
    /// What an import of the whole file writes, as size and digest (masked as the store masks).
    want: Option<Sum>,
}

type Sum = (u64, [u8; 32]);

fn prove_lines(m: &Member, index: &StoreIndex) -> Result<FileProof, String> {
    let mut proof = FileProof { rel: m.rel.clone(), path: m.path.clone(), records: 0, matched: 0, unmatched: Vec::new(), sidecar: Vec::new(), whole: false, want: None };
    let mut reader = BufReader::new(std::fs::File::open(&m.path).map_err(|e| e.to_string())?);
    let mut matcher = Matcher::new(index);
    let mut buf = Vec::new();
    let mut n = 0;
    loop {
        buf.clear();
        if reader.read_until(b'\n', &mut buf).map_err(|e| e.to_string())? == 0 {
            break;
        }
        n += 1;
        while matches!(buf.last(), Some(b'\n' | b'\r')) {
            buf.pop();
        }
        if buf.is_empty() {
            continue;
        }
        let Ok(text) = std::str::from_utf8(&buf) else {
            return whole_proof(m);
        };
        proof.records += 1;
        let r: Record = record(n, text);
        if matcher.matches(&r) {
            proof.matched += 1;
            if let Some(f) = sidecar_fields(&r) {
                proof.sidecar.push((n, Value::Object(f)));
            }
        } else {
            proof.unmatched.push(r.line);
        }
    }
    Ok(proof)
}

fn is_lines_file(m: &Member) -> bool {
    m.path.extension().is_some_and(|e| e == "jsonl")
}

fn import_name(sid: &str, rel: &str) -> String {
    format!("import/{sid}/{rel}")
}

fn lines_set(bytes: &[u8]) -> HashSet<String> {
    String::from_utf8_lossy(bytes).split('\n').filter(|l| !l.is_empty()).map(str::to_string).collect()
}

/// A file imported whole is read into memory, so a file past this is not imported (and its session not removed).
const MAX_WHOLE_BYTES: u64 = 256 << 20;

/// What an import of a whole file writes: the bytes with a value srv holds secret replaced by its mark. A file that is
/// not UTF-8 cannot be masked, so one that holds a secret in any spelling is refused rather than written in clear.
fn imported_bytes(path: &Path) -> Result<Vec<u8>, String> {
    if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > MAX_WHOLE_BYTES {
        return Err("a file of the session is too large to import whole".to_string());
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    match String::from_utf8(bytes) {
        Ok(text) => Ok(withhold::text(&text).into_owned().into_bytes()),
        Err(e) => {
            let bytes = e.into_bytes();
            if withhold::holds(&String::from_utf8_lossy(&bytes)) {
                return Err("a file that is not text holds a value that is withheld".to_string());
            }
            Ok(bytes)
        }
    }
}

fn bytes_digest(bytes: &[u8]) -> Sum {
    (bytes.len() as u64, Sha256::digest(bytes).into())
}

fn whole_proof(m: &Member) -> Result<FileProof, String> {
    let want = bytes_digest(&imported_bytes(&m.path)?);
    Ok(FileProof { rel: m.rel.clone(), path: m.path.clone(), records: 0, matched: 0, unmatched: Vec::new(), sidecar: Vec::new(), whole: true, want: Some(want) })
}

fn file_digest(path: &Path) -> Result<Sum, String> {
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut total = 0u64;
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok((total, h.finalize().into()));
        }
        total += n as u64;
        h.update(&buf[..n]);
    }
}

fn zone_digest(zone: &dyn Zone, name: &str) -> Result<Sum, String> {
    let size = zone.size(name)?;
    let mut h = Sha256::new();
    let mut at = 0;
    while at < size {
        let part = zone.chunk(name, at, CHUNK.min(size - at))?;
        if part.is_empty() {
            return Err(format!("short read of {name}"));
        }
        at += part.len() as u64;
        h.update(&part);
    }
    Ok((size, h.finalize().into()))
}

/// Write a file's bytes whole under `name`, replacing what is there.
fn import_whole(zone: &dyn Zone, name: &str, path: &Path) -> Result<(), String> {
    let bytes = imported_bytes(path)?;
    zone.delete(name)?;
    for part in bytes.chunks(CHUNK as usize) {
        zone.append(name, part)?;
    }
    Ok(())
}

/// Append the lines of `lines` that the zone's file does not already hold.
fn import_lines(zone: &dyn Zone, name: &str, lines: &[String]) -> Result<(), String> {
    let held = lines_set(&zone.read_all(name)?);
    let mut out = String::new();
    for l in lines.iter().filter(|l| !held.contains(*l)) {
        out.push_str(l);
        out.push('\n');
    }
    zone.append(name, out.as_bytes())
}

fn sidecar_line(sid: &str, rel: &str, n: usize, fields: &Value) -> String {
    json!({ "s": sid, "f": rel, "n": n, "fields": fields }).to_string()
}

/// While secret matching is closed every line reads as the mark: nothing may be proved or imported then.
fn require_open_matching(mode: &str) -> Result<(), String> {
    if mode == "closed" {
        return Err("secret matching is closed: no provider file is read".to_string());
    }
    Ok(())
}

/// What the caller holds across the removal (an agent's lease): the removal asks before each file whether it is
/// still held, and stops when it is not.
pub trait Hold {
    fn still_held(&self) -> Result<(), String>;
}

impl Hold for () {
    fn still_held(&self) -> Result<(), String> {
        Ok(())
    }
}

/// What the end of a session did, in counts.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub files: usize,
    pub records: usize,
    pub matched: usize,
    pub imported_records: usize,
    pub imported_files: usize,
    pub removed_files: usize,
}

/// Prove, import, verify and remove one session's files. `before_remove` runs after the verify and before any file is
/// touched: an `Err` stops with nothing removed, and what it returns is kept until the removal is done and asked before
/// each file whether it is still held. `erase` is called for each file after it is overwritten and
/// before it is unlinked (a test's hook; pass a no-op otherwise). Nothing is removed unless every record and file is
/// matched or read back equal; an `Err` has removed nothing.
pub fn end_session(
    files: &SessionFiles,
    read: &[&dyn Zone],
    zone: &dyn Zone,
    before_remove: &mut dyn FnMut() -> Result<Box<dyn Hold>, String>,
    erase: &mut dyn FnMut(&Path, u64),
) -> Result<Outcome, String> {
    require_open_matching(&withhold::health().mode)?;
    let members = members(files)?;
    let index = index_store(read, &files.sid)?;
    // What each file is before the proof reads it: removal checks it is still that.
    let mut raw = Vec::new();
    for m in &members {
        raw.push((m.path.clone(), file_digest(&m.path)?));
    }
    let mut proofs = Vec::new();
    for m in &members {
        proofs.push(if is_lines_file(m) { prove_lines(m, &index)? } else { whole_proof(m)? });
    }
    import(files, zone, &proofs)?;
    verify(files, zone, &proofs)?;
    // The caller's last word, held until the files are gone: it may refuse (nothing is removed) or return what it
    // holds across the removal.
    let held = before_remove()?;
    let removed = remove_proved(files, &raw, &*held, erase)?;
    let mut out = Outcome { files: members.len(), removed_files: removed, ..Default::default() };
    for p in &proofs {
        out.records += p.records;
        out.matched += p.matched;
        out.imported_records += p.unmatched.len();
        out.imported_files += usize::from(p.whole);
    }
    Ok(out)
}

fn import(files: &SessionFiles, zone: &dyn Zone, proofs: &[FileProof]) -> Result<(), String> {
    let mut sidecar = Vec::new();
    for p in proofs {
        let name = import_name(&files.sid, &p.rel);
        if p.whole {
            import_whole(zone, &name, &p.path)?;
        } else if !p.unmatched.is_empty() {
            import_lines(zone, &name, &p.unmatched)?;
        }
        sidecar.extend(p.sidecar.iter().map(|(n, f)| sidecar_line(&files.sid, &p.rel, *n, f)));
    }
    if sidecar.is_empty() {
        return Ok(());
    }
    let held = lines_set(&zone.read_all(SIDECAR_FILE)?);
    let fresh: Vec<&String> = sidecar.iter().filter(|l| !held.contains(*l)).collect();
    let mut out = String::new();
    for l in fresh {
        out.push_str(l);
        out.push('\n');
    }
    zone.append(SIDECAR_FILE, out.as_bytes())
}

/// The second look: what was imported reads back equal to what the files hold.
fn verify(files: &SessionFiles, zone: &dyn Zone, proofs: &[FileProof]) -> Result<(), String> {
    let held_sidecar = lines_set(&zone.read_all(SIDECAR_FILE)?);
    for p in proofs {
        let name = import_name(&files.sid, &p.rel);
        if p.whole {
            if p.want != Some(zone_digest(zone, &name)?) {
                return Err("an imported file does not read back equal".to_string());
            }
        } else if !p.unmatched.is_empty() {
            let held = lines_set(&zone.read_all(&name)?);
            if p.unmatched.iter().any(|l| !held.contains(l)) {
                return Err("an imported record does not read back".to_string());
            }
        }
        if p.sidecar.iter().any(|(n, f)| !held_sidecar.contains(&sidecar_line(&files.sid, &p.rel, *n, f))) {
            return Err("a sidecar line does not read back".to_string());
        }
    }
    Ok(())
}

/// Overwrite a file with zeros, sync it, let `hook` see it, and unlink it. The file is opened once, and what it holds
/// must still be what the proof read (`expect`): a file that grew or changed, or a link put in its place, is refused.
fn erase_file(path: &Path, expect: Sum, hook: &mut dyn FnMut(&Path, u64)) -> Result<(), String> {
    use std::io::{Seek, SeekFrom};
    if !std::fs::symlink_metadata(path).map_err(|e| e.to_string())?.is_file() {
        return Err("a file of the session changed into something that is not a file".to_string());
    }
    let mut f = std::fs::OpenOptions::new().read(true).write(true).open(path).map_err(|e| e.to_string())?;
    let mut h = Sha256::new();
    let mut size = 0u64;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        size += n as u64;
        h.update(&buf[..n]);
    }
    if (size, <[u8; 32]>::from(h.finalize())) != expect {
        return Err("a file of the session changed after it was proved".to_string());
    }
    f.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let zeros = vec![0u8; 64 << 10];
    let mut left = size;
    while left > 0 {
        let n = left.min(zeros.len() as u64) as usize;
        f.write_all(&zeros[..n]).map_err(|e| e.to_string())?;
        left -= n as u64;
    }
    f.sync_all().map_err(|e| e.to_string())?;
    drop(f);
    hook(path, size);
    std::fs::remove_file(path).map_err(|e| e.to_string())
}

/// Remove the files the proof read, by session id: each overwritten with zeros and then unlinked, then the session's
/// folders. It names only `<sid>.jsonl` and `<sid>/` under the project folder and follows no link. A file that was
/// added, is missing, or no longer holds what the proof read stops it **before any file is touched**. Returns how
/// many files it removed.
fn remove_proved(files: &SessionFiles, proved: &[(PathBuf, Sum)], held: &dyn Hold, erase: &mut dyn FnMut(&Path, u64)) -> Result<usize, String> {
    held.still_held()?;
    let now = members(files)?;
    let same = now.len() == proved.len() && now.iter().zip(proved).all(|(m, (p, _))| &m.path == p);
    if !same {
        return Err("the files of the session changed after they were proved".to_string());
    }
    for (path, digest) in proved {
        if file_digest(path)? != *digest {
            return Err("a file of the session changed after it was proved".to_string());
        }
    }
    for (path, digest) in proved {
        held.still_held()?;
        erase_file(path, *digest, erase)?;
    }
    remove_dirs(&files.folder);
    Ok(proved.len())
}

/// Remove the folder and its empty subfolders, deepest first; one that is not empty stays.
fn remove_dirs(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        if std::fs::symlink_metadata(e.path()).is_ok_and(|m| m.is_dir()) {
            remove_dirs(&e.path());
        }
    }
    let _ = std::fs::remove_dir(dir);
}

#[cfg(test)]
#[path = "provider_tests.rs"]
mod tests;
