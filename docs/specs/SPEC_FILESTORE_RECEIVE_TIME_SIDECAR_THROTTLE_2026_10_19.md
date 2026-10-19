# Spec: the receive-time sidecar is written at most once a second per file

**Date:** 2026-10-19
**Author:** Maricon@charlie
**Type:** Spec (a write-volume change; what readers see differs by under a second)
**Status:** active — the change and its tests are in the PR that adds this file (#5171); remaining: Posa's A/B on the Windows machine that stalls.
**Issue:** #5141. Measured in `docs/reports/REPORT_FILESTORE_WRITE_VOLUME_2026_10_19.md`.
**Platform:** all; measured on Linux.

## 1. The problem

Every line of an agent's output is followed, in the same transaction, by a record in the sidecar `output.tsidx` (`{"off":<byte offset of the batch>,"ms":<unix ms>}`, about 30 bytes), in the block's store and in the global transcript store. A 30-byte append costs a rewrite of the sidecar's last part, up to 64 KB, because `append_in_tx` replaces the whole part. The sidecar's part grows slowly, so it is half full on average: each record costs about as much as the line itself. Of the four part rewrites a transcript line pays, **two are the sidecar's**.

Lines arrive in bursts of hundreds a second, and the time of a replayed line has no use below a second ("batch granularity is deliberate", `file_ops.rs`).

## 2. The change

`FileStore::append_lines_stamped` writes a sidecar record only when this store has not written one for that file within the last second (`STAMP_MIN_INTERVAL_MS` = 1000), or has not written one since it started, or the clock went backwards. The first line after a pause is always stamped, and so is the first line of a file (offset 0) and any line whose sidecar is missing or empty, which the transaction checks with one row lookup: a transcript and its sidecar deleted or replaced together (a session archive) start again from a record at offset 0 even inside the second. The in-memory decision is per store and per `(zone, sidecar)`; it is recorded only after the transaction commits, so a failed append does not use up the second. Nothing is stored differently: the sidecar is the same format, with fewer records.

A line without a record of its own takes **the newest record at or before it**. That is how every reader already looks a line up (`stamps_for`, used by `blockfile:read_range`, `agent:open_tail` and srv's history record), so it needs no reader change. A line's time is therefore at most one second earlier than when it arrived.

## 3. Who sees a stamp

A stamp is only the **fallback** time of a replayed line that has no timestamp of its own:

- `parseHistoryLines.ts`: a node's own wire timestamp always wins; the stamp fills timestamp-less nodes. They show as message times (hover) and feed the day dividers in the history view (`day-dividers.ts`).
- srv's history record (`history/record.rs`): the time of each message and the first and last times of a session, in Open Agent History and the session list.
- Nothing exports a stamp.

The error is under one second. It could only move a message across a calendar-day boundary if the message arrived in the last second before midnight.

## 4. Measurement

Loap-3's memory lab, 12 agents, 10 rounds, seed 1, Linux, one pinned portable (main at 056a94157), only the srv executable differing; 3 runs per side, interleaved. Medians.

| | baseline | with the throttle | |
|---|---:|---:|---:|
| srv bytes written | 14505 MB (14474–14514) | 7812 MB (7791–7819) | **−46.1%** |
| bytes that reached storage | 8623 MB | 5423 MB | −37.1% |
| srv CPU | 104.8 s | 71.5 s | **−31.8%** |
| round time | 13.14 s | 12.88 s | −1.9% |
| srv memory | 34.8 MB | 35.1 MB | +0.9% |
| app total memory | 902 MB | 936 MB | +3.7% (renderers 597 → 626 MB, ranges not overlapping at 3 runs; other, unrelated conditions in the same lab moved the renderers by −3.2% to +2.8%) |

With the lab's `--disk-load 50` (a writer streaming 50 MB/s to the same volume), 2 runs per side: srv bytes written 14468 → 7803 MB (−46.1%), to storage −37.1%, srv CPU 107.6 → 74.3 s (−30.9%), round time 12.4 s → 13.1 s (+6%, with the two sides' ranges overlapping: 11.5–13.2 s against 13.1 s).

The store lock never reached 500 ms on this machine, so the stall itself is for the Windows machine (Posa).

## 5. Tests

`tests_stamp_throttle.rs`: records at 0, 1000 and 2001 ms for lines at 0, 10, 500, 999, 1000, 1500, 1999, 2001 and 2002 ms; the first line and the first after a pause are stamped; a backwards clock is stamped; a failed append does not use up the second; a transcript recreated inside the second is stamped from its first line, and so is a line whose sidecar was emptied; each sidecar has its own clock; 3000 lines over ten seconds leave at most eleven records, ascending in offset and time. In `open_tail_tests`, a sidecar longer than any fixed window still stamps the oldest line of a long slice. Three older tests that asserted a record per append (`tsidx_stamped_*`, `tsidx_offsets_match_actual_append_positions`, `a_stamped_append_writes_the_line_and_its_stamp_together`) were updated to what is now true.

## 6. Limits

- The memory is per store and per process: a restarted srv stamps each file's next line.
- In a continuous burst a line's time is that of the last record, up to a second older than the line; the first line after a pause is exact.
