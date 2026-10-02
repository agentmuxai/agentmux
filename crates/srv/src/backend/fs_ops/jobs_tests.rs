// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Copy/move jobs, against real temp folders. Every test runs its own
//! `Jobs`, so tests don't share slots or ops.

use super::*;

// ── Harness ──────────────────────────────────────────────────────────────

struct Recorder {
    events: Mutex<Vec<FsOpEvent>>,
    changed: Condvar,
}

impl Recorder {
    /// Wait until `found` picks something out of the events so far.
    fn wait_until<T>(&self, what: &str, found: impl Fn(&[FsOpEvent]) -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut events = self.events.lock().unwrap();
        loop {
            if let Some(t) = found(&events) {
                return t;
            }
            let now = Instant::now();
            assert!(now < deadline, "timed out waiting for {what}; states so far: {:?}", states(&events));
            events = self.changed.wait_timeout(events, deadline - now).unwrap().0;
        }
    }

    fn final_event(&self) -> FsOpEvent {
        self.wait_until("a final event", |evs| evs.iter().find(|e| is_final(e.state)).cloned())
    }

    /// The `n`th conflict event (1-based).
    fn conflict(&self, n: usize) -> FsOpEvent {
        self.wait_until("a conflict", |evs| {
            evs.iter().filter(|e| e.state == FsOpEventState::Conflict).nth(n - 1).cloned()
        })
    }

    fn all(&self) -> Vec<FsOpEvent> {
        self.events.lock().unwrap().clone()
    }

    fn count(&self, state: FsOpEventState) -> usize {
        self.all().iter().filter(|e| e.state == state).count()
    }
}

fn states(events: &[FsOpEvent]) -> Vec<FsOpEventState> {
    events.iter().map(|e| e.state).collect()
}

fn is_final(state: FsOpEventState) -> bool {
    matches!(state, FsOpEventState::Done | FsOpEventState::Failed | FsOpEventState::Canceled)
}

fn recorder() -> (Arc<Recorder>, Emit) {
    let rec = Arc::new(Recorder { events: Mutex::new(Vec::new()), changed: Condvar::new() });
    let sink = rec.clone();
    let emit: Emit = Arc::new(move |e: &FsOpEvent| {
        sink.events.lock().unwrap().push(e.clone());
        sink.changed.notify_all();
    });
    (rec, emit)
}

fn jobs_with(configure: impl FnOnce(&mut JobConfig)) -> Arc<Jobs> {
    let mut config = JobConfig::default();
    configure(&mut config);
    Arc::new(Jobs::new(config))
}

