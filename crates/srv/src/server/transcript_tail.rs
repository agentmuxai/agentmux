// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The last lines of an agent's transcript, for the readers that want only a few
//! (`Agents transcript`, the previews in `Agents conversations`).
//!
//! Both used to read the block's whole `output` file (hundreds of MB for a long
//! session) under the store's one connection, on a tokio worker, to return at
//! most 500 lines. On a busy machine that parked workers and the connection long
//! enough for every other call to queue behind them: a `GetControllerStatus`
//! that does no work of its own took 38 s (narko, 2026-10-29, CPU ~96%). This
//! reads a bounded tail instead, and [`off_worker`] keeps what is left off the
//! async workers.

use std::sync::Arc;

use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;
use crate::backend::transcript::source::{Tail, TranscriptSource};

/// The first tail read; grown fourfold while it holds too few lines.
const FIRST_WINDOW: i64 = 256 * 1024;

/// What a transcript read found.
pub(crate) struct TailLines {
    /// The last non-blank lines, oldest first.
    pub lines: Vec<String>,
    /// Whether the transcript holds earlier lines than these.
    pub truncated: bool,
}

/// Run blocking work (store reads) off the async workers. A panic in `f` gives
/// `T::default()`, as no result at all would for a best-effort reader.
pub(crate) async fn off_worker<T: Default + Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(f).await.unwrap_or_default()
}

/// The last `requested` non-blank lines of `bytes`. `complete`: `bytes` starts at
/// the start of the file; otherwise its first line may be cut and is dropped.
/// `None` when a partial window holds no more than `requested` whole lines, so
/// the caller must read more.
fn last_lines(bytes: &[u8], requested: usize, complete: bool) -> Option<TailLines> {
    let text = String::from_utf8_lossy(bytes);
    let mut lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if !complete && !lines.is_empty() {
        lines.remove(0);
    }
    if !complete && lines.len() <= requested {
        return None;
    }
    let truncated = lines.len() > requested;
    let from = lines.len().saturating_sub(requested);
    Some(TailLines { lines: lines[from..].iter().map(|l| l.to_string()).collect(), truncated })
}

/// [`last_lines`] over a live transcript, reading `FIRST_WINDOW` bytes from the end and
/// more only while that holds too few lines. `None`: no live file (archived, or
/// not written yet), or bytes not all stored yet.
fn live_tail_lines(src: &TranscriptSource<'_>, requested: usize) -> Option<TailLines> {
    tail_lines_with(|window| src.tail(window), requested)
}

/// [`live_tail_lines`] over any way of reading the last `window` bytes.
fn tail_lines_with(mut read: impl FnMut(i64) -> Option<Tail>, requested: usize) -> Option<TailLines> {
    let mut window = FIRST_WINDOW;
    loop {
        let tail = read(window)?;
        if let Some(found) = last_lines(&tail.bytes, requested, tail.size <= window) {
            return Some(found);
        }
        window = window.saturating_mul(4);
    }
}

