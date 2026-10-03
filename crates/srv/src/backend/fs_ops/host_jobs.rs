// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Copy and move between this computer and an SSH host, or within one host
//! (spec §6.3 of SPEC_REMOTE_TERMINALS_AND_DURABLE_SESSIONS_2026_10_02.md):
//! the same `fs.op` as a local copy, with the same progress, cancel and
//! conflict questions (`jobs::Jobs::start_host`), the host's side carried by
//! its helper (`remote::files`).
//!
//! - A file goes to a temp file beside its destination, in pieces (ranged
//!   reads, appends), and is renamed into place only when complete, never
//!   over anything the user didn't choose to replace.
//! - A folder onto a folder merges; anything else already there is asked
//!   about (replace, skip, keep both).
//! - Links aren't copied between two machines: where one points means
//!   something only on its own side.
//! - A move removes each source only once it landed; within one host it is a
//!   rename on that host.
//! - Nothing in a protected place is changed: this computer's rules
//!   (`ProtectedPaths`), a host's system folders, root and home folder
//!   (`remote::protected`).

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::runtime::Handle;

use super::jobs::{EntrySide, HostJob, HostStop, JOBS};
use super::platform::{self, display_path};
use super::ProtectedPaths;
use crate::backend::remote::files::{RemoteError, RemoteFiles};
use crate::backend::rpc_types::{FsOpChoice, FsOpKind, FsOpStartResult};

/// One side of a transfer.
#[derive(Clone)]
pub enum End {
    /// This computer.
    Local,
    /// An SSH host, through its helper; `home` in tidy form.
    Host {
        files: Arc<RemoteFiles>,
        home: String,
    },
}

/// What a path is, on either side.
#[derive(Debug, Clone, Copy)]
struct Info {
    is_dir: bool,
    is_link: bool,
    size: u64,
    mtime: Option<u64>,
}

impl Info {
    fn side(&self) -> EntrySide {
        EntrySide {
            is_dir: self.is_dir,
            size: (!self.is_dir).then_some(self.size),
            mtime: self.mtime,
        }
    }
}

/// A rename that found its target taken.
enum RenameError {
    Taken,
    Other(String),
}

fn host_error(e: RemoteError) -> String {
    e.message
}

impl End {
    pub fn host(files: Arc<RemoteFiles>) -> Self {
        let home = super::remote::resolve("/", &files.home);
        End::Host { files, home }
    }

    /// Both are the same host's helper.
    fn same_host(&self, other: &End) -> bool {
        match (self, other) {
            (End::Host { files: a, .. }, End::Host { files: b, .. }) => a.conn == b.conn,
            _ => false,
        }
    }

    fn join(&self, dir: &str, name: &str) -> String {
        match self {
            End::Local => display_path(&Path::new(dir).join(name)),
            End::Host { .. } => posix_join(dir, name),
        }
    }

    fn parent(&self, path: &str) -> String {
        match self {
            End::Local => Path::new(path)
                .parent()
                .map(display_path)
                .unwrap_or_default(),
            End::Host { .. } => posix_parent(path),
        }
    }

    /// Why `path` may not be changed, if it may not.
    fn refusal(&self, path: &str) -> Option<String> {
        match self {
            End::Local => ProtectedPaths::current().check(Path::new(path)).err(),
            End::Host { files, home } => super::remote::protected(home, path).then(|| {
                format!("AgentMux doesn't change {path} on {}: it's a system folder or your home folder.", files.conn)
            }),
        }
    }

