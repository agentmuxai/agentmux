use super::*;
use std::time::{Duration, SystemTime};

fn set_mtime(path: &Path, secs: u64) {
    let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(secs)).unwrap();
}

/// An AppImage-like file of `len` bytes with modification time `secs`.
fn appimage(dir: &Path, name: &str, len: usize, secs: u64) -> PathBuf {
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, vec![7u8; len]).unwrap();
    set_mtime(&path, secs);
    path
}

fn sidecar(file: &Path, channel: &str) {
    std::fs::write(appimage_channel_file(file), format!("{channel}\n")).unwrap();
}

fn text(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

#[test]
fn a_file_moved_and_renamed_with_its_record_left_behind_starts_in_the_same_channel() {
    let dir = tempfile::tempdir().unwrap();
    let record = dir.path().join("shared").join(RECORD_FILE);
    let desktop = dir.path().join("Desktop");
    let old = appimage(&desktop, "AgentMux_0.60.29_amd64.AppImage", 1000, 5_000);
    sidecar(&old, "local-main-1");

    // A start from where it is: nothing moved, and the file is remembered.
    let first = reconcile(&record, &old);
    assert_eq!(first, Reconciled { channel: Some("local-main-1".into()), moved_from: None });

    // What AppImageLauncher's Integrate does: the file moves and takes a hash suffix; the record stays behind.
    let apps = dir.path().join("Applications");
    std::fs::create_dir_all(&apps).unwrap();
    let new = apps.join("AgentMux_0.60.29_amd64_9f8e7d.AppImage");
    std::fs::rename(&old, &new).unwrap();
    assert!(!appimage_channel_file(&new).exists());

    // The channel is known before anything is written (what the data paths read) ...
    assert_eq!(lookup(&record, &new).map(|f| f.channel).as_deref(), Some("local-main-1"));
    // ... and the start repairs the record beside the file and says where the file came from.
    let second = reconcile(&record, &new);
    assert_eq!(second.channel.as_deref(), Some("local-main-1"));
    assert_eq!(second.moved_from, Some(text(&old)));
    assert_eq!(read_channel_file(&appimage_channel_file(&new)).as_deref(), Some("local-main-1"));

    // The next start has nothing left to do.
    assert_eq!(reconcile(&record, &new), Reconciled { channel: Some("local-main-1".into()), moved_from: None });
}

#[test]
fn a_copy_is_not_a_move_while_the_original_is_still_there() {
    let dir = tempfile::tempdir().unwrap();
    let record = dir.path().join(RECORD_FILE);
    let original = appimage(dir.path(), "a/AgentMux.AppImage", 500, 9_000);
    sidecar(&original, "local-x");
    reconcile(&record, &original);

    // The same bytes with the same modification time, at another path: a duplicate of a release.
    let copy = appimage(dir.path(), "b/AgentMux.AppImage", 500, 9_000);
    assert_eq!(lookup(&record, &copy), None, "a duplicate never takes its twin's channel");
    assert_eq!(reconcile(&record, &copy), Reconciled::default());
    assert!(!appimage_channel_file(&copy).exists());
    // The original keeps its own.
    assert_eq!(lookup(&record, &original).map(|f| f.channel).as_deref(), Some("local-x"));

    // Once the original is gone, that copy (with its time kept, as a move across disks can) is where it went.
    std::fs::remove_file(&original).unwrap();
    assert_eq!(lookup(&record, &copy).map(|f| f.channel).as_deref(), Some("local-x"));
}

#[test]
fn a_different_file_never_takes_the_channel() {
    let dir = tempfile::tempdir().unwrap();
    let record = dir.path().join(RECORD_FILE);
    let old = appimage(dir.path(), "AgentMux_0.60.29.AppImage", 800, 3_000);
    sidecar(&old, "local-x");
    reconcile(&record, &old);
    std::fs::remove_file(&old).unwrap();

    // Another size, another modification time, or the same path now holding a different file.
    let other_size = appimage(dir.path(), "n/other.AppImage", 801, 3_000);
    let other_time = appimage(dir.path(), "n/other2.AppImage", 800, 3_001);
    let replaced = appimage(dir.path(), "AgentMux_0.60.29.AppImage", 800, 3_002);
    for file in [&other_size, &other_time, &replaced] {
        assert_eq!(lookup(&record, file), None, "{}", file.display());
    }
    // A move that changes the modification time is not recognised: the file starts as it is built.
    let copied = appimage(dir.path(), "m/AgentMux.AppImage", 800, 4_444);
    assert_eq!(reconcile(&record, &copied), Reconciled::default());
}

#[test]
fn nothing_recorded_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let record = dir.path().join("shared").join(RECORD_FILE);
    let file = appimage(dir.path(), "AgentMux.AppImage", 100, 1_000);
    assert_eq!(reconcile(&record, &file), Reconciled::default());
    assert!(!appimage_channel_file(&file).exists());
    assert!(!record.exists(), "a plain release has no channel to keep");
    // A path that isn't there is nothing either.
    assert_eq!(reconcile(&record, &dir.path().join("missing.AppImage")), Reconciled::default());
}