fn jobs() -> Arc<Jobs> {
    jobs_with(|_| {})
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn req(kind: FsOpKind, sources: &[&Path], dest_dir: &Path) -> FsOpStartReq {
    FsOpStartReq {
        kind,
        sources: sources.iter().map(|p| s(p)).collect(),
        dest_dir: s(dest_dir),
        block_id: "block-1".to_string(),
    }
}

/// Start an op and return its id and recorder.
fn start(jobs: &Arc<Jobs>, req: FsOpStartReq) -> (String, Arc<Recorder>) {
    let (rec, emit) = recorder();
    let op_id = jobs.start(&req, emit).unwrap_or_else(|e| panic!("start refused: {e}"));
    (op_id.op_id, rec)
}

/// Start an op, wait for its final event, and check it left the registry.
fn run(jobs: &Arc<Jobs>, req: FsOpStartReq) -> FsOpEvent {
    let (_, rec) = start(jobs, req);
    let last = rec.final_event();
    wait_unregistered(jobs);
    last
}

fn wait_unregistered(jobs: &Jobs) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while jobs.op_count() > 0 {
        assert!(Instant::now() < deadline, "a finished op stayed in the registry");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn write(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> =
        std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

fn no_temp_files(dir: &Path) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        assert!(!name.starts_with(".agentmux-"), "a temporary entry was left behind: {name}");
        if entry.file_type().unwrap().is_dir() {
            no_temp_files(&entry.path());
        }
    }
}

fn assert_done_cleanly(last: &FsOpEvent) {
    assert_eq!(last.state, FsOpEventState::Done, "{last:?}");
    assert!(last.failures.is_none(), "unexpected failures: {:?}", last.failures);
    assert_eq!(last.done_items, last.total_items, "{last:?}");
    assert_eq!(last.done_bytes, last.total_bytes, "{last:?}");
}

/// Try to create a file symlink; Windows needs Developer Mode or elevation,
/// so a test that needs one is skipped where it can't be made.
fn try_symlink_file(target: &Path, link: &Path) -> bool {
    #[cfg(unix)]
    return std::os::unix::fs::symlink(target, link).is_ok();
    #[cfg(windows)]
    return std::os::windows::fs::symlink_file(target, link).is_ok();
}

fn try_symlink_dir(target: &Path, link: &Path) -> bool {
    #[cfg(unix)]
    return std::os::unix::fs::symlink(target, link).is_ok();
    #[cfg(windows)]
    return std::os::windows::fs::symlink_dir(target, link).is_ok();
}

// ── Copy ─────────────────────────────────────────────────────────────────

#[test]
fn copies_a_file_and_a_nested_tree() {
    let (src, dest) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(&src.path().join("a.txt"), "hello");
    write(&src.path().join("tree/one.txt"), "1");
    write(&src.path().join("tree/sub/two.txt"), "22");
    std::fs::create_dir(src.path().join("tree/empty")).unwrap();

    let jobs = jobs();
    let (_, rec) = start(&jobs, req(FsOpKind::Copy, &[&src.path().join("a.txt"), &src.path().join("tree")], dest.path()));
    let last = rec.final_event();
    assert_done_cleanly(&last);
    assert_eq!(last.total_items, 6, "a.txt, tree, one.txt, sub, two.txt, empty");
    assert_eq!(last.total_bytes, 8);
    let events = rec.all();
    assert_eq!(events.first().unwrap().state, FsOpEventState::Running);
    assert_eq!(events.iter().filter(|e| is_final(e.state)).count(), 1, "exactly one final event, last");
    assert!(is_final(events.last().unwrap().state));

    assert_eq!(read(&dest.path().join("a.txt")), "hello");
    assert_eq!(read(&dest.path().join("tree/one.txt")), "1");
    assert_eq!(read(&dest.path().join("tree/sub/two.txt")), "22");
    assert!(dest.path().join("tree/empty").is_dir());
    assert_eq!(read(&src.path().join("a.txt")), "hello", "a copy leaves the source");
    no_temp_files(dest.path());
    wait_unregistered(&jobs);
}

#[test]
fn a_copy_keeps_the_modification_time() {
    let (src, dest) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let file = src.path().join("old.txt");
    write(&file, "x");
    let old = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_500_000_000);
    std::fs::File::options().write(true).open(&file).unwrap().set_modified(old).unwrap();
    assert_done_cleanly(&run(&jobs(), req(FsOpKind::Copy, &[&file], dest.path())));
    let copied = std::fs::metadata(dest.path().join("old.txt")).unwrap().modified().unwrap();
    assert_eq!(copied, old);
}

#[test]
fn copying_into_an_existing_folder_merges() {
    let (src, dest) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(&src.path().join("d/new.txt"), "new");
    write(&src.path().join("d/sub/x.txt"), "x");
    write(&dest.path().join("d/old.txt"), "old");

    let jobs = jobs();
    let (_, rec) = start(&jobs, req(FsOpKind::Copy, &[&src.path().join("d")], dest.path()));
    assert_done_cleanly(&rec.final_event());
    assert_eq!(rec.count(FsOpEventState::Conflict), 0, "a folder onto a folder is a merge, not a conflict");
    assert_eq!(names(&dest.path().join("d")), ["new.txt", "old.txt", "sub"]);
    assert_eq!(read(&dest.path().join("d/sub/x.txt")), "x");
}

#[test]
fn copying_into_the_same_folder_keeps_both() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("a.txt"), "a");
    write(&dir.path().join("d/f.txt"), "f");
    let jobs = jobs();
    let (_, rec) = start(&jobs, req(FsOpKind::Copy, &[&dir.path().join("a.txt"), &dir.path().join("d")], dir.path()));
    assert_done_cleanly(&rec.final_event());
    assert_eq!(rec.count(FsOpEventState::Conflict), 0, "nothing to ask: it's a duplicate");
    assert_eq!(read(&dir.path().join("a (2).txt")), "a");
    assert_eq!(read(&dir.path().join("d (2)/f.txt")), "f");

    assert_done_cleanly(&run(&jobs, req(FsOpKind::Copy, &[&dir.path().join("a.txt")], dir.path())));
    assert_eq!(names(dir.path()), ["a (2).txt", "a (3).txt", "a.txt", "d", "d (2)"]);
}