    fn stat(&self, rt: &Handle, path: &str) -> Result<Option<Info>, String> {
        match self {
            End::Local => match std::fs::symlink_metadata(path) {
                Ok(m) => Ok(Some(Info {
                    is_dir: m.is_dir(),
                    is_link: m.file_type().is_symlink(),
                    size: m.len(),
                    mtime: m
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_millis() as u64),
                })),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e.to_string()),
            },
            End::Host { files, .. } => match rt.block_on(files.stat(path)) {
                Ok(e) => Ok(Some(Info {
                    is_dir: e.kind == agentmux_remote::fsproto::Kind::Dir && !e.symlink,
                    is_link: e.symlink,
                    size: e.size,
                    mtime: (e.mtime_ms > 0).then_some(e.mtime_ms as u64),
                })),
                Err(e) if e.is_not_found() => Ok(None),
                Err(e) => Err(host_error(e)),
            },
        }
    }

    fn list(&self, rt: &Handle, dir: &str) -> Result<Vec<String>, String> {
        match self {
            End::Local => {
                let mut names: Vec<String> = std::fs::read_dir(dir)
                    .map_err(|e| e.to_string())?
                    .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
                    .collect();
                names.sort();
                Ok(names)
            }
            End::Host { files, .. } => Ok(rt
                .block_on(files.list(dir))
                .map_err(host_error)?
                .into_iter()
                .map(|e| e.name)
                .collect()),
        }
    }

    fn mkdir(&self, rt: &Handle, path: &str) -> Result<(), String> {
        match self {
            End::Local => std::fs::create_dir(path).map_err(|e| e.to_string()),
            End::Host { files, .. } => rt.block_on(files.mkdir(path, false)).map_err(host_error),
        }
    }

    /// Up to `len` bytes of a file from `offset`, and whether it ends there.
    fn read(
        &self,
        rt: &Handle,
        path: &str,
        offset: u64,
        len: usize,
    ) -> Result<(Vec<u8>, bool), String> {
        match self {
            End::Local => {
                let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
                f.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
                let mut data = Vec::with_capacity(len);
                f.take(len as u64)
                    .read_to_end(&mut data)
                    .map_err(|e| e.to_string())?;
                let eof = data.len() < len;
                Ok((data, eof))
            }
            End::Host { files, .. } => rt
                .block_on(files.read_range(path, offset, len as u32))
                .map_err(host_error),
        }
    }

    /// A new, empty temp file in `dir`, for a piece-by-piece copy.
    fn create_temp(&self, rt: &Handle, dir: &str) -> Result<String, String> {
        let name = format!(".agentmux-part-{}", uuid::Uuid::new_v4().simple());
        let path = self.join(dir, &name);
        match self {
            End::Local => {
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .map_err(|e| e.to_string())?;
            }
            End::Host { files, .. } => rt
                .block_on(files.write(&path, Vec::new()))
                .map_err(host_error)?,
        }
        Ok(path)
    }

    fn append(&self, rt: &Handle, path: &str, data: Vec<u8>) -> Result<(), String> {
        match self {
            End::Local => std::fs::OpenOptions::new()
                .append(true)
                .open(path)
                .and_then(|mut f| f.write_all(&data))
                .map_err(|e| e.to_string()),
            End::Host { files, .. } => rt.block_on(files.append(path, data)).map_err(host_error),
        }
    }

    /// Rename, never over anything.
    fn rename_no_replace(&self, rt: &Handle, from: &str, to: &str) -> Result<(), RenameError> {
        match self {
            End::Local => {
                platform::rename_no_replace(Path::new(from), Path::new(to)).map_err(|e| {
                    if e.kind() == std::io::ErrorKind::AlreadyExists {
                        RenameError::Taken
                    } else {
                        RenameError::Other(e.to_string())
                    }
                })
            }
            End::Host { files, .. } => rt.block_on(files.rename(from, to)).map_err(|e| {
                if e.kind == Some(agentmux_remote::fsproto::ErrKind::AlreadyExists) {
                    RenameError::Taken
                } else {
                    RenameError::Other(e.message)
                }
            }),
        }
    }

    /// Remove a file, a link, or an empty folder.
    fn remove(&self, rt: &Handle, path: &str) -> Result<(), String> {
        match self {
            End::Local => {
                platform::remove_entry_no_follow(Path::new(path)).map_err(|e| e.to_string())
            }
            End::Host { files, .. } => rt.block_on(files.delete(path, false)).map_err(host_error),
        }
    }

    /// The display form of `path` on this side, naming the host.
    fn show(&self, path: &str) -> String {
        match self {
            End::Local => path.to_string(),
            End::Host { files, .. } => format!("{}:{path}", files.conn),
        }
    }

    /// `path` as this side spells it: a host's resolved under its home.
    fn tidy(&self, path: &str) -> String {
        match self {
            End::Local => display_path(&PathBuf::from(path)),
            End::Host { home, .. } => super::remote::resolve(home, path),
        }
    }
}

/// `name` in `dir`, a host's POSIX path.
fn posix_join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// The folder a host's POSIX path is in; `/` at the top.
fn posix_parent(path: &str) -> String {
    match path.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(i) => path[..i].to_string(),
    }
}

