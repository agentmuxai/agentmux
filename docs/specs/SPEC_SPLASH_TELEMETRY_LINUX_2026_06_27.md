# SPEC: Splash Startup Telemetry — Linux

**Date:** 2026-06-27
**Status:** Ready to implement
**Area:** `agentmux-launcher/src/splash_linux/` · `src/main.rs`
**Depends on:** `SPEC_SPLASH_STARTUP_TELEMETRY_2026_06_25.md` (Windows reference impl)

---

## 1. Background

The Windows splash already shows a live startup timeline — per-phase running clocks, final durations, and a 3-second summary hold. The infrastructure is fully platform-agnostic:

- `startup_events.rs` — `StartupEventSink` / `StartupEvent` types, no platform gate
- Emitters in `main.rs` and `srv_spawner.rs` fire `StageBegin`/`StageEnd` for saga recovery, migrations, and backend spawn on every platform

On Linux the event receiver is dropped immediately (`main.rs:923–924`) before `splash_linux::spawn()` is called. The splash thread never sees any events. This spec wires the receiver through and adds stage rendering to both X11 and Wayland backends.

macOS is a separate follow-up — the AppKit main-thread constraint makes it structurally different.

---

## 2. Current Linux Launch Path

```
main()
├─ splash_linux::spawn()          ← fire-and-forget, detached thread "agentmux-splash"
│  └─ x11::run() OR wayland::run()
│     ├─ renders pulsing brain + footer @ 60 fps
│     └─ polls AGENTMUX_SPLASH_READY_FILE for dismiss signal
│
└─ tokio::runtime.block_on(launcher_main())
   ├─ (startup_sink, startup_rx) created at line 923
   ├─ startup_rx dropped at line 924   ← ALL EVENTS SILENTLY DISCARDED
   ├─ saga recovery → startup_sink.stage_begin/end("saga", ...)
   ├─ srv_spawner: migrations → startup_sink.sub_begin/end(...)
   ├─ srv_spawner: backend → startup_sink.stage_begin/end("backend", ...)
   └─ host spawned → writes AGENTMUX_SPLASH_READY_FILE → splash dismisses
```

The change is purely additive: pass the receiver down the existing call chain.

---

## 3. Changes Required

### 3.1 `agentmux-launcher/src/main.rs`

**Lines 144–151 (Linux block):**

```rust
// BEFORE
if !splash_config::splash_disabled() {
    splash_linux::spawn();
}
tokio::runtime::Runtime::new().unwrap().block_on(launcher_main(...))

// AFTER
let (startup_sink, startup_rx) = startup_events::StartupEventSink::new();
if !splash_config::splash_disabled() {
    splash_linux::spawn(startup_rx);
} else {
    drop(startup_rx);
}
tokio::runtime::Runtime::new().unwrap().block_on(launcher_main(..., startup_sink))
```

Remove the now-redundant second `StartupEventSink::new()` call at line 923 and thread `startup_sink` through `launcher_main`. (The sink is already used by `srv_spawner` and `main.rs` saga recovery — they just receive it as a parameter.)

### 3.2 `agentmux-launcher/src/splash_linux/mod.rs`

Update `spawn()` signature to accept the receiver and pass it to each backend:

```rust
// BEFORE (line 217)
pub fn spawn() {

// AFTER
pub fn spawn(startup_rx: std::sync::mpsc::Receiver<startup_events::StartupEvent>) {
```

Pass `startup_rx` into each backend arm:

```rust
// X11 arm
std::thread::Builder::new()
    .name("agentmux-splash".into())
    .spawn(move || {
        if let Err(e) = x11::run(&ready_file, footer, startup_rx) { ... }
    })

// Wayland arm
std::thread::Builder::new()
    .name("agentmux-splash".into())
    .spawn(move || {
        if let Err(e) = wayland::run(&ready_file, footer, startup_rx) { ... }
    })
```

### 3.3 `agentmux-launcher/src/splash_linux/x11.rs`