#[test]
fn the_record_beside_the_file_wins_and_is_what_is_remembered() {
    let dir = tempfile::tempdir().unwrap();
    let record = dir.path().join(RECORD_FILE);
    let file = appimage(dir.path(), "AgentMux.AppImage", 100, 1_000);
    sidecar(&file, "first");
    reconcile(&record, &file);
    sidecar(&file, "second");
    let r = reconcile(&record, &file);
    assert_eq!(r.channel.as_deref(), Some("second"));
    assert_eq!(lookup(&record, &file).map(|f| f.channel).as_deref(), Some("second"));
    assert_eq!(load(&record).len(), 1, "one record per file");
}

#[test]
fn a_record_that_is_not_a_channel_name_or_cannot_be_read_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let record = dir.path().join(RECORD_FILE);
    let file = appimage(dir.path(), "AgentMux.AppImage", 100, 1_000);
    let id = file_id(&file).unwrap();
    let too_long = "c".repeat(65);
    for bad in ["../evil", "a b", "", "x/y", too_long.as_str()] {
        let records = vec![Record { path: text(&dir.path().join("gone.AppImage")), channel: bad.into(), id: id.clone() }];
        std::fs::write(&record, serde_json::to_vec(&records).unwrap()).unwrap();
        assert_eq!(lookup(&record, &file), None, "{bad:?}");
    }
    // Not even JSON: nothing is found, and a start can still record over it.
    std::fs::write(&record, "{ not json").unwrap();
    assert_eq!(lookup(&record, &file), None);
    sidecar(&file, "good");
    assert_eq!(reconcile(&record, &file).channel.as_deref(), Some("good"));
    assert_eq!(lookup(&record, &file).map(|f| f.channel).as_deref(), Some("good"));
    // A record beside the file that isn't a channel name is ignored too.
    std::fs::write(appimage_channel_file(&file), "../evil\n").unwrap();
    assert_eq!(reconcile(&dir.path().join("other.json"), &file), Reconciled::default());
}

#[test]
fn only_the_most_recent_files_are_remembered() {
    let dir = tempfile::tempdir().unwrap();
    let record = dir.path().join(RECORD_FILE);
    let mut files = Vec::new();
    for n in 0..(MAX_RECORDS + 4) {
        let file = appimage(dir.path(), &format!("f{n}.AppImage"), 100 + n, 1_000 + n as u64);
        remember(&record, &file, "chan").unwrap();
        files.push(file);
    }
    assert_eq!(load(&record).len(), MAX_RECORDS);
    assert_eq!(lookup(&record, &files[0]), None, "the oldest was dropped");
    assert!(lookup(&record, files.last().unwrap()).is_some());
}