#[test]
fn keep_both_names() {
    let n = |name: &str, i: u32, dir: bool| numbered_name(OsStr::new(name), i, dir).to_string_lossy().into_owned();
    assert_eq!(n("a.txt", 2, false), "a (2).txt");
    assert_eq!(n("a.tar.gz", 2, false), "a.tar (2).gz");
    assert_eq!(n("Makefile", 3, false), "Makefile (3)");
    assert_eq!(n(".env", 2, false), ".env (2)");
    assert_eq!(n("dir", 2, true), "dir (2)");
    assert_eq!(n("my.folder", 2, true), "my.folder (2)", "a folder's dot isn't an extension");
}

#[test]
fn keep_both_moves_past_names_taken_meanwhile() {
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("a.txt");
    write(&dir.path().join("a (2).txt"), "taken");
    let mut tried = Vec::new();
    let put = keep_both(&dest, false, &ProtectedPaths::default(), |c| {
        tried.push(file_name_lossy(c));
        std::fs::OpenOptions::new().write(true).create_new(true).open(c).map(drop)
    })
    .unwrap();
    assert_eq!(file_name_lossy(&put), "a (3).txt");
    assert_eq!(tried, ["a (2).txt", "a (3).txt"]);
    assert_eq!(read(&dir.path().join("a (2).txt")), "taken", "never overwritten");
}

// ── Conflicts ────────────────────────────────────────────────────────────

/// `src/a.txt` = "new" and `dest/a.txt` = "old".
fn conflicting_file() -> (tempfile::TempDir, tempfile::TempDir) {
    let (src, dest) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(&src.path().join("a.txt"), "new");
    write(&dest.path().join("a.txt"), "old");
    (src, dest)
}

#[test]
fn a_conflict_waits_and_skip_keeps_the_destination() {
    let (src, dest) = conflicting_file();
    let jobs = jobs();
    let (op_id, rec) = start(&jobs, req(FsOpKind::Copy, &[&src.path().join("a.txt")], dest.path()));
    let conflict = rec.conflict(1).conflict.expect("a conflict event carries the conflict");
    assert!(conflict.source.ends_with("a.txt") && conflict.dest.ends_with("a.txt"));
    assert!(!conflict.source_is_dir && !conflict.dest_is_dir);
    assert_eq!((conflict.source_size, conflict.dest_size), (Some(3), Some(3)));
    assert!(conflict.source_mtime.is_some() && conflict.dest_mtime.is_some());

    jobs.resolve(&op_id, FsOpChoice::Skip, false).unwrap();
    let last = rec.final_event();
    assert_eq!(last.state, FsOpEventState::Done);
    assert!(last.failures.is_none(), "a skip isn't a failure");
    assert_eq!(read(&dest.path().join("a.txt")), "old");
    wait_unregistered(&jobs);
    assert!(jobs.resolve(&op_id, FsOpChoice::Skip, false).is_err(), "a finished op takes no answer");
}

#[test]
fn replace_overwrites_without_leaving_temp_files() {
    let (src, dest) = conflicting_file();
    let jobs = jobs();
    let (op_id, rec) = start(&jobs, req(FsOpKind::Copy, &[&src.path().join("a.txt")], dest.path()));
    rec.conflict(1);
    jobs.resolve(&op_id, FsOpChoice::Replace, false).unwrap();
    assert_done_cleanly(&rec.final_event());
    assert_eq!(read(&dest.path().join("a.txt")), "new");
    assert_eq!(names(dest.path()), ["a.txt"]);
}

#[test]
fn keep_both_writes_a_numbered_copy() {
    let (src, dest) = conflicting_file();
    let jobs = jobs();
    let (op_id, rec) = start(&jobs, req(FsOpKind::Copy, &[&src.path().join("a.txt")], dest.path()));
    rec.conflict(1);
    jobs.resolve(&op_id, FsOpChoice::KeepBoth, false).unwrap();
    assert_done_cleanly(&rec.final_event());
    assert_eq!(read(&dest.path().join("a.txt")), "old");
    assert_eq!(read(&dest.path().join("a (2).txt")), "new");
}

#[test]
fn apply_to_all_answers_later_conflicts() {
    let (src, dest) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    for name in ["a.txt", "b.txt"] {
        write(&src.path().join(name), "new");
        write(&dest.path().join(name), "old");
    }
    let jobs = jobs();
    let (op_id, rec) =
        start(&jobs, req(FsOpKind::Copy, &[&src.path().join("a.txt"), &src.path().join("b.txt")], dest.path()));
    rec.conflict(1);
    jobs.resolve(&op_id, FsOpChoice::Replace, true).unwrap();
    assert_done_cleanly(&rec.final_event());
    assert_eq!(rec.count(FsOpEventState::Conflict), 1, "the second conflict was answered already");
    assert_eq!(read(&dest.path().join("a.txt")), "new");
    assert_eq!(read(&dest.path().join("b.txt")), "new");
}