fn file_name(path: &str) -> String {
    path.rsplit(['/', '\\'])
        .find(|s| !s.is_empty())
        .unwrap_or(path)
        .to_string()
}

/// `name (n).ext` for the first free n.
fn keep_both_name(end: &End, rt: &Handle, dir: &str, name: &str) -> Result<String, String> {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for n in 2..10_000 {
        let candidate = end.join(dir, &format!("{stem} ({n}){ext}"));
        if end.stat(rt, &candidate)?.is_none() {
            return Ok(candidate);
        }
    }
    Err(format!("No free name for a copy of “{name}”."))
}

/// What became of one entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Done,
    NotDone,
    Stopped,
}

struct Transfer<'r> {
    rt: &'r Handle,
    kind: FsOpKind,
    src: End,
    dest: End,
}

impl Transfer<'_> {
    /// Items and file bytes under `path` on the source.
    fn measure(&self, path: &str) -> (u64, u64) {
        let mut stack = vec![path.to_string()];
        let (mut items, mut bytes) = (0u64, 0u64);
        while let Some(p) = stack.pop() {
            let Ok(Some(info)) = self.src.stat(self.rt, &p) else {
                continue;
            };
            items += 1;
            if info.is_dir {
                for name in self.src.list(self.rt, &p).unwrap_or_default() {
                    stack.push(self.src.join(&p, &name));
                }
            } else if !info.is_link {
                bytes += info.size;
            }
        }
        (items, bytes)
    }

    fn pass_over(&self, job: &mut HostJob<'_, '_>, path: &str) {
        let (items, bytes) = self.measure(path);
        job.advance(items, bytes);
    }

    fn entry(&self, job: &mut HostJob<'_, '_>, src: &str, dest: &str) -> Outcome {
        if job.is_canceled() {
            return Outcome::Stopped;
        }
        let shown = self.src.show(src);
        job.set_current(&shown);
        let info = match self.src.stat(self.rt, src) {
            Ok(Some(i)) => i,
            Ok(None) => {
                job.fail(&shown, format!("“{}” isn't there anymore.", file_name(src)));
                return Outcome::NotDone;
            }
            Err(e) => {
                job.fail(&shown, e);
                return Outcome::NotDone;
            }
        };
        if info.is_link {
            job.fail(
                &shown,
                format!(
                    "“{}” is a link: links aren't copied between two machines.",
                    file_name(src)
                ),
            );
            job.advance(1, 0);
            return Outcome::NotDone;
        }
        if let Some(why) = self.dest.refusal(dest) {
            job.fail(&shown, why);
            self.pass_over(job, src);
            return Outcome::NotDone;
        }
        if self.kind == FsOpKind::Move {
            if let Some(why) = self.src.refusal(src) {
                job.fail(&shown, why);
                self.pass_over(job, src);
                return Outcome::NotDone;
            }
        }
        let existing = match self.dest.stat(self.rt, dest) {
            Ok(e) => e,
            Err(e) => {
                job.fail(&shown, e);
                self.pass_over(job, src);
                return Outcome::NotDone;
            }
        };
        let mut dest = dest.to_string();
        let mut replace = false;
        if let Some(there) = existing {
            if info.is_dir && there.is_dir {
                return self.merge(job, src, &dest);
            }
            match job.ask(&shown, info.side(), &self.dest.show(&dest), there.side()) {
                None => return Outcome::Stopped,
                Some(FsOpChoice::Skip) => {
                    self.pass_over(job, src);
                    return Outcome::NotDone;
                }
                Some(FsOpChoice::KeepBoth) => {
                    let dir = self.dest.parent(&dest);
                    match keep_both_name(&self.dest, self.rt, &dir, &file_name(&dest)) {
                        Ok(d) => dest = d,
                        Err(e) => {
                            job.fail(&shown, e);
                            self.pass_over(job, src);
                            return Outcome::NotDone;
                        }
                    }
                }
                Some(FsOpChoice::Replace) => {
                    if info.is_dir || there.is_dir {
                        job.fail(
                            &shown,
                            format!(
                                "“{}” and what's there aren't both files: not replaced.",
                                file_name(src)
                            ),
                        );
                        self.pass_over(job, src);
                        return Outcome::NotDone;
                    }
                    replace = true;
                }
            }
        }
        // Within one host, a move is a rename there.
        if self.kind == FsOpKind::Move && !replace && self.src.same_host(&self.dest) {
            match self.src.rename_no_replace(self.rt, src, &dest) {
                Ok(()) => {
                    self.pass_over_after_rename(job, &dest);
                    return Outcome::Done;
                }
                Err(RenameError::Taken) => {
                    job.fail(
                        &shown,
                        format!("Something named “{}” appeared there.", file_name(&dest)),
                    );
                    self.pass_over(job, src);
                    return Outcome::NotDone;
                }
                Err(RenameError::Other(_)) => {}
            }
        }
        if info.is_dir {
            if let Err(e) = self.dest.mkdir(self.rt, &dest) {
                job.fail(&shown, e);
                self.pass_over(job, src);
                return Outcome::NotDone;
            }
            job.advance(1, 0);
            let out = self.children(job, src, &dest);
            if out == Outcome::Done && self.kind == FsOpKind::Move {
                if let Err(e) = self.src.remove(self.rt, src) {
                    job.fail(&shown, e);
                    return Outcome::NotDone;
                }
            }
            return out;
        }
        self.file(job, src, &dest, &shown, info.size, replace)
    }

    /// After a rename within one host, count what moved as done.
    fn pass_over_after_rename(&self, job: &mut HostJob<'_, '_>, moved_to: &str) {
        let probe = Transfer {
            rt: self.rt,
            kind: self.kind,
            src: self.dest.clone(),
            dest: self.dest.clone(),
        };
        probe.pass_over(job, moved_to);
    }

    fn merge(&self, job: &mut HostJob<'_, '_>, src: &str, dest: &str) -> Outcome {
        job.advance(1, 0);
        let out = self.children(job, src, dest);
        if out == Outcome::Done && self.kind == FsOpKind::Move {
            if let Err(e) = self.src.remove(self.rt, src) {
                job.fail(&self.src.show(src), e);
                return Outcome::NotDone;
            }
        }
        out
    }

    fn children(&self, job: &mut HostJob<'_, '_>, src: &str, dest: &str) -> Outcome {
        let names = match self.src.list(self.rt, src) {
            Ok(n) => n,
            Err(e) => {
                job.fail(&self.src.show(src), e);
                return Outcome::NotDone;
            }
        };
        let mut all = Outcome::Done;
        for name in names {
            match self.entry(
                job,
                &self.src.join(src, &name),
                &self.dest.join(dest, &name),
            ) {
                Outcome::Stopped => return Outcome::Stopped,
                Outcome::NotDone => all = Outcome::NotDone,
                Outcome::Done => {}
            }
        }
        all
    }

    fn file(
        &self,
        job: &mut HostJob<'_, '_>,
        src: &str,
        dest: &str,
        shown: &str,
        size: u64,
        replace: bool,
    ) -> Outcome {
        let dir = self.dest.parent(dest);
        let temp = match self.dest.create_temp(self.rt, &dir) {
            Ok(t) => t,
            Err(e) => {
                job.fail(shown, e);
                job.advance(1, size);
                return Outcome::NotDone;
            }
        };
        let discard = |t: &Transfer<'_>| {
            let _ = t.dest.remove(t.rt, &temp);
        };
        // Pieces: the job's chunk, within what one message may carry.
        let chunk = job.chunk_size().clamp(64 * 1024, 4 << 20);
        let mut offset = 0u64;
        loop {
            if job.is_canceled() {
                discard(self);
                return Outcome::Stopped;
            }
            let (data, eof) = match self.src.read(self.rt, src, offset, chunk) {
                Ok(r) => r,
                Err(e) => {
                    discard(self);
                    job.fail(shown, e);
                    job.advance(1, size.saturating_sub(offset));
                    return Outcome::NotDone;
                }
            };
            let n = data.len() as u64;
            if n > 0 {
                if let Err(e) = self.dest.append(self.rt, &temp, data) {
                    discard(self);
                    job.fail(shown, e);
                    job.advance(1, size.saturating_sub(offset));
                    return Outcome::NotDone;
                }
                offset += n;
                job.advance(0, n);
            }
            if eof || n == 0 {
                break;
            }
        }
        if replace {
            if let Err(e) = self.dest.remove(self.rt, dest) {
                discard(self);
                job.fail(shown, e);
                job.advance(1, 0);
                return Outcome::NotDone;
            }
        }
        match self.dest.rename_no_replace(self.rt, &temp, dest) {
            Ok(()) => {}
            Err(RenameError::Taken) => {
                discard(self);
                job.fail(
                    shown,
                    format!(
                        "Something named “{}” appeared there meanwhile; nothing was replaced.",
                        file_name(dest)
                    ),
                );
                job.advance(1, 0);
                return Outcome::NotDone;
            }
            Err(RenameError::Other(e)) => {
                discard(self);
                job.fail(shown, e);
                job.advance(1, 0);
                return Outcome::NotDone;
            }
        }
        job.advance(1, 0);
        if self.kind == FsOpKind::Move {
            if let Err(e) = self.src.remove(self.rt, src) {
                job.fail(
                    shown,
                    format!("Copied, but the original couldn't be removed: {e}"),
                );
                return Outcome::NotDone;
            }
        }
        Outcome::Done
    }
}

