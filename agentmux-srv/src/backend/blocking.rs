// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Running a blocking section from code that may be on an async worker.
//!
//! A synchronous call that can take seconds (SQLite with a busy timeout, file
//! I/O, per-process OS queries) must not run inline on a Tokio worker. It is
//! not only that one worker that stops: the worker that happens to be driving
//! the I/O driver stops polling it, and no other worker takes the driver over
//! until that one parks again. For the length of the call no socket is
//! serviced — the HTTP health endpoint included — while timers and
//! already-queued tasks keep running, so the server looks alive in its logs
//! and answers nothing. That is what the launcher's liveness probe saw before
//! it recycled srv. See
//! `docs/analysis/ANALYSIS_SRV_HTTP_STALL_IO_DRIVER_STARVATION_2026_09_26.md`.

/// Run `f`, handing this worker's other tasks (and the I/O driver, if it holds
/// it) to another thread first when called on a multi-thread runtime worker.
///
/// Synchronous and order-preserving, unlike `spawn_blocking`: the caller
/// continues only after `f` returns, so a write that must land before the
/// next one still does.
///
/// - Multi-thread runtime worker: `block_in_place`.
/// - Blocking-pool thread, or no runtime: nothing to hand off — runs `f`.
/// - Current-thread runtime (most `#[tokio::test]`s): `block_in_place` panics
///   there and there is no other worker to hand off to — runs `f`.
pub fn off_async_worker<R>(f: impl FnOnce() -> R) -> R {
    let on_multi_thread = tokio::runtime::Handle::try_current()
        .is_ok_and(|h| h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread);
    if on_multi_thread {
        tokio::task::block_in_place(f)
    } else {
        f()
    }
}

#[cfg(test)]
mod tests {
    use super::off_async_worker;

    #[test]
    fn runs_without_a_runtime() {
        assert_eq!(off_async_worker(|| 7), 7);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn does_not_panic_on_a_current_thread_runtime() {
        // block_in_place would panic here; the helper must fall back.
        assert_eq!(off_async_worker(|| 7), 7);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn runs_on_a_multi_thread_worker() {
        assert_eq!(off_async_worker(|| 7), 7);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn runs_on_a_blocking_pool_thread() {
        let v = tokio::task::spawn_blocking(|| off_async_worker(|| 7)).await.unwrap();
        assert_eq!(v, 7);
    }

    /// The point of the helper: while a worker sits in a long blocking section,
    /// the runtime still services I/O. One worker makes this deterministic —
    /// with the section inline, that worker IS the I/O driver and nothing else
    /// can poll it, so the round-trip below would wait the full 1.5s;
    /// `block_in_place` hands its core to another thread first.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn io_is_still_serviced_while_a_worker_blocks() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let (mut s, _) = listener.accept().await.unwrap();
                tokio::spawn(async move {
                    let mut b = [0u8; 4];
                    let _ = s.read(&mut b).await;
                    let _ = s.write_all(b"pong").await;
                });
            }
        });
        let (entered_tx, entered_rx) = std::sync::mpsc::channel::<()>();
        let blocker = tokio::spawn(async move {
            off_async_worker(move || {
                entered_tx.send(()).unwrap();
                std::thread::sleep(std::time::Duration::from_millis(1500));
            });
        });
        // Start the round-trip only once the worker is inside the section.
        tokio::task::spawn_blocking(move || entered_rx.recv().unwrap()).await.unwrap();
        let started = std::time::Instant::now();
        let mut c = tokio::net::TcpStream::connect(addr).await.unwrap();
        c.write_all(b"ping").await.unwrap();
        let mut b = [0u8; 4];
        c.read_exact(&mut b).await.unwrap();
        assert_eq!(&b, b"pong");
        assert!(
            started.elapsed() < std::time::Duration::from_millis(800),
            "round-trip took {:?} while another worker was blocked",
            started.elapsed()
        );
        blocker.await.unwrap();
    }
}