#[test]
fn without_apply_to_all_each_conflict_is_asked() {
    let (src, dest) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    for name in ["a.txt", "b.txt"] {
        write(&src.path().join(name), "new");
        write(&dest.path().join(name), "old");
    }
    let jobs = jobs();
    let (op_id, rec) =
        start(&jobs, req(FsOpKind::Copy, &[&src.path().join("a.txt"), &src.path().join("b.txt")], dest.path()));
    rec.conflict(1);
    jobs.resolve(&op_id, FsOpChoice::Replace, false).unwrap();
    rec.conflict(2);
    jobs.resolve(&op_id, FsOpChoice::Skip, false).unwrap();
    assert_eq!(rec.final_event().state, FsOpEventState::Done);
    assert_eq!(read(&dest.path().join("a.txt")), "new");
    assert_eq!(read(&dest.path().join("b.txt")), "old");
}

#[test]
fn replace_across_types_swaps_a_folder_and_a_file() {
    let (src, dest) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    // A file over a folder…
    write(&src.path().join("x"), "file");
    write(&dest.path().join("x/inner.txt"), "inner");
    // …and a folder over a file.
    write(&src.path().join("y/f.txt"), "f");
    write(&dest.path().join("y"), "old file");

    let jobs = jobs();
    let (op_id, rec) = start(&jobs, req(FsOpKind::Copy, &[&src.path().join("x"), &src.path().join("y")], dest.path()));
    let conflict = rec.conflict(1).conflict.unwrap();
    assert!(conflict.dest_is_dir && !conflict.source_is_dir);
    jobs.resolve(&op_id, FsOpChoice::Replace, true).unwrap();
    assert_done_cleanly(&rec.final_event());
    assert_eq!(read(&dest.path().join("x")), "file");
    assert_eq!(read(&dest.path().join("y/f.txt")), "f");
    assert_eq!(names(dest.path()), ["x", "y"], "the replaced originals are gone, and no temporary names remain");
}

#[test]
fn replace_asks_again_if_the_destination_changed_type_meanwhile() {
    let (src, dest) = conflicting_file();
    let jobs = jobs();
    let (op_id, rec) = start(&jobs, req(FsOpKind::Copy, &[&src.path().join("a.txt")], dest.path()));
    assert!(!rec.conflict(1).conflict.unwrap().dest_is_dir);
    // While the prompt is up, the file becomes a folder.
    std::fs::remove_file(dest.path().join("a.txt")).unwrap();
    write(&dest.path().join("a.txt/inner.txt"), "keep me");
    jobs.resolve(&op_id, FsOpChoice::Replace, false).unwrap();

    let again = rec.conflict(2).conflict.unwrap();
    assert!(again.dest_is_dir, "the user is asked about what is there now");
    jobs.resolve(&op_id, FsOpChoice::Skip, false).unwrap();
    assert_eq!(rec.final_event().state, FsOpEventState::Done);
    assert_eq!(read(&dest.path().join("a.txt/inner.txt")), "keep me");
}

#[test]
fn a_finished_or_unknown_op_takes_no_answer() {
    let jobs = jobs();
    assert!(jobs.resolve("no-such-op", FsOpChoice::Skip, false).is_err());
    jobs.cancel("no-such-op");
}

// ── Refusals ─────────────────────────────────────────────────────────────

#[test]
fn a_folder_cannot_go_into_itself() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("folder");
    std::fs::create_dir_all(folder.join("sub")).unwrap();
    let jobs = jobs();
    let (_, emit) = recorder();

    let err = jobs.start(&req(FsOpKind::Copy, &[&folder], &folder.join("sub")), emit.clone()).unwrap_err();
    assert_eq!(err, "Can't copy a folder into itself.");
    let err = jobs.start(&req(FsOpKind::Copy, &[&folder], &folder), emit.clone()).unwrap_err();
    assert_eq!(err, "Can't copy a folder into itself.");
    let err = jobs.start(&req(FsOpKind::Move, &[&folder], &folder.join("sub")), emit.clone()).unwrap_err();
    assert_eq!(err, "Can't move a folder into itself.");

    // `/a/d/d` into `/a` would merge into `/a/d`, the folder that holds it.
    std::fs::create_dir_all(dir.path().join("d/d")).unwrap();
    let err = jobs.start(&req(FsOpKind::Copy, &[&dir.path().join("d/d")], dir.path()), emit).unwrap_err();
    assert!(err.contains("would land on the folder that holds"), "{err}");
    assert_eq!(jobs.op_count(), 0, "a refused op never registers");
}