/// One source to transfer, as planned.
struct Item {
    src: String,
    /// Copied into the folder it is already in: kept as `name (n)`.
    duplicate: bool,
}

fn under(path: &str, dir: &str) -> bool {
    path.strip_prefix(dir)
        .is_some_and(|rest| rest.starts_with('/'))
        || (dir == "/" && path.starts_with('/') && path != "/")
}

/// The local plan's rules (`jobs::plan`) where source and destination are
/// one host's folders: a folder never into itself or a folder of its own; a
/// move into the folder it is in is nothing to do, a copy there is a
/// duplicate; and no destination lands on a folder holding another source.
/// Between two machines none of this can happen.
fn plan_on_one_host(
    kind: FsOpKind,
    src: &End,
    sources: &[String],
    dest: &End,
    dest_dir: &str,
) -> Result<Vec<Item>, String> {
    if !src.same_host(dest) {
        return Ok(sources
            .iter()
            .map(|s| Item {
                src: s.clone(),
                duplicate: false,
            })
            .collect());
    }
    let verb = if kind == FsOpKind::Move {
        "move"
    } else {
        "copy"
    };
    let mut items = Vec::new();
    for s in sources {
        if dest_dir == s || under(dest_dir, s) {
            return Err(format!("Can't {verb} a folder into itself."));
        }
        let in_dest_already = src.parent(s) == dest_dir;
        if in_dest_already && kind == FsOpKind::Move {
            continue;
        }
        items.push(Item {
            src: s.clone(),
            duplicate: in_dest_already,
        });
    }
    // E.g. copying `/a/d/d` into `/a` would merge into `/a/d`, which holds
    // the source itself.
    let landing: std::collections::HashSet<String> = items
        .iter()
        .filter(|i| !i.duplicate)
        .map(|i| file_name(&i.src))
        .collect();
    for item in &items {
        let rest = if dest_dir == "/" {
            item.src.strip_prefix('/')
        } else {
            item.src.strip_prefix(&format!("{dest_dir}/"))
        };
        let Some(rest) = rest else { continue };
        let mut parts = rest.split('/').filter(|p| !p.is_empty());
        if let (Some(first), Some(_)) = (parts.next(), parts.next()) {
            if landing.contains(first) {
                return Err(format!(
                    "Can't {verb} there: “{first}” would land on the folder that holds “{}”.",
                    file_name(&item.src)
                ));
            }
        }
    }
    Ok(items)
}

