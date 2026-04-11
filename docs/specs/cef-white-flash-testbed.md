# CEF White Flash Testbed — Spec

## Goal

Build a minimal standalone CEF app (no sidecar, no frontend, no IPC) that reproduces the white flash on startup. Use it to isolate the exact cause and test fixes without risk to the main AgentMux build.

## Why

Every attempt to fix the white flash in AgentMux has broken CEF initialization (GPU process dies, singleton fallback to Chrome, etc.). We need a safe sandbox to iterate fast without 10 version bumps per attempt.

## Architecture

```
cef-testbed/
├── Cargo.toml          # depends on cef crate only
├── src/
│   └── main.rs         # ~150 lines, minimal CEF app
├── runtime/            # CEF DLLs + resources (symlinked from dist/cef)
└── test-page.html      # dark background, simple content
```

Single binary. Loads a local HTML file with `background: #222`. No sidecar, no IPC server, no frontend build. Just CEF + a window + a page.

## Test Matrix

Each test variant is a small code change in main.rs. Run each, observe with eyes + screen recording (Win+G):

| # | Variant | What it tests |
|---|---------|--------------|
| 1 | Baseline: CEF Views, default settings | Reproduce the flash |
| 2 | CEF Views + `background_color: 0xFF222222` | Does the setting help at all? |
| 3 | Native window + WS_VISIBLE | Compare native vs Views |
| 4 | Native window, NO WS_VISIBLE, ShowWindow in on_load_end | Does hidden→show flash? |
| 5 | Native window, NO WS_VISIBLE, ShowWindow after rAF IPC | Does waiting for paint help? |
| 6 | Native window + WS_VISIBLE + dark class brush on WM_ERASEBKGND | Does GDI brush help? |
| 7 | `--disable-gpu` flag | Is it GPU compositor delay? |
| 8 | `--disable-gpu-compositing` flag | Narrower GPU test |

## Implementation

### Cargo.toml

```toml
[package]
name = "cef-testbed"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "cef-testbed"
path = "src/main.rs"

[dependencies]
cef = { version = "146", default-features = false, features = ["build-util"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

[target.'cfg(target_os = "windows")'.dependencies]
windows-sys = { version = "0.59", features = [
    "Win32_Foundation",
    "Win32_UI_WindowsAndMessaging",
    "Win32_Graphics_Dwm",
    "Win32_Graphics_Gdi",
    "Win32_System_LibraryLoader",
    "Win32_UI_Controls",
] }
```

### test-page.html

```html
<!doctype html>
<html>
<head>
  <style>
    html, body { margin: 0; background: #222; color: #f7f7f7; }
    body { display: flex; align-items: center; justify-content: center; height: 100vh; }
    h1 { font-family: sans-serif; font-size: 48px; opacity: 0.8; }
  </style>
</head>
<body>
  <h1>CEF Testbed</h1>
  <script>
    // Log paint timing
    new PerformanceObserver(list => {
      for (const entry of list.getEntries()) {
        console.log(`[paint] ${entry.name}: ${entry.startTime.toFixed(1)}ms`);
      }
    }).observe({ entryTypes: ['paint'] });
  </script>
</body>
</html>
```

### main.rs (Baseline — Variant 1)

Minimal CEF Views app. Each variant is a small diff from this baseline.