#[cfg(windows)]
#[test]
fn into_itself_ignores_case_on_windows() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("Folder");
    std::fs::create_dir_all(folder.join("sub")).unwrap();
    let (_, emit) = recorder();
    let upper = PathBuf::from(s(&folder.join("sub")).to_uppercase());
    let err = jobs().start(&req(FsOpKind::Copy, &[&folder], &upper), emit).unwrap_err();
    assert_eq!(err, "Can't copy a folder into itself.");
}

#[test]
fn missing_sources_and_destinations_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("a.txt"), "a");
    let (_, emit) = recorder();
    let jobs = jobs();
    let err = jobs.start(&req(FsOpKind::Copy, &[&dir.path().join("gone.txt")], dir.path()), emit.clone()).unwrap_err();
    assert!(err.contains("doesn't exist"), "{err}");
    let err = jobs.start(&req(FsOpKind::Copy, &[&dir.path().join("a.txt")], &dir.path().join("nope")), emit.clone()).unwrap_err();
    assert!(err.contains("destination folder doesn't exist"), "{err}");
    let err = jobs.start(&req(FsOpKind::Copy, &[&dir.path().join("a.txt")], &dir.path().join("a.txt")), emit.clone()).unwrap_err();
    assert!(err.contains("isn't a folder"), "{err}");
    let err = jobs.start(&req(FsOpKind::Copy, &[], dir.path()), emit).unwrap_err();
    assert!(err.contains("Nothing"), "{err}");
}

#[test]
fn a_protected_destination_or_source_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let data = root_path.join("data");
    let free = root_path.join("free");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&free).unwrap();
    write(&free.join("a.txt"), "a");
    write(&data.join("state.db"), "db");
    let protect = ProtectedPaths::for_test(&root_path.join("home"), &data, &[], &[], &[]);
    let jobs = jobs_with(|c| c.protect = Some(protect));
    let (_, emit) = recorder();

    let err = jobs.start(&req(FsOpKind::Copy, &[&free.join("a.txt")], &data), emit.clone()).unwrap_err();
    assert!(err.contains("AgentMux's own data"), "{err}");
    let err = jobs.start(&req(FsOpKind::Move, &[&data.join("state.db")], &free), emit.clone()).unwrap_err();
    assert!(err.contains("AgentMux's own data"), "a move takes its source away, so the source is checked too: {err}");
    // Copying out of it changes nothing there.
    let (_, rec) = start(&jobs, req(FsOpKind::Copy, &[&data.join("state.db")], &free));
    assert_done_cleanly(&rec.final_event());
}

// ── Move ─────────────────────────────────────────────────────────────────

#[test]
fn move_renames_on_the_same_volume() {
    let (src, dest) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(&src.path().join("a.txt"), "a");
    write(&src.path().join("tree/sub/b.txt"), "bb");
    let last = run(&jobs(), req(FsOpKind::Move, &[&src.path().join("a.txt"), &src.path().join("tree")], dest.path()));
    assert_done_cleanly(&last);
    assert_eq!(last.total_items, 4);
    assert_eq!(read(&dest.path().join("a.txt")), "a");
    assert_eq!(read(&dest.path().join("tree/sub/b.txt")), "bb");
    assert!(names(src.path()).is_empty(), "the sources are gone");
}

#[test]
fn moving_into_the_folder_it_is_in_does_nothing() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("a.txt"), "a");
    let last = run(&jobs(), req(FsOpKind::Move, &[&dir.path().join("a.txt")], dir.path()));
    assert_done_cleanly(&last);
    assert_eq!(last.total_items, 0);
    assert_eq!(names(dir.path()), ["a.txt"]);
}

#[test]
fn a_move_conflict_skip_keeps_the_source_and_replace_moves_it() {
    let (src, dest) = conflicting_file();
    let jobs = jobs();
    let source = src.path().join("a.txt");

    let (op_id, rec) = start(&jobs, req(FsOpKind::Move, &[&source], dest.path()));
    rec.conflict(1);
    jobs.resolve(&op_id, FsOpChoice::Skip, false).unwrap();
    rec.final_event();
    assert_eq!(read(&source), "new", "a skipped move keeps its source");
    assert_eq!(read(&dest.path().join("a.txt")), "old");

    let (op_id, rec) = start(&jobs, req(FsOpKind::Move, &[&source], dest.path()));
    rec.conflict(1);
    jobs.resolve(&op_id, FsOpChoice::Replace, false).unwrap();
    assert_done_cleanly(&rec.final_event());
    assert!(!source.exists());
    assert_eq!(read(&dest.path().join("a.txt")), "new");
}