/// Start a copy or move of `sources` (on `src`) into `dest_dir` (on `dest`)
/// as an `fs.op` (`emit` carries its events). At least one side is a host.
pub async fn start(
    kind: FsOpKind,
    src: End,
    sources: Vec<String>,
    dest: End,
    dest_dir: String,
    emit: super::jobs::Emit,
) -> Result<FsOpStartResult, String> {
    let rt = Handle::current();
    let dest_dir = dest.tidy(&dest_dir);
    let sources: Vec<String> = sources.iter().map(|s| src.tidy(s)).collect();
    // Checked before answering, as a local copy's plan is (awaited here: only
    // the op's own thread blocks on the runtime).
    let is_dir = match &dest {
        End::Local => std::fs::metadata(&dest_dir)
            .map(|m| m.is_dir())
            .map_err(|e| e.kind()),
        End::Host { files, .. } => files
            .stat(&dest_dir)
            .await
            .map(|e| e.kind == agentmux_remote::fsproto::Kind::Dir)
            .map_err(|e| {
                if e.is_not_found() {
                    std::io::ErrorKind::NotFound
                } else {
                    std::io::ErrorKind::Other
                }
            }),
    };
    match is_dir {
        Ok(true) => {}
        Ok(false) => return Err("The destination isn't a folder.".to_string()),
        Err(std::io::ErrorKind::NotFound) => {
            return Err("The destination folder isn't there anymore.".to_string())
        }
        Err(_) => return Err("Couldn't reach the destination folder.".to_string()),
    }
    if sources.is_empty() {
        return Err("Nothing to copy.".to_string());
    }
    let items = plan_on_one_host(kind, &src, &sources, &dest, &dest_dir)?;
    let shown_dest = dest.show(&dest_dir);
    let requested = sources.len();
    JOBS.start_host(
        kind,
        requested,
        shown_dest,
        emit,
        Box::new(move |job| {
            let t = Transfer {
                rt: &rt,
                kind,
                src,
                dest,
            };
            let (mut total_items, mut total_bytes) = (0, 0);
            for item in &items {
                let (i, b) = t.measure(&item.src);
                total_items += i;
                total_bytes += b;
            }
            job.add_total(total_items, total_bytes);
            for item in &items {
                let name = file_name(&item.src);
                let target = if item.duplicate {
                    // Copied into the folder it is in: always beside itself,
                    // as `name (n)`, never a question about itself.
                    match keep_both_name(&t.dest, &rt, &dest_dir, &name) {
                        Ok(d) => d,
                        Err(e) => {
                            job.fail(&t.src.show(&item.src), e);
                            t.pass_over(job, &item.src);
                            continue;
                        }
                    }
                } else {
                    t.dest.join(&dest_dir, &name)
                };
                if t.entry(job, &item.src, &target) == Outcome::Stopped {
                    return Err(HostStop::Canceled);
                }
            }
            Ok(())
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::rpc_types::{FsOpEvent, FsOpEventState};
    use std::sync::Mutex;

    fn events() -> (super::super::jobs::Emit, Arc<Mutex<Vec<FsOpEvent>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        (
            Arc::new(move |e: &FsOpEvent| s.lock().unwrap().push(e.clone())),
            seen,
        )
    }

    async fn finished(seen: &Arc<Mutex<Vec<FsOpEvent>>>) -> FsOpEvent {
        for _ in 0..500 {
            if let Some(e) = seen
                .lock()
                .unwrap()
                .iter()
                .find(|e| {
                    matches!(
                        e.state,
                        FsOpEventState::Done | FsOpEventState::Failed | FsOpEventState::Canceled
                    )
                })
                .cloned()
            {
                return e;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("the op never finished");
    }

    async fn host(home: &Path) -> End {
        let (r, w) = crate::backend::remote::files::testing::helper(home.to_path_buf());
        End::host(
            RemoteFiles::over("testhost", r, w, Box::new(()))
                .await
                .unwrap(),
        )
    }

    /// Up to a host and back down, folders and all, in pieces bigger than
    /// one read; links stay behind with a reason.
    // Linux: a host's paths are POSIX, so the test home's must be.
    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn files_and_folders_go_up_to_a_host_and_back_down() {
        let local = tempfile::tempdir().unwrap();
        let remote = tempfile::tempdir().unwrap();
        let big: Vec<u8> = (0..(5 << 20) + 17).map(|i| (i % 251) as u8).collect();
        std::fs::create_dir_all(local.path().join("proj/src")).unwrap();
        std::fs::write(local.path().join("proj/src/big.bin"), &big).unwrap();
        std::fs::write(local.path().join("proj/readme.md"), b"hi").unwrap();
        let h = host(remote.path()).await;

        let (emit, seen) = events();
        let src = display_path(&local.path().join("proj"));
        let dest_dir = display_path(remote.path());
        start(
            FsOpKind::Copy,
            End::Local,
            vec![src],
            h.clone(),
            dest_dir,
            emit,
        )
        .await
        .unwrap();
        let end = finished(&seen).await;
        assert_eq!(end.state, FsOpEventState::Done, "{end:?}");
        assert_eq!(end.done_bytes, end.total_bytes);
        assert_eq!(
            std::fs::read(remote.path().join("proj/src/big.bin")).unwrap(),
            big
        );
        assert_eq!(
            std::fs::read(remote.path().join("proj/readme.md")).unwrap(),
            b"hi"
        );
        // No temp file left on the host.
        assert!(std::fs::read_dir(remote.path().join("proj/src"))
            .unwrap()
            .all(|e| !e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".agentmux-part")));

        // Back down, as a move: the host's copy goes once it landed.
        let down = tempfile::tempdir().unwrap();
        let (emit, seen) = events();
        let src = display_path(&remote.path().join("proj"));
        start(
            FsOpKind::Move,
            h,
            vec![src],
            End::Local,
            display_path(down.path()),
            emit,
        )
        .await
        .unwrap();
        let end = finished(&seen).await;
        assert_eq!(end.state, FsOpEventState::Done, "{end:?}");
        assert_eq!(
            std::fs::read(down.path().join("proj/src/big.bin")).unwrap(),
            big
        );
        assert!(!remote.path().join("proj").exists());
    }

    /// Something already at the destination is asked about; Skip leaves it,
    /// and Keep both puts the copy beside it.
    // Linux: a host's paths are POSIX, so the test home's must be.
    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_existing_file_is_asked_about_never_overwritten_silently() {
        let local = tempfile::tempdir().unwrap();
        let remote = tempfile::tempdir().unwrap();
        std::fs::write(local.path().join("a.txt"), b"new").unwrap();
        std::fs::write(remote.path().join("a.txt"), b"old").unwrap();
        let h = host(remote.path()).await;
        let (emit, seen) = events();
        let op = start(
            FsOpKind::Copy,
            End::Local,
            vec![display_path(&local.path().join("a.txt"))],
            h,
            display_path(remote.path()),
            emit,
        )
        .await
        .unwrap();
        // Wait for the question, then keep both.
        for _ in 0..500 {
            if seen
                .lock()
                .unwrap()
                .iter()
                .any(|e| e.state == FsOpEventState::Conflict)
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let asked = seen
            .lock()
            .unwrap()
            .iter()
            .find(|e| e.state == FsOpEventState::Conflict)
            .cloned()
            .unwrap();
        let c = asked.conflict.unwrap();
        assert_eq!((c.source_size, c.dest_size), (Some(3), Some(3)));
        assert!(c.dest.starts_with("testhost:"), "{}", c.dest);
        JOBS.resolve(&op.op_id, FsOpChoice::KeepBoth, false)
            .unwrap();
        let end = finished(&seen).await;
        assert_eq!(end.state, FsOpEventState::Done, "{end:?}");
        assert_eq!(std::fs::read(remote.path().join("a.txt")).unwrap(), b"old");
        assert_eq!(
            std::fs::read(remote.path().join("a (2).txt")).unwrap(),
            b"new"
        );
    }

    /// Within one host a move is a rename there; nothing comes through here.
    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_move_within_one_host_is_a_rename_there() {
        let remote = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(remote.path().join("a")).unwrap();
        std::fs::create_dir_all(remote.path().join("b")).unwrap();
        std::fs::write(remote.path().join("a/x.txt"), b"x").unwrap();
        let h = host(remote.path()).await;
        let (emit, seen) = events();
        start(
            FsOpKind::Move,
            h.clone(),
            vec![display_path(&remote.path().join("a/x.txt"))],
            h,
            display_path(&remote.path().join("b")),
            emit,
        )
        .await
        .unwrap();
        let end = finished(&seen).await;
        assert_eq!(end.state, FsOpEventState::Done, "{end:?}");
        assert_eq!(std::fs::read(remote.path().join("b/x.txt")).unwrap(), b"x");
        assert!(!remote.path().join("a/x.txt").exists());
    }

    /// Canceled mid-file: nothing half-written is left on the host.
    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_canceled_upload_leaves_nothing_behind() {
        let local = tempfile::tempdir().unwrap();
        let remote = tempfile::tempdir().unwrap();
        std::fs::write(local.path().join("huge.bin"), vec![1u8; 16 << 20]).unwrap();
        // A slow link: each piece takes a while, so the cancel lands halfway.
        let (r, w) = crate::backend::remote::files::testing::slow_helper(
            remote.path().to_path_buf(),
            std::time::Duration::from_millis(20),
        );
        let h = End::host(
            RemoteFiles::over("testhost", r, w, Box::new(()))
                .await
                .unwrap(),
        );
        let (emit, seen) = events();
        let op = start(
            FsOpKind::Copy,
            End::Local,
            vec![display_path(&local.path().join("huge.bin"))],
            h,
            display_path(remote.path()),
            emit,
        )
        .await
        .unwrap();
        for _ in 0..500 {
            if seen.lock().unwrap().iter().any(|e| e.done_bytes > 0) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        JOBS.cancel(&op.op_id);
        let end = finished(&seen).await;
        assert_eq!(end.state, FsOpEventState::Canceled, "{end:?}");
        let left: Vec<String> = std::fs::read_dir(remote.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(left.is_empty(), "{left:?}");
    }

    /// A host's system folders are never written, whatever is asked.
    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_protected_folder_on_the_host_is_refused() {
        let local = tempfile::tempdir().unwrap();
        let remote = tempfile::tempdir().unwrap();
        std::fs::write(local.path().join("a.txt"), b"a").unwrap();
        let h = host(remote.path()).await;
        let (emit, seen) = events();
        start(
            FsOpKind::Copy,
            End::Local,
            vec![display_path(&local.path().join("a.txt"))],
            h,
            "/etc".into(),
            emit,
        )
        .await
        .unwrap();
        let end = finished(&seen).await;
        let failures = end.failures.unwrap_or_default();
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert!(
            failures[0]
                .error
                .as_deref()
                .unwrap_or("")
                .contains("doesn't change"),
            "{failures:?}"
        );
        assert!(!std::path::Path::new("/etc/a.txt").exists());
    }

    /// Within one host: never a folder into itself, a copy into its own
    /// folder is a duplicate, and nothing lands on a folder holding a source.
    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn within_one_host_the_local_plans_rules_hold() {
        let remote = tempfile::tempdir().unwrap();
        let r = display_path(remote.path());
        std::fs::create_dir_all(remote.path().join("d/d/inner")).unwrap();
        std::fs::write(remote.path().join("d/a.txt"), b"a").unwrap();
        let h = host(remote.path()).await;
        let into_itself = start(
            FsOpKind::Copy,
            h.clone(),
            vec![format!("{r}/d")],
            h.clone(),
            format!("{r}/d/d"),
            events().0,
        )
        .await;
        assert!(into_itself.unwrap_err().contains("into itself"));
        let landing = start(
            FsOpKind::Copy,
            h.clone(),
            vec![format!("{r}/d/d")],
            h.clone(),
            r.clone(),
            events().0,
        )
        .await;
        assert!(landing.unwrap_err().contains("would land on the folder"));

        // A copy beside itself: `a (2).txt`, nothing asked.
        let (emit, seen) = events();
        start(
            FsOpKind::Copy,
            h.clone(),
            vec![format!("{r}/d/a.txt")],
            h,
            format!("{r}/d"),
            emit,
        )
        .await
        .unwrap();
        let end = finished(&seen).await;
        assert_eq!(end.state, FsOpEventState::Done, "{end:?}");
        assert!(!seen
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.state == FsOpEventState::Conflict));
        assert_eq!(
            std::fs::read(remote.path().join("d/a (2).txt")).unwrap(),
            b"a"
        );
        assert_eq!(std::fs::read(remote.path().join("d/a.txt")).unwrap(), b"a");
    }

    #[test]
    fn one_hosts_plan_matches_the_local_rules() {
        assert!(
            under("/a/b", "/a") && under("/a", "/") && !under("/ab", "/a") && !under("/a", "/a")
        );
    }

    #[test]
    fn host_paths_join_and_split_as_posix() {
        assert_eq!(posix_join("/home/u", "a.txt"), "/home/u/a.txt");
        assert_eq!(posix_join("/", "a"), "/a");
        assert_eq!(posix_parent("/home/u/a.txt"), "/home/u");
        assert_eq!(posix_parent("/a"), "/");
        assert_eq!(posix_parent("/"), "/");
        // Names from either side: a host's path, or this computer's.
        assert_eq!(file_name("/a/b/c.txt"), "c.txt");
        assert_eq!(file_name("C:\\x\\y.md"), "y.md");
        assert_eq!(file_name("/a/b/"), "b");
    }
}