```rust
// cef-testbed: minimal CEF app to reproduce and fix white flash.
// Toggle variants via --variant=N command line flag.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use cef::*;
use std::cell::RefCell;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_writer(std::io::stderr)
        .init();

    tracing::info!("CEF testbed starting");

    let args = cef::args::Args::new();
    let cmd_line = args.as_cmd_line().expect("Failed to parse args");

    // Subprocess check
    let type_switch = CefString::from("type");
    let is_browser = cmd_line.has_switch(Some(&type_switch)) != 1;
    let ret = execute_process(Some(args.as_main_args()), None, std::ptr::null_mut());
    if !is_browser {
        std::process::exit(ret);
    }
    assert_eq!(ret, -1);

    // Parse variant number
    let variant_switch = CefString::from("variant");
    let variant: u32 = if cmd_line.has_switch(Some(&variant_switch)) != 0 {
        CefString::from(&cmd_line.switch_value(Some(&variant_switch)))
            .to_string().parse().unwrap_or(1)
    } else { 1 };
    tracing::info!("Running variant {}", variant);

    // Resolve test page URL
    let exe_dir = std::env::current_exe().ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_default();
    let page_path = exe_dir.join("test-page.html");
    let url_str = format!("file:///{}", page_path.to_str().unwrap().replace('\\', "/"));

    // CEF App
    let mut app = TestApp::new(variant, url_str);

    let settings = Settings {
        no_sandbox: 1,
        background_color: if variant >= 2 { 0xFF222222 } else { 0 },
        root_cache_path: CefString::from(
            exe_dir.join("cef-cache").to_str().unwrap_or("")
        ),
        resources_dir_path: CefString::from(exe_dir.to_str().unwrap_or("")),
        locales_dir_path: CefString::from(
            exe_dir.join("locales").to_str().unwrap_or("")
        ),
        browser_subprocess_path: CefString::from(
            std::env::current_exe().unwrap().to_str().unwrap_or("")
        ),
        ..Default::default()
    };

    let init = initialize(
        Some(args.as_main_args()),
        Some(&settings),
        Some(&mut app),
        std::ptr::null_mut(),
    );
    if init != 1 {
        tracing::error!("CEF initialize failed (returned {})", init);
        std::process::exit(1);
    }

    tracing::info!("Entering message loop");
    run_message_loop();
    shutdown();
}

// ---- App + BrowserProcessHandler ----

wrap_app! {
    pub struct TestApp {
        variant: u32,
        url: String,
    }
    impl App {
        fn on_before_command_line_processing(
            &self, _process_type: Option<&CefString>,
            command_line: Option<&mut CommandLine>,
        ) {
            if let Some(cmd) = command_line {
                let key = CefString::from("disable-features");
                let val = CefString::from("CalculateNativeWinOcclusion");
                cmd.append_switch_with_value(Some(&key), Some(&val));

                if self.variant >= 2 {
                    let bg_key = CefString::from("background-color");
                    let bg_val = CefString::from("ff222222");
                    cmd.append_switch_with_value(Some(&bg_key), Some(&bg_val));
                }
                if self.variant == 7 {
                    cmd.append_switch(Some(&CefString::from("disable-gpu")));
                }
                if self.variant == 8 {
                    cmd.append_switch(Some(&CefString::from("disable-gpu-compositing")));
                }
            }
        }
        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(TestBPH::new(self.variant, self.url.clone()))
        }
    }
}

wrap_browser_process_handler! {
    pub struct TestBPH {
        variant: u32,
        url: String,
    }
    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            let url = CefString::from(self.url.as_str());
            let settings = BrowserSettings {
                background_color: if self.variant >= 2 { 0xFF222222 } else { 0 },
                ..Default::default()
            };

            // TODO: Branch on self.variant for Views vs Native mode
            // For now, always use CEF Views (variant 1-2 baseline)

            let mut client = TestClient::new(self.variant);
            let mut bv_delegate = TestBVDelegate::new();
            let browser_view = browser_view_create(
                client.as_mut(), Some(&url), Some(&settings),
                None, None, Some(&mut bv_delegate),
            );
            let mut wd = TestWindowDelegate::new(
                RefCell::new(browser_view), self.variant,
            );
            window_create_top_level(Some(&mut wd));
        }
    }
}

// ---- Window Delegate ----

wrap_window_delegate! {
    pub struct TestWindowDelegate {
        browser_view: RefCell<Option<BrowserView>>,
        variant: u32,
    }
    impl ViewDelegate {
        fn preferred_size(&self, _view: Option<&mut View>) -> Size {
            Size { width: 800, height: 600 }
        }
    }
    impl PanelDelegate {}
    impl WindowDelegate {
        fn on_window_created(&self, window: Option<&mut Window>) {
            let bv = self.browser_view.borrow();
            let (Some(window), Some(bv)) = (window, bv.as_ref()) else { return };
            let mut view = View::from(bv);
            window.add_child_view(Some(&mut view));
            tracing::info!("[variant {}] on_window_created", self.variant);
            window.show();
        }
        fn on_window_destroyed(&self, _window: Option<&mut Window>) {
            *self.browser_view.borrow_mut() = None;
        }
        fn can_close(&self, _window: Option<&mut Window>) -> i32 { 1 }
        fn initial_show_state(&self, _w: Option<&mut Window>) -> ShowState {
            ShowState::NORMAL
        }
        fn is_frameless(&self, _w: Option<&mut Window>) -> i32 { 0 }
        fn can_resize(&self, _w: Option<&mut Window>) -> i32 { 1 }
        fn window_runtime_style(&self) -> RuntimeStyle { RuntimeStyle::ALLOY }
    }
}

wrap_browser_view_delegate! {
    pub struct TestBVDelegate {}
    impl ViewDelegate {}
    impl BrowserViewDelegate {
        fn browser_runtime_style(&self) -> RuntimeStyle { RuntimeStyle::ALLOY }
    }
}

// ---- Client + Handlers ----

wrap_client! {
    pub struct TestClient { variant: u32 }
    impl Client {
        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(TestLifeSpan::new(self.variant))
        }
        fn load_handler(&self) -> Option<LoadHandler> {
            Some(TestLoadHandler::new(self.variant))
        }
    }
}

wrap_life_span_handler! {
    struct TestLifeSpan { variant: u32 }
    impl LifeSpanHandler {
        fn on_after_created(&self, _browser: Option<&mut Browser>) {
            tracing::info!("[variant {}] on_after_created", self.variant);
        }
        fn on_before_close(&self, _browser: Option<&mut Browser>) {
            quit_message_loop();
        }
    }
}

wrap_load_handler! {
    struct TestLoadHandler { variant: u32 }
    impl LoadHandler {
        fn on_load_end(
            &self, _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>, _status: i32,
        ) {
            let Some(frame) = frame else { return };
            if frame.is_main() != 1 { return; }
            tracing::info!("[variant {}] on_load_end (main frame)", self.variant);
        }
    }
}
```

## How to Use

```bash
# Build
cargo build -p cef-testbed --release

# Copy CEF DLLs (one-time)
cp dist/cef/*.dll target/release/
cp dist/cef/*.pak target/release/
cp dist/cef/*.bin target/release/
cp -r dist/cef/locales target/release/
cp cef-testbed/test-page.html target/release/

# Run variants
target/release/cef-testbed.exe                  # Variant 1: baseline
target/release/cef-testbed.exe --variant=2      # + background_color
target/release/cef-testbed.exe --variant=7      # + disable-gpu
```

## Recording

Use Win+G (Xbox Game Bar) or OBS to record each variant at 60fps. Frame-step through the recording to see exactly:
- Which frame shows white
- How many frames of white before dark content
- Whether `background_color` changes the flash color

## Success Criteria

Find a variant where the window appears with dark content from the very first visible frame, without breaking CEF initialization.