**Signature (line 45):**
```rust
// BEFORE
pub(super) fn run(ready_file: &Path, footer: Vec<String>) -> Result<(), Box<dyn Error>>

// AFTER
pub(super) fn run(
    ready_file: &Path,
    footer: Vec<String>,
    startup_rx: std::sync::mpsc::Receiver<startup_events::StartupEvent>,
) -> Result<(), Box<dyn Error>>
```

**Draw loop (lines 117–175) — add `try_recv` drain at the top of each tick:**

```rust
loop {
    // Drain all pending startup events (non-blocking; fires before every frame)
    while let Ok(event) = startup_rx.try_recv() {
        stage_list.apply(event);
    }

    let now = Instant::now();
    // ... existing dismiss / fade / pulse logic ...

    let brain_alpha = pulse_alpha(elapsed.as_secs_f32());
    render_frame(&mut buf, w, h, brain_alpha, window_alpha, radius,
                 true, &footer, &stage_list.lines());

    // ... existing blit / flush / sleep(16ms) ...
}
```

### 3.4 `agentmux-launcher/src/splash_linux/wayland.rs`

**Signature (line 47):**
```rust
pub(super) fn run(
    ready_file: &Path,
    footer: Vec<String>,
    startup_rx: std::sync::mpsc::Receiver<startup_events::StartupEvent>,
) -> Result<(), Box<dyn Error>>
```

Store `startup_rx` in `SplashState`. In `SplashState::draw()` (line 117), drain at the start:

```rust
fn draw(&mut self, qh: &QueueHandle<Self>) {
    // Drain startup events
    while let Ok(event) = self.startup_rx.try_recv() {
        self.stage_list.apply(event);
    }

    // ... existing dismiss / fade / pulse logic ...
    render_frame(canvas, w, h, brain_alpha, window_alpha, CORNER_RADIUS_PX,
                 true, &self.footer, &self.stage_list.lines());
    // ... existing surface commit ...
}
```

---

## 4. Stage List Renderer (`StageList`)

New shared struct in `splash_linux/mod.rs` (used by both backends):

```rust
pub(super) struct StageList {
    stages: Vec<StageEntry>,
}

struct StageEntry {
    label: String,
    started_at: Instant,
    duration_ms: Option<u64>,   // None = still running
    status: StartupStatus,
    subs: Vec<SubEntry>,        // migration sub-items
}

impl StageList {
    pub fn apply(&mut self, event: StartupEvent) {
        match event {
            StartupEvent::StageBegin { label, .. } => {
                self.stages.push(StageEntry::new(label));
            }
            StartupEvent::StageEnd { stage, duration_ms, status, .. } => {
                if let Some(e) = self.stage_mut(&stage) {
                    e.duration_ms = Some(duration_ms);
                    e.status = status;
                }
            }
            StartupEvent::SubBegin { stage, id, label } => {
                if let Some(e) = self.stage_mut(&stage) {
                    e.subs.push(SubEntry { id, label, duration_ms: None });
                }
            }
            StartupEvent::SubEnd { stage, id, duration_ms, .. } => {
                if let Some(e) = self.stage_mut(&stage) {
                    if let Some(s) = e.subs.iter_mut().find(|s| s.id == id) {
                        s.duration_ms = Some(duration_ms);
                    }
                }
            }
        }
    }

    /// Lines to pass to render_frame — e.g. "✓ Backend   42ms", "  Migrations  ..."
    pub fn lines(&self) -> Vec<String> { ... }
}
```

### 4.1 Line format

```
✓ Saga recovery          12ms
✓ Migrations             341ms
  ↳ 0009_cron_schema      8ms
  ↳ 0010_identity_dedup   3ms
● Backend startup        ...     ← running: show elapsed, no final
```

Symbols: `✓` (ok), `✗` (error/warn), `●` (running). All rendered via the existing `splash_text` software blitter. Same glyph set the footer already uses.

### 4.2 Layout

