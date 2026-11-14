# Runbook: the app freezes for a moment (the host UI thread stalls, Windows)

**Status:** living. Written 2026-11-14 with the stall watch (`crates/cef/src/ui_tasks/stall_watch.rs`, task #47585). Add to it when a step turns out to be missing.

For a report like "a click took seconds", "the browser freezes during navigation", or slow `[perf] ipc ...` lines in the host log with nothing to explain them.

## 1. What the host UI thread is, and what a stall looks like

One thread of the host runs the window, every browser pane's callbacks, the input owner and the tasks other threads post to it. While it is blocked:

- a click or a key is not handled until it is free again,
- a navigation does not start,
- every IPC call that needs it waits, so the frontend logs `[perf] ipc <name> <N>ms` with large numbers for calls that do almost nothing (`browser_panes_set_rects`, `input_select`, `browser_panes_set_overlay_clip`).

The host log says so directly:

```
[ui-stall] the UI thread took no message for 2191 ms
```

A watcher thread posts a message to the UI thread every 200 ms, and a hook on that thread's queue notes when it is taken off. 250 ms or more without that is logged, at most twice a second. It is always on, in every build, and it changes nothing the UI thread does.

## 2. Find what it is stuck in

Start the app with `AGENTMUX_UI_STALL_STACKS=1`. During a stall the watcher then stops the UI thread for a moment, reads its call stack and logs it, up to six times per stall (the offsets here are made up):

```
[ui-stall] stack at 259 ms: ntdll.dll+0xa1234 < KERNELBASE.dll+0x5a312 < agentmux-cef.dll+0x94cd52 < ... < libcef.dll+0x8e2af30
```

Frames are innermost first, as `module+offset`. The offset is into the module's image, so it does not change between runs of one build.

- The first frames usually name the kind of wait on their own: look the system modules' offsets up with a debugger (`cdb -z <dll> -c "ln <module>+0x<offset>; q"` resolves them from Microsoft's symbols). `CreateProcessW` is a process being started, `NtWaitForSingleObject` a lock or an event, `NtUserMessageCall` a message sent to a window on another thread.
- `agentmux-cef.dll` frames are the host's own code. Resolve them against the build's symbol file. A source-tree build writes one symbol file for the host library and one for the host's own executable under the same name (`target/release/deps/agentmux_cef.pdb`), and the second overwrites the first; when the debugger says "mismatched pdb", resolve against `target/release/agentmux-cef.exe` instead (it holds the same code at other addresses: find the bytes around the frame's offset in it).
- `libcef.dll` frames between them mean the browser engine called into the host: the host frames above them are a handler of ours.

Stack sampling is off by default because stopping a thread to read its stack is not free of risk: if that thread holds the loader's lock at that moment the reader waits for it. A guard resumes the thread after 150 ms whatever happens and the sample is dropped, so the worst case is one missing sample.

## 3. What it has found

- **A dev build froze for seconds at every page load (task #47585, 2026-11-14).** The host asked "is this a source-tree build?" on the UI thread at every load end and every pane navigation check, and the answer was built by starting `git rev-parse` and waiting for it. An installed build never reached that code. Six of six samples showed `CreateProcessW` under `agentmux_common::runtime_mode::is_dev_self`. Fixed by answering from the binary's path alone, once.

## 4. Rules this leaves for code that runs on the UI thread

- No process starts, no waits for another process, no network and no file reads that can be slow, in a handler the browser engine calls or in a posted UI task.
- A value that cannot change while the process runs is read once.
- A test for a path that must stay free of process starts can count them: `agentmux_common::runtime_mode::git_branch_lookups()` counts the `git` starts made on the calling thread.