#[test]
fn a_move_merges_into_an_existing_folder() {
    let (src, dest) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(&src.path().join("d/new.txt"), "new");
    write(&dest.path().join("d/old.txt"), "old");
    assert_done_cleanly(&run(&jobs(), req(FsOpKind::Move, &[&src.path().join("d")], dest.path())));
    assert_eq!(names(&dest.path().join("d")), ["new.txt", "old.txt"]);
    assert!(!src.path().join("d").exists(), "the emptied source folder is removed");
}

#[test]
fn a_move_across_volumes_copies_then_deletes() {
    let (src, dest) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(&src.path().join("c.txt"), "c");
    write(&src.path().join("tree/one.txt"), "1");
    write(&src.path().join("tree/sub/two.txt"), "2");
    let jobs = jobs_with(|c| c.assume_cross_volume = true);
    let last = run(&jobs, req(FsOpKind::Move, &[&src.path().join("c.txt"), &src.path().join("tree")], dest.path()));
    assert_done_cleanly(&last);
    assert_eq!(read(&dest.path().join("c.txt")), "c");
    assert_eq!(read(&dest.path().join("tree/sub/two.txt")), "2");
    assert!(names(src.path()).is_empty(), "every source went once its copy was complete");
    no_temp_files(dest.path());
}

#[test]
fn a_move_across_volumes_never_deletes_a_skipped_source() {
    let (src, dest) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(&src.path().join("tree/one.txt"), "1");
    write(&src.path().join("tree/sub/two.txt"), "new");
    write(&dest.path().join("tree/sub/two.txt"), "old");
    let jobs = jobs_with(|c| c.assume_cross_volume = true);
    let (op_id, rec) = start(&jobs, req(FsOpKind::Move, &[&src.path().join("tree")], dest.path()));
    rec.conflict(1);
    jobs.resolve(&op_id, FsOpChoice::Skip, false).unwrap();
    assert_eq!(rec.final_event().state, FsOpEventState::Done);

    assert!(!src.path().join("tree/one.txt").exists(), "a moved file's source is gone");
    assert_eq!(read(&dest.path().join("tree/one.txt")), "1");
    assert_eq!(read(&src.path().join("tree/sub/two.txt")), "new", "the skipped source stays");
    assert_eq!(read(&dest.path().join("tree/sub/two.txt")), "old");
    assert!(src.path().join("tree/sub").is_dir(), "and so do the folders holding it");
}

#[test]
fn read_only_files_copy_and_move() {
    let (src, dest) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let file = src.path().join("ro.txt");
    write(&file, "ro");
    let mut perms = std::fs::metadata(&file).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&file, perms).unwrap();

    assert_done_cleanly(&run(&jobs(), req(FsOpKind::Copy, &[&file], dest.path())));
    let copied = dest.path().join("ro.txt");
    assert!(std::fs::metadata(&copied).unwrap().permissions().readonly(), "permissions are copied");
    assert_eq!(read(&copied), "ro");

    // Across volumes, a read-only original must still go once copied.
    let jobs = jobs_with(|c| c.assume_cross_volume = true);
    let (op_id, rec) = start(&jobs, req(FsOpKind::Move, &[&file], dest.path()));
    rec.conflict(1);
    jobs.resolve(&op_id, FsOpChoice::KeepBoth, false).unwrap();
    assert_done_cleanly(&rec.final_event());
    assert!(!file.exists());
    assert_eq!(read(&dest.path().join("ro (2).txt")), "ro");

    // Let the temp folder clean itself up.
    for name in ["ro.txt", "ro (2).txt"] {
        let path = dest.path().join(name);
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        std::fs::set_permissions(&path, perms).unwrap();
    }
}