Stage list sits between the brain logo and the footer (two fixed lines). Max visible lines: 6 (3 stages + 3 sub-items). If more subs exist, show only the last 2 with "↳ …" ellipsis above.

---

## 5. `render_frame` Extension

`render_frame()` in `splash_linux/mod.rs` currently takes `footer: &[String]`. Extend to also accept `stages: &[String]` (or a combined `body: &[String]`):

```rust
fn render_frame(
    buf: &mut [u8], w: i32, h: i32,
    brain_alpha: f32, window_alpha: f32,
    corner_radius: i32, has_compositor: bool,
    footer: &[String],
    stages: &[String],    // NEW — empty slice = current behavior
)
```

Stage lines drawn above the footer separator, using the same text blitter, slightly smaller font weight (or same weight is fine).

---

## 6. Summary Hold

After `AGENTMUX_SPLASH_READY_FILE` appears, **hold for 1.5 seconds** before beginning fade-out. This lets the user see the completed timeline.

```rust
// Current dismiss condition (x11.rs line ~130):
let dismiss = ready_file.exists() && elapsed >= hold_duration || elapsed >= DISMISS_TIMEOUT;

// Add configurable hold:
const SUMMARY_HOLD_MS: u64 = 1500;
let hold_after_ready = Duration::from_millis(
    std::env::var("AGENTMUX_SPLASH_HOLD_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(SUMMARY_HOLD_MS)
);
```

`AGENTMUX_SPLASH_HOLD_MS=0` = CI mode (dismiss immediately, same as today).

---

## 7. Selftest Path

`splash_linux::splash_selftest()` (invoked via `--splash-selftest`) should also accept and populate fake events so the rendered timeline can be eyeballed without a full build:

```rust
fn splash_selftest() {
    let (sink, rx) = startup_events::StartupEventSink::new();
    // Fire fake events in a background thread
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        sink.stage_begin("saga", "Saga recovery");
        std::thread::sleep(Duration::from_millis(300));
        sink.stage_end("saga", 300, StartupStatus::Ok, None);
        // ... etc
    });
    splash_linux::spawn(rx);
    std::thread::sleep(Duration::from_secs(5));
}
```

---

## 8. What Is NOT in Scope

- **macOS** — `splash_mac.rs` runs on the AppKit main thread (CoreFoundation runloop); wiring the receiver requires coordinating with `run_until_dismissed()`. Separate spec.
- **CEF first-paint event** (S5) — the host sends `AGENTMUX_SPLASH_READY_FILE`; no additional IPC needed for the splash to know CEF is done.
- **Frontend events** (S6) — out of scope for this spec; those are displayed in the statusbar after the splash is gone.
- **Pruner integration** (S3) — `SPEC_LOCAL_CHANNEL_PRUNER_2026_06_25.md` owns that stage.

---

## 9. Acceptance

- `./agentmux-launcher --splash-selftest` shows animated stage list on both X11 and Wayland
- Cold start: saga, migrations, and backend stages appear as they happen, with running clocks
- After first paint: 1.5-second summary hold, then 200ms fade-out
- `AGENTMUX_SPLASH_HOLD_MS=0` skips the hold (CI)
- No regression on the static brain/footer path when no events are received (empty `stage_list`)
- Windows splash unaffected (no shared code)

---

## 10. Files Touched

| File | Change |
|---|---|
| `agentmux-launcher/src/main.rs` | Create sink before `splash_linux::spawn()`; pass rx; remove redundant second sink creation at line 923 |
| `agentmux-launcher/src/splash_linux/mod.rs` | `spawn(rx)` signature; new `StageList` struct; `render_frame` extra `stages` param |
| `agentmux-launcher/src/splash_linux/x11.rs` | `run(…, rx)` signature; `try_recv` drain in 16ms loop |
| `agentmux-launcher/src/splash_linux/wayland.rs` | `run(…, rx)` signature; `startup_rx` on `SplashState`; drain in `draw()` |