/// The last `requested` lines of `block_id`'s transcript: a bounded tail read of the
/// live file, else (an archived session) the whole-session read. Blocking.
pub(crate) fn tail_lines(
    mstore: &Arc<Store>,
    filestore: &Arc<FileStore>,
    block_id: &str,
    requested: usize,
) -> Result<TailLines, String> {
    if let Some(found) = live_tail_lines(&TranscriptSource::resolve(filestore, mstore, block_id), requested) {
        return Ok(found);
    }
    let (raw, _) = crate::backend::session_archive::read_session_output(mstore, filestore, block_id)?;
    Ok(last_lines(&raw, requested, true).unwrap_or(TailLines { lines: Vec::new(), truncated: false }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::filestore::{FileMeta, FileOpts};

    const BLOCK: &str = "blk-tail";

    fn store_with(lines: impl Iterator<Item = String>) -> (FileStore, Store) {
        store_with_cache(lines, None)
    }

    /// `cache_cap`: the file store's cache size; a small one makes every read go to the database.
    fn store_with_cache(lines: impl Iterator<Item = String>, cache_cap: Option<usize>) -> (FileStore, Store) {
        let fs = match cache_cap {
            Some(cap) => FileStore::open_in_memory_with_cap(cap).unwrap(),
            None => FileStore::open_in_memory().unwrap(),
        };
        let mstore = Store::open_in_memory().unwrap();
        fs.make_file(BLOCK, "output", FileMeta::default(), FileOpts::default()).unwrap();
        let mut chunk = String::new();
        for l in lines {
            chunk.push_str(&l);
            chunk.push('\n');
            if chunk.len() > 1 << 20 {
                fs.append_lines(BLOCK, "output", chunk.as_bytes()).unwrap();
                chunk.clear();
            }
        }
        if !chunk.is_empty() {
            fs.append_lines(BLOCK, "output", chunk.as_bytes()).unwrap();
        }
        (fs, mstore)
    }

    /// What the route did before: every line of the whole file, then the last ones.
    fn whole_file(fs: &FileStore, requested: usize) -> TailLines {
        let raw = fs.read_file(BLOCK, "output").unwrap().unwrap_or_default();
        last_lines(&raw, requested, true).unwrap()
    }

    fn tail(fs: &FileStore, mstore: &Store, requested: usize) -> TailLines {
        live_tail_lines(&TranscriptSource::resolve_with(fs, None, mstore, BLOCK), requested).expect("a live file")
    }

    fn same(a: &TailLines, b: &TailLines) {
        assert_eq!((&a.lines, a.truncated), (&b.lines, b.truncated));
    }

    #[test]
    fn a_short_transcript_is_returned_whole() {
        let (fs, m) = store_with(["{\"a\":1}", "", "{\"a\":2}"].into_iter().map(String::from));
        let got = tail(&fs, &m, 100);
        assert_eq!(got.lines, ["{\"a\":1}", "{\"a\":2}"]);
        assert!(!got.truncated);
        same(&got, &whole_file(&fs, 100));
    }

    #[test]
    fn a_long_transcript_gives_the_same_last_lines_as_reading_all_of_it() {
        // ~6 MB of 200-byte lines: far past the first window.
        let (fs, m) = store_with((0..30_000).map(|i| format!("{{\"n\":{i},\"pad\":\"{}\"}}", "x".repeat(180))));
        for requested in [1, 5, 100, 500] {
            let (got, want) = (tail(&fs, &m, requested), whole_file(&fs, requested));
            same(&got, &want);
            assert_eq!(got.lines.len(), requested);
            assert!(got.truncated);
            assert!(got.lines.last().unwrap().starts_with("{\"n\":29999,"));
        }
    }

    #[test]
    fn lines_longer_than_the_first_window_widen_it() {
        // 60 lines of 100 KB: the first 256 KiB holds two whole ones.
        let (fs, m) = store_with((0..60).map(|i| format!("{{\"n\":{i},\"pad\":\"{}\"}}", "y".repeat(100_000))));
        let got = tail(&fs, &m, 10);
        same(&got, &whole_file(&fs, 10));
        assert_eq!(got.lines.len(), 10);
        assert!(got.lines[0].starts_with("{\"n\":50,") && got.truncated);
    }

    #[test]
    fn exactly_the_requested_lines_in_a_file_bigger_than_the_window_are_not_truncated() {
        let (fs, m) = store_with((0..50).map(|i| format!("{{\"n\":{i},\"pad\":\"{}\"}}", "z".repeat(20_000))));
        let got = tail(&fs, &m, 50);
        same(&got, &whole_file(&fs, 50));
        assert_eq!((got.lines.len(), got.truncated), (50, false));
    }

    #[test]
    fn a_large_transcript_is_read_through_one_bounded_window() {
        // 6 MB, 100 lines asked for: one read of the first window, nothing more.
        let (fs, m) = store_with((0..30_000).map(|i| format!("{{\"n\":{i},\"pad\":\"{}\"}}", "x".repeat(180))));
        let src = TranscriptSource::resolve_with(&fs, None, &m, BLOCK);
        let (mut windows, mut read) = (Vec::new(), 0usize);
        let got = tail_lines_with(
            |w| {
                windows.push(w);
                let t = src.tail(w);
                read += t.as_ref().map_or(0, |t| t.bytes.len());
                t
            },
            100,
        )
        .unwrap();
        assert_eq!(windows, [FIRST_WINDOW], "no wider read than the first window");
        assert!(read as i64 <= FIRST_WINDOW, "{read} bytes read of a ~6 MB file");
        assert_eq!(got.lines.len(), 100);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn work_in_off_worker_does_not_stop_the_runtime() {
        let t = std::time::Instant::now();
        let work = tokio::spawn(off_worker(|| {
            std::thread::sleep(std::time::Duration::from_millis(400));
            7u32
        }));
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(t.elapsed() < std::time::Duration::from_millis(250), "the runtime was held for {:?}", t.elapsed());
        assert_eq!(work.await.unwrap(), 7);
    }

    #[test]
    fn no_file_means_the_caller_falls_back() {
        let (fs, m) = (FileStore::open_in_memory().unwrap(), Store::open_in_memory().unwrap());
        assert!(live_tail_lines(&TranscriptSource::resolve_with(&fs, None, &m, BLOCK), 10).is_none());
    }

    /// Before and after, on a 96 MB transcript: the cost of one read, and how long a
    /// 2-worker runtime stops answering while four of them run. Not run by default.
    /// `cargo test -p agentmux-srv --bin agentmux-srv transcript_tail::tests::bench -- --ignored --nocapture`
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "a measurement, not a check"]
    async fn bench_a_96_mb_transcript() {
        use std::sync::Arc;
        use std::time::{Duration, Instant};
        // A 1 MB cache, so a read goes to the database as one of a long session's does.
        let lines = (0..96_000).map(|i| format!("{{\"n\":{i},\"pad\":\"{}\"}}", "w".repeat(1000)));
        let (fs, m) = store_with_cache(lines, Some(1 << 20));
        let (fs, m) = (Arc::new(fs), Arc::new(m));
        let old = |fs: &FileStore| whole_file(fs, 100);

        let t = Instant::now();
        for _ in 0..3 {
            old(&fs);
        }
        println!("one read, before (whole file):   {:>7.1} ms (mean of 3)", t.elapsed().as_secs_f64() * 1e3 / 3.0);
        let t = Instant::now();
        let mut n = 0;
        for _ in 0..3 {
            n = tail(&fs, &m, 100).lines.len();
        }
        println!("one read, after  (bounded tail): {:>7.1} ms (mean of 3, {n} lines)", t.elapsed().as_secs_f64() * 1e3 / 3.0);

        // A probe that wants to run every 10 ms; the worst gap is how long the runtime was deaf.
        async fn probe_while(work: impl std::future::Future<Output = ()>) -> u128 {
            let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let flag = stop.clone();
            let probe = tokio::spawn(async move {
                let mut worst = 0u128;
                while !flag.load(std::sync::atomic::Ordering::Relaxed) {
                    let t = Instant::now();
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    worst = worst.max(t.elapsed().as_millis() - 10);
                }
                worst
            });
            tokio::time::sleep(Duration::from_millis(50)).await;
            work.await;
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
            probe.await.unwrap()
        }
        let before = probe_while(async {
            let tasks: Vec<_> = (0..4).map(|_| { let fs = fs.clone(); tokio::spawn(async move { old(&fs); }) }).collect();
            for t in tasks { t.await.unwrap(); }
        })
        .await;
        println!("runtime unresponsive, before (4 reads on the workers):  {before} ms");
        let after = probe_while(async {
            let tasks: Vec<_> = (0..4)
                .map(|_| {
                    let (fs, m) = (fs.clone(), m.clone());
                    tokio::spawn(async move { off_worker(move || { tail_lines_live(&fs, &m); Some(()) }).await; })
                })
                .collect();
            for t in tasks { t.await.unwrap(); }
        })
        .await;
        println!("runtime unresponsive, after  (4 tail reads, off-worker): {after} ms");
    }

    fn tail_lines_live(fs: &FileStore, m: &Store) -> usize {
        tail(fs, m, 100).lines.len()
    }
}