#[test]
fn per_item_failures_are_reported_and_the_rest_goes_on() {
    let (src, dest) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(&src.path().join("d/f.txt"), "f");
    write(&src.path().join("z.txt"), "z");
    // `d/f.txt` leaves with `d`, so it is gone by its turn.
    let last = run(
        &jobs(),
        req(FsOpKind::Move, &[&src.path().join("d"), &src.path().join("d/f.txt"), &src.path().join("z.txt")], dest.path()),
    );
    assert_eq!(last.state, FsOpEventState::Done);
    let failures = last.failures.expect("one item failed");
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(failures[0].path.ends_with("f.txt") && !failures[0].ok);
    assert!(failures[0].error.as_deref().unwrap().contains("doesn't exist"), "{failures:?}");
    assert_eq!(read(&dest.path().join("d/f.txt")), "f");
    assert_eq!(read(&dest.path().join("z.txt")), "z", "the item after the failure still moved");
}

// ── Cancel and the queue ─────────────────────────────────────────────────

#[test]
fn cancel_mid_file_removes_the_partial_copy() {
    let (src, dest) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let big = src.path().join("big.bin");
    std::fs::write(&big, vec![7u8; 256 * 1024]).unwrap();
    let jobs = jobs_with(|c| {
        c.chunk_size = 4096;
        c.after_chunk = Some(Arc::new(|control: &OpControl, written: u64| {
            if written >= 16 * 1024 {
                control.cancel();
            }
        }));
    });
    let last = run(&jobs, req(FsOpKind::Copy, &[&big], dest.path()));
    assert_eq!(last.state, FsOpEventState::Canceled);
    assert!(last.done_bytes < last.total_bytes, "{last:?}");
    assert!(names(dest.path()).is_empty(), "no partial file, under any name: {:?}", names(dest.path()));
    assert_eq!(std::fs::metadata(&big).unwrap().len(), 256 * 1024, "the source is untouched");
}

#[test]
fn cancel_mid_move_across_volumes_keeps_the_source() {
    let (src, dest) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let big = src.path().join("big.bin");
    std::fs::write(&big, vec![7u8; 64 * 1024]).unwrap();
    let jobs = jobs_with(|c| {
        c.chunk_size = 4096;
        c.assume_cross_volume = true;
        c.after_chunk = Some(Arc::new(|control: &OpControl, _| control.cancel()));
    });
    assert_eq!(run(&jobs, req(FsOpKind::Move, &[&big], dest.path())).state, FsOpEventState::Canceled);
    assert!(names(dest.path()).is_empty());
    assert_eq!(std::fs::metadata(&big).unwrap().len(), 64 * 1024);
}

#[test]
fn cancel_wakes_a_conflict_wait() {
    let (src, dest) = conflicting_file();
    let jobs = jobs();
    let (op_id, rec) = start(&jobs, req(FsOpKind::Copy, &[&src.path().join("a.txt")], dest.path()));
    rec.conflict(1);
    jobs.cancel(&op_id);
    let last = rec.final_event();
    assert_eq!(last.state, FsOpEventState::Canceled);
    assert!(last.error.is_none(), "the user canceled; nothing to explain");
    assert_eq!(read(&dest.path().join("a.txt")), "old");
    wait_unregistered(&jobs);
}

#[test]
fn an_unanswered_conflict_stops_the_op_after_the_limit() {
    let (src, dest) = conflicting_file();
    let jobs = jobs_with(|c| c.conflict_wait_limit = Duration::from_millis(50));
    let last = run(&jobs, req(FsOpKind::Copy, &[&src.path().join("a.txt")], dest.path()));
    assert_eq!(last.state, FsOpEventState::Canceled);
    assert!(last.error.as_deref().unwrap_or("").contains("No one answered"), "{last:?}");
    assert_eq!(read(&dest.path().join("a.txt")), "old");
}

#[test]
fn a_queued_op_takes_no_answer_and_can_be_canceled() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("a.txt"), "a");
    let jobs = jobs_with(|c| c.max_concurrent = 0);
    let (op_id, rec) = start(&jobs, req(FsOpKind::Copy, &[&dir.path().join("a.txt")], dir.path()));
    let first = rec.wait_until("the queued event", |evs| evs.first().cloned());
    assert_eq!(first.state, FsOpEventState::Running);
    assert_eq!((first.done_items, first.total_items), (0, 0));
    let err = jobs.resolve(&op_id, FsOpChoice::Replace, true).unwrap_err();
    assert!(err.contains("isn't waiting"), "{err}");
    jobs.cancel(&op_id);
    assert_eq!(rec.final_event().state, FsOpEventState::Canceled);
    wait_unregistered(&jobs);
    assert_eq!(names(dir.path()), ["a.txt"]);
}

#[test]
fn queued_ops_run_when_a_slot_frees() {
    let (src, dest) = conflicting_file();
    write(&src.path().join("b.txt"), "b");
    let jobs = jobs_with(|c| c.max_concurrent = 1);
    let (first, first_rec) = start(&jobs, req(FsOpKind::Copy, &[&src.path().join("a.txt")], dest.path()));
    first_rec.conflict(1);
    let (_, second_rec) = start(&jobs, req(FsOpKind::Copy, &[&src.path().join("b.txt")], dest.path()));
    second_rec.wait_until("the queued event", |evs| evs.first().cloned());
    std::thread::sleep(Duration::from_millis(100));
    assert!(second_rec.all().iter().all(|e| e.state == FsOpEventState::Running && e.total_items == 0), "still queued");

    jobs.resolve(&first, FsOpChoice::Skip, false).unwrap();
    assert_eq!(first_rec.final_event().state, FsOpEventState::Done);
    assert_done_cleanly(&second_rec.final_event());
    assert_eq!(read(&dest.path().join("b.txt")), "b");
    wait_unregistered(&jobs);
}

// ── Links and events ─────────────────────────────────────────────────────

#[test]
fn a_symlink_is_copied_as_a_link_and_never_walked() {
    let (src, dest) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(&src.path().join("target.txt"), "t");
    if !try_symlink_file(Path::new("target.txt"), &src.path().join("link.txt")) {
        return;
    }
    // A folder holding a link to a big folder elsewhere: only the link goes.
    let elsewhere = tempfile::tempdir().unwrap();
    write(&elsewhere.path().join("huge.bin"), "lots");
    std::fs::create_dir(src.path().join("tree")).unwrap();
    if !try_symlink_dir(elsewhere.path(), &src.path().join("tree/out")) {
        return;
    }

    let last = run(&jobs(), req(FsOpKind::Copy, &[&src.path().join("link.txt"), &src.path().join("tree")], dest.path()));
    assert_done_cleanly(&last);
    assert_eq!(last.total_items, 3, "link.txt, tree and tree/out; nothing behind the link");
    assert_eq!(last.total_bytes, 0);

    let link = dest.path().join("link.txt");
    assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
    assert_eq!(std::fs::read_link(&link).unwrap(), Path::new("target.txt"), "the stored target, as it was");
    let out = dest.path().join("tree/out");
    assert!(std::fs::symlink_metadata(&out).unwrap().file_type().is_symlink());
    assert_eq!(std::fs::read_link(&out).unwrap(), elsewhere.path());
}

#[test]
fn running_events_are_throttled_and_the_final_counts_are_complete() {
    let (src, dest) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    for i in 0..300 {
        write(&src.path().join(format!("many/f{i:03}.txt")), "x");
    }
    let began = Instant::now();
    let (_, rec) = start(&jobs(), req(FsOpKind::Copy, &[&src.path().join("many")], dest.path()));
    let last = rec.final_event();
    let elapsed = began.elapsed();
    assert_done_cleanly(&last);
    assert_eq!((last.total_items, last.total_bytes), (301, 300));
    // One while queued, one with the totals, then one per 100 ms at most.
    let allowed = 3 + (elapsed.as_millis() / PROGRESS_INTERVAL.as_millis()) as usize;
    let running = rec.count(FsOpEventState::Running);
    assert!(running <= allowed, "{running} running events in {elapsed:?}");
}

#[test]
fn the_broker_emitter_publishes_to_the_block() {
    use crate::backend::mps::{SubscriptionRequest, WpsClient};
    struct Client(Mutex<Vec<MuxEvent>>);
    impl WpsClient for Arc<Client> {
        fn send_event(&self, _route_id: &str, event: MuxEvent) {
            self.0.lock().unwrap().push(event);
        }
    }
    let broker = Arc::new(Broker::new());
    let client = Arc::new(Client(Mutex::new(Vec::new())));
    broker.set_client(Box::new(client.clone()));
    broker.subscribe(
        "route",
        SubscriptionRequest { event: EVENT_FILES_OP.to_string(), scopes: vec!["block:b1".to_string()], allscopes: false },
    );
    let emit = broker_emitter(broker, "b1".to_string());
    let job_config = JobConfig::default();
    let job = Job::new(&job_config, "op-1".to_string(), FsOpKind::Copy, Arc::new(OpControl::default()), emit.clone());
    emit(&job.event(FsOpEventState::Running));
    let events = client.0.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event, EVENT_FILES_OP);
    assert_eq!(events[0].persist, 0);
    let data = events[0].data.as_ref().unwrap();
    assert_eq!(data["op_id"], "op-1");
    assert_eq!(data["kind"], "copy");
    assert_eq!(data["state"], "running");
}
