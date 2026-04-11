# CEF (Chromium Embedded Framework) Architecture — Technical Reference

## Table of Contents

1. [CEF's Relationship to Chromium](#1-cefs-relationship-to-chromium)
2. [Multi-Process Architecture](#2-multi-process-architecture)
3. [The CEF C API Layer](#3-the-cef-c-api-layer)
4. [Language Bindings Ecosystem](#4-language-bindings-ecosystem)
5. [CEF Views vs Native Window](#5-cef-views-vs-native-window)
6. [The Handler/Callback Model](#6-the-handlercallback-model)
7. [IPC and Message Routing](#7-ipc-and-message-routing)
8. [Content/Rendering Pipeline](#8-contentrendering-pipeline)
9. [CEF Build System](#9-cef-build-system)
10. [How CEF Differs from Electron](#10-how-cef-differs-from-electron)
11. [Sandboxing](#11-sandboxing)
12. [The GPU Process in Detail](#12-the-gpu-process-in-detail)

---

## 1. CEF's Relationship to Chromium

### What CEF Is

CEF is an open-source framework built on top of the Chromium project. While Chromium is a full browser application, CEF is a library for **embedding** a Chromium-based browser into other applications. It enables developers to use HTML, CSS, and JavaScript to create application UI without shipping an entire browser.

CEF3 (the current generation) is built on the **Chromium Content API** — the same API layer that Chrome itself uses. The Content API provides the multi-process browser architecture, the Blink rendering engine, and the V8 JavaScript engine. CEF wraps this in a stable, documented API.

### What CEF Exposes vs Hides

CEF acts as a **stable abstraction layer** over Chromium's internal APIs:

**Hides:**
- Blink and Content API implementation details (which are unstable and change frequently)
- The complexity of Chromium's multi-process management
- Low-level threading and IPC plumbing
- Chromium build system complexity

**Exposes:**
- Browser creation and lifecycle management
- JavaScript execution and binding
- Network layer (custom schemes, request interception, cookie management, auth handling)
- Rendering control (on-screen, off-screen/windowless)
- Custom plugins, protocols, JavaScript objects and extensions
- DevTools remote debugging
- Drag-and-drop, printing, downloads, context menus
- Process-level callbacks (browser process, renderer process)

The Blink and Content APIs are not stable and many features require additional implementation. CEF provides these implementations and hides the complexity behind a versioned API surface.

### Version Mapping

Since 2019, CEF uses the format:

```
X.Y.Z+gHHHHHHH+chromium-A.B.C.D
```

| Component | Meaning |
|-----------|---------|
| `X` | Major version = Chromium milestone (e.g., CEF 146 = Chromium 146) |
| `Y` | Increments when C/C++ API changes; resets to 0 per branch |
| `Z` | Bugfix counter; resets when Y increments |
| `gHHHHHHH` | 7-character Git commit hash |
| `A.B.C.D` | Full Chromium version |

**Legacy format (pre-2019):** `X.YYYY.A.gHHHHHHH` where X was always 3, YYYY was the Chromium branch number, and A was the commit count.

### Branch Versioning

CEF maintains branches that track Chromium release milestones:

- **Master branch** — Tracks Chromium master. Not for production.
- **Release branches** — Created per Chromium milestone (MXX). Frozen APIs with only security/bug fixes. Example: Branch 7680 = Chromium 146 (Stable).
- **LTS (Long-Term Support)** — Every 6th branch starting with M138 receives extended support (~8 additional months of security fixes after exiting stable).
- **Legacy** — Old branches (back to 2704) available but unsupported.

Current branch-to-Chromium mapping (as of early 2026):

| Branch | Chromium | Channel |
|--------|----------|---------|
| 7727 | 147 | Beta |
| 7680 | 146 | Stable |
| 7204 | 138 | LTS |

The `CHROMIUM_BUILD_COMPATIBILITY.txt` file in the CEF repo specifies the exact Chromium version for any given branch/commit. The generated `cef_version.h` header contains all version information at compile time.

---

## 2. Multi-Process Architecture

CEF3 inherits Chromium's multi-process model. Each process type runs in an isolated OS process for security, stability, and parallelism.

### Process Types

```
+--------------------------------------------------------------------+
|                        BROWSER PROCESS                              |
|  (Host application main process)                                    |
|                                                                     |
|  - Window creation and management                                   |
|  - UI thread (main event loop)                                      |
|  - IO thread (IPC, network events)                                  |
|  - FILE threads (filesystem operations)                             |
|  - Network stack (via Network Service)                              |
|  - Cookie management, auth, certificate handling                    |
|  - CefApp::GetBrowserProcessHandler() callbacks                     |
|  - CefClient handler callbacks                                      |
+--------------------------------------------------------------------+
       |  IPC (Mojo / Legacy IPC)       |  IPC            |  IPC
       v                                v                 v
+------------------+  +------------------+  +------------------+
| RENDERER PROC 1  |  | RENDERER PROC 2  |  | RENDERER PROC N  |
| (per origin)     |  | (per origin)     |  |                  |
|                  |  |                  |  |                  |
| - Blink (HTML/   |  | - Blink          |  | - Blink          |
|   CSS layout)    |  |                  |  |                  |
| - V8 (JavaScript)|  | - V8             |  | - V8             |
| - DOM access     |  |                  |  |                  |
| - JS bindings    |  |                  |  |                  |
| - TID_RENDERER   |  |                  |  |                  |
|   thread         |  |                  |  |                  |
+------------------+  +------------------+  +------------------+
       |
       | GPU command buffer (shared memory)
       v
+--------------------------------------------------------------------+
|                         GPU PROCESS                                 |
|                                                                     |
|  - Accelerated compositing (Viz display compositor)                 |
|  - ANGLE (translates GL ES -> D3D/Vulkan/Metal)                    |
|  - Rasterization of display lists into GPU texture tiles            |
|  - Frame aggregation from all processes                             |
|  - Final draw to screen                                             |
|  - WebGL execution                                                  |
+--------------------------------------------------------------------+

+---------------------+  +---------------------+
| UTILITY PROCESS(ES) |  | NETWORK SERVICE     |
| - Short-lived tasks |  | - HTTP/HTTPS stack  |
| - Data decoding     |  | - DNS resolution    |
| - Audio service     |  | - Cookie storage    |
+---------------------+  +---------------------+
```

### What Runs Where

**Browser Process (main):**
- Window creation and native UI
- All `CefClient` handler callbacks (DisplayHandler, LifeSpanHandler, RequestHandler, etc.)
- `CefApp::GetBrowserProcessHandler()` callbacks
- Network request management
- `CefBrowserHost` operations
- Threads: TID_UI (main), TID_IO (IPC/network), TID_FILE_* (filesystem)

**Renderer Process(es):**
- Blink rendering engine (HTML parsing, CSS layout, painting)
- V8 JavaScript engine execution
- DOM access and manipulation
- Custom JavaScript bindings
- `CefApp::GetRenderProcessHandler()` callbacks
- One renderer per unique origin (scheme + domain) by default
- Thread: TID_RENDERER (main thread; all Blink/V8 must run here)

**GPU Process:**
- Accelerated compositing via Viz display compositor
- GPU rasterization via Skia/Ganesh
- ANGLE translation (GL ES -> platform-native API)
- WebGL rendering
- Frame aggregation from all render processes + browser process
- Final draw to screen

**Utility Processes:**
- Short-lived tasks (data decoding, file operations)
- Audio service, storage service
- Sandboxed for security

### Subprocess Spawning

CEF supports two models:

**1. Single-executable model (Windows/Linux):**
The main application executable is re-spawned with `--type=renderer`, `--type=gpu-process`, etc. The entry point detects this:

```cpp
int main(int argc, char* argv[]) {
    CefMainArgs main_args(argc, argv);
    
    // If this is a subprocess, CefExecuteProcess blocks until exit
    int exit_code = CefExecuteProcess(main_args, app.get(), nullptr);
    if (exit_code >= 0)
        return exit_code;  // Was a subprocess
    
    // This is the browser process
    CefInitialize(main_args, settings, app.get(), nullptr);
    CefRunMessageLoop();
    CefShutdown();
}
```

**2. Separate-executable model (all platforms, required on macOS):**
A small helper binary handles subprocesses. Configured via `CefSettings.browser_subprocess_path`. The helper binary only calls `CefExecuteProcess()`. This is preferred when the main application is large or has long startup time.

**Linux Zygote:**
On Linux, Chromium uses a Zygote process (spawned at startup) that forks to create renderer processes. There are actually two zygotes — one sandboxed (for renderers), one unsandboxed (for processes needing system access). The Zygote is triggered by `--type=zygote`.

### IPC Mechanisms

Processes communicate via:

1. **Mojo** — The modern IPC system. Uses Mojom IDL to define interfaces. Message pipes provide bidirectional, typed communication. ~3x faster than legacy IPC with 1/3 less context switching. Generates bindings for C++, Java, and JavaScript.

2. **Legacy IPC** — Older system using `IPC::Channel`. Still present but being phased out in favor of Mojo.

3. **Shared Memory** — Used for bulk data (textures, buffers, command buffers). The GPU command buffer is a shared-memory ring buffer.

4. **CefProcessMessage** — CEF's high-level abstraction over IPC for application-level browser-to-renderer messaging.

---

## 3. The CEF C API Layer

### Architecture

CEF is C++ internally but exposes a C API through `libcef` (the shared library). A static C++ wrapper library (`libcef_dll_wrapper`) sits on top:

```
+-------------------------------------------+
|          Your Application (C++)           |
+-------------------------------------------+
|       libcef_dll_wrapper (static lib)     |
|  C++ classes: CefBrowser, CefClient, etc. |
|  CefRefPtr<T> smart pointers              |
+-------------------------------------------+
|          libcef C API (shared lib)        |
|  C structs: cef_browser_t, cef_client_t   |
|  cef_base_ref_counted_t reference counting|
|  cef_string_t string types                |
+-------------------------------------------+
|        Chromium Content API (C++)         |
|        Blink, V8, cc, Viz, Mojo           |
+-------------------------------------------+
```

### Why the C API Exists

Two critical reasons:

1. **Memory Management Isolation** — `libcef` and the host application may use different C/C++ runtimes with different heap allocators. Objects (including strings) must be freed by the same runtime that allocated them. The C API boundary enforces this.

2. **ABI Stability** — C++ has no stable ABI (name mangling, vtable layout, etc. vary by compiler). A C API provides a stable binary interface that any language can call. This enables the language bindings ecosystem.

3. **String Type Flexibility** — `libcef` can be compiled with different underlying string types (UTF-8, UTF-16, or wide). The C API abstracts this away.

### C++ to C Struct Mapping

Every C++ class maps to a C struct with the same name pattern:

| C++ Class | C Struct | Header |
|-----------|----------|--------|
| `CefBrowser` | `cef_browser_t` | `cef_browser_capi.h` |
| `CefClient` | `cef_client_t` | `cef_client_capi.h` |
| `CefApp` | `cef_app_t` | `cef_app_capi.h` |
| `CefLifeSpanHandler` | `cef_life_span_handler_t` | `cef_life_span_handler_capi.h` |

The translation between C++ and C is handled by an **auto-generated translator tool**. Most developers never touch the C API directly — they use the C++ wrapper.

### Reference Counting: `cef_base_ref_counted_t`

Every CEF C struct begins with a `cef_base_ref_counted_t` member:

```c
typedef struct _cef_base_ref_counted_t {
    size_t size;
    void (CEF_CALLBACK *add_ref)(struct _cef_base_ref_counted_t* self);
    int  (CEF_CALLBACK *release)(struct _cef_base_ref_counted_t* self);
    int  (CEF_CALLBACK *has_one_ref)(struct _cef_base_ref_counted_t* self);
    int  (CEF_CALLBACK *has_at_least_one_ref)(struct _cef_base_ref_counted_t* self);
} cef_base_ref_counted_t;
```

A callback struct (like `cef_client_t`) is laid out as:

```c
typedef struct _cef_client_t {
    cef_base_ref_counted_t base;  // Must be first member
    
    // Vtable-style function pointers:
    cef_life_span_handler_t* (CEF_CALLBACK *get_life_span_handler)(
        struct _cef_client_t* self);
    cef_display_handler_t* (CEF_CALLBACK *get_display_handler)(
        struct _cef_client_t* self);
    int (CEF_CALLBACK *on_process_message_received)(
        struct _cef_client_t* self,
        cef_browser_t* browser,
        cef_frame_t* frame,
        cef_process_id_t source_process,
        cef_process_message_t* message);
    // ... more handler getters
} cef_client_t;
```

Because `base` is the first member, a `cef_client_t*` can be safely cast to `cef_base_ref_counted_t*`.

In C++, the wrapper uses `CefRefPtr<T>` (similar to `std::shared_ptr`) which calls `AddRef()`/`Release()` automatically. The `IMPLEMENT_REFCOUNTING(ClassName)` macro provides the atomic ref-count implementation.

### String Types: `cef_string_t`

```c
typedef struct _cef_string_utf16_t {
    char16_t* str;
    size_t length;
    void (*dtor)(char16_t* str);  // Destructor for correct heap deallocation
} cef_string_utf16_t;

typedef cef_string_utf16_t cef_string_t;  // Default is UTF-16
```

The `dtor` function pointer ensures the string is freed by the correct runtime, even when strings cross the API boundary.

---

## 4. Language Bindings Ecosystem

CEF's C API enables bindings in virtually any language that can call C functions.

### C++ — Direct API (libcef_dll_wrapper)

The "official" way to use CEF. The static library `libcef_dll_wrapper` wraps the C API in idiomatic C++ classes:

```cpp
class CefClient : public virtual CefBaseRefCounted {
public:
    virtual CefRefPtr<CefLifeSpanHandler> GetLifeSpanHandler() { return nullptr; }
    virtual CefRefPtr<CefDisplayHandler> GetDisplayHandler() { return nullptr; }
    virtual bool OnProcessMessageReceived(
        CefRefPtr<CefBrowser> browser,
        CefRefPtr<CefFrame> frame,
        CefProcessId source_process,
        CefRefPtr<CefProcessMessage> message) { return false; }
};
```

Distributed as **source code** in the binary distribution. You compile it into your application. The sample applications (`cefsimple`, `cefclient`) demonstrate usage.

### Rust — cef-rs / cef-ui

Two-layer architecture:

**`cef-dll-sys` (low-level):** Uses `rust-bindgen` to auto-generate raw FFI bindings from CEF's C API headers. The `build.rs` script downloads platform-specific CEF binaries, aggregates headers into `wrapper.h`, runs bindgen, and compiles `libcef_dll_wrapper` via CMake.

**`cef` crate (safe wrapper):** Wraps raw FFI with safe, idiomatic Rust APIs.

Key patterns:

- **`RcImpl<T, I>`** — Dual-pointer struct containing the raw CEF C struct (`T`), a Rust trait implementation (`I`), and an `AtomicUsize` ref count. The CEF struct is positioned first so C code treats the pointer as a raw C struct, while Rust accesses the trait through the embedded implementation.

- **Conversion traits** — `ConvertParam<T>` (Rust -> C), `ConvertReturnValue<T>` (C -> Rust), `WrapParamRef<T, P>` (handle mutable C pointers).

- **`RefGuard<T>`** — RAII wrapper that calls `release()` on drop, preventing leaks.

- **Reference counting** — `AtomicUsize` with relaxed ordering for increments, release ordering for decrements, acquire fence on reaching zero.

- **Pattern recognition** — The binding generator detects C patterns like `(count: usize, ptr: *const T)` and converts them to safe `&[T]` slices.

### Java — JCEF (Java CEF)

Official Java bindings maintained at `github.com/chromiumembedded/java-cef`. Uses JNI to bridge Java classes to the CEF C++ API. Provides `CefApp`, `CefClient`, `CefBrowser` as Java classes. Used by JetBrains IDEs (IntelliJ IDEA, etc.) for their embedded browser.

### C#/.NET — CefSharp and CefGlue

**CefSharp:** ~30% C++/CLI, ~70% C#. Provides WPF and WinForms browser controls. Wraps the C++ API directly through C++/CLI interop. Most popular .NET CEF binding.

**CefGlue:** Pure C# using P/Invoke against the C API directly. Provides Avalonia and WPF controls. No C++/CLI dependency, so it works on .NET Core/5+/6+ more naturally.

### Python — cefpython

Python bindings using Cython to wrap the CEF C++ API. Provides a Pythonic interface for embedding browsers in Python applications (e.g., with wxPython, PyQt, Tk).

### Go — go-cef / cef2go / energy

Multiple Go bindings exist:
- `cef2go` (by CzarekTomczak) — CGo bindings to the C API
- `energy` (by energye) — More complete Go framework wrapping CEF

### Delphi — CEF4Delphi

Comprehensive Delphi/Lazarus/FPC bindings. Provides VCL, FireMonkey (FMX), and Lazarus components. Supports Windows, Linux, and macOS.

---

## 5. CEF Views vs Native Window

CEF provides two windowing modes for browser rendering.

### Mode 1: Native Window (Parent HWND)

You provide a native window handle (HWND on Windows, X11 window on Linux, NSView on macOS). CEF creates its browser widget as a child of your window.

```cpp
CefWindowInfo window_info;
window_info.SetAsChild(parent_hwnd, rect);  // Your native window
CefBrowserHost::CreateBrowser(window_info, client, url, settings, nullptr, nullptr);
```

**Pros:**
- Full control over window chrome, title bar, frame
- Easy integration with existing native UI frameworks
- Can mix native widgets with embedded browser
- Works on all platforms

**Cons:**
- You manage window lifecycle, sizing, DPI scaling
- Must handle coordinate conversion (DIP vs device pixels) yourself
- Frameless windows require platform-specific code (WS_POPUP on Windows, etc.)

### Mode 2: CEF Views (Cross-Platform)

CEF manages the window using its built-in Views/Aura framework (the same GUI framework Chromium uses on Windows and Linux).

```cpp
// Create browser view
CefRefPtr<CefBrowserView> browser_view = CefBrowserView::CreateBrowserView(
    client, url, settings, nullptr, nullptr, nullptr);

// Create window containing the browser view
CefWindow::CreateTopLevelWindow(new MyWindowDelegate(browser_view));
```

**Pros:**
- Cross-platform window management (consistent behavior)
- Built-in support for frameless windows (`CefWindowDelegate::IsFrameless()` returns true)
- Integrated with CEF's bounds/resize callbacks
- Simpler code for basic windowed browsers

**Cons:**
- Currently supported on **Windows and Linux only** (macOS support limited)
- Less flexibility for complex native UI
- Tied to CEF's window management semantics

**Frameless in Views:** Return `true` from `CefWindowDelegate::IsFrameless()`. The `cefclient` sample demonstrates this with `--hide-frame`.

### Mode 3: Off-Screen (Windowless) Rendering

CEF renders to a pixel buffer that you paint yourself. No native window is created by CEF.

```cpp
CefWindowInfo window_info;
window_info.SetAsWindowless(parent_hwnd);  // parent is optional
CefBrowserHost::CreateBrowser(window_info, client, url, settings, nullptr, nullptr);
```

You implement `CefRenderHandler`:
- `GetViewRect()` — Return desired view rectangle
- `OnPaint()` — Receive invalidated regions + pixel buffer (BGRA format)
- `GetScreenInfo()` — Configure DPI/screen info

You forward input events via `CefBrowserHost::SendMouseClickEvent()`, `SendKeyEvent()`, etc.

**Pros:**
- Render browser content into any surface (game engines, custom compositors)
- Full control over compositing
- Works for headless/server-side rendering

**Cons:**
- No accelerated compositing (software rendering only) — performance impact
- You handle all input forwarding manually
- More complex integration

### Coordinate Systems

- **CEF Views APIs** — Use DIP (Density Independent Pixels) directly
- **Windows APIs** — Expect device (pixel) coordinates; use `CefDisplay` for conversion
- **Linux** — Root window pixel coordinates
- **macOS Cocoa** — DIP with lower-left origin

---

## 6. The Handler/Callback Model

CEF uses a handler/callback pattern where your application implements interfaces that CEF calls at specific lifecycle points. In the C API, these are vtable-style structs with function pointers.

### CefApp — Application-Level Callbacks

```
CefApp
  |-- OnBeforeCommandLineProcessing()    // Modify command-line args
  |-- OnRegisterCustomSchemes()          // Register custom URI schemes
  |-- GetBrowserProcessHandler()         // Return browser-process handler
  |      |-- OnContextInitialized()      // CEF fully initialized
  |      |-- OnBeforeChildProcessLaunch() // Before subprocess spawn
  |-- GetRenderProcessHandler()          // Return renderer-process handler
         |-- OnWebKitInitialized()       // WebKit initialized in renderer
         |-- OnBrowserCreated()          // Browser created in renderer
         |-- OnContextCreated()          // V8 context created (JS ready)
         |-- OnContextReleased()         // V8 context destroyed
         |-- OnProcessMessageReceived()  // IPC from browser process
```

### CefClient — Per-Browser Handler Hub

A single `CefClient` can be shared among multiple browser instances. It returns sub-handlers:

```
CefClient
  |-- GetDisplayHandler()       -> CefDisplayHandler
  |     |-- OnTitleChange()
  |     |-- OnAddressChange()
  |     |-- OnConsoleMessage()
  |
  |-- GetLifeSpanHandler()      -> CefLifeSpanHandler
  |     |-- OnBeforePopup()          // Intercept popup creation
  |     |-- OnAfterCreated()         // Browser created (UI thread)
  |     |-- DoClose()                // Close requested
  |     |-- OnBeforeClose()          // Final cleanup
  |
  |-- GetLoadHandler()          -> CefLoadHandler
  |     |-- OnLoadStart()
  |     |-- OnLoadEnd()
  |     |-- OnLoadError()
  |
  |-- GetRequestHandler()       -> CefRequestHandler
  |     |-- OnBeforeBrowse()
  |     |-- GetResourceHandler()     // Intercept arbitrary requests
  |     |-- GetResourceResponseFilter() // Filter response data
  |     |-- GetAuthCredentials()     // Proxy/HTTP auth
  |     |-- OnCertificateError()
  |
  |-- GetDownloadHandler()      -> CefDownloadHandler
  |-- GetDragHandler()          -> CefDragHandler
  |-- GetFocusHandler()         -> CefFocusHandler
  |-- GetKeyboardHandler()      -> CefKeyboardHandler
  |-- GetContextMenuHandler()   -> CefContextMenuHandler
  |-- GetDialogHandler()        -> CefDialogHandler
  |-- GetRenderHandler()        -> CefRenderHandler (off-screen only)
  |     |-- GetViewRect()
  |     |-- OnPaint()
  |     |-- GetScreenInfo()
  |
  |-- OnProcessMessageReceived()  // IPC from renderer process
```

### C API Vtable Pattern

In the C API, each handler is a struct with function pointers (a C vtable):

```c
// You allocate and populate this struct
cef_life_span_handler_t handler;
handler.base.size = sizeof(cef_life_span_handler_t);
handler.base.add_ref = my_add_ref;
handler.base.release = my_release;
handler.base.has_one_ref = my_has_one_ref;
handler.on_after_created = my_on_after_created;
handler.do_close = my_do_close;
handler.on_before_close = my_on_before_close;
```

### Thread Safety — Which Callbacks Run Where

This is critical to get right:

| Callback | Thread | Process |
|----------|--------|---------|
| `OnAfterCreated()` | TID_UI | Browser |
| `OnBeforeClose()` | TID_UI | Browser |
| `OnContextInitialized()` | TID_UI | Browser |
| `OnTitleChange()` | TID_UI | Browser |
| `OnConsoleMessage()` | TID_UI | Browser |
| `OnLoadStart/End/Error()` | TID_UI | Browser |
| `GetAuthCredentials()` | TID_IO | Browser |
| Network callbacks | TID_IO (varies) | Browser |
| `OnContextCreated()` | TID_RENDERER | Renderer |
| `OnProcessMessageReceived()` | TID_UI (browser) or TID_RENDERER (renderer) | Both |
| `OnPaint()` | TID_UI | Browser |

Use `CefCurrentlyOn(TID_UI)` to assert the expected thread:

```cpp
#define CEF_REQUIRE_UI_THREAD()       DCHECK(CefCurrentlyOn(TID_UI));
#define CEF_REQUIRE_IO_THREAD()       DCHECK(CefCurrentlyOn(TID_IO));
#define CEF_REQUIRE_RENDERER_THREAD() DCHECK(CefCurrentlyOn(TID_RENDERER));
```

Cross-thread work is dispatched via `CefPostTask()`:

```cpp
CefPostTask(TID_UI, base::BindOnce(&MyFunction, arg1, arg2));
```

---

## 7. IPC and Message Routing

### Process Startup Data

When creating a browser, you can pass initialization data via the `extra_info` parameter (`CefDictionaryValue`) to `CefBrowserHost::CreateBrowser()`. This data is delivered to every renderer process associated with that browser via `CefRenderProcessHandler::OnBrowserCreated()`.

### Runtime Process Messages (CefProcessMessage)

The fundamental IPC primitive for application-level messaging:

```
Browser Process                          Renderer Process
      |                                       |
      |  CefProcessMessage("my_msg")          |
      |-------------------------------------->|
      |  browser->GetMainFrame()              |
      |    ->SendProcessMessage(PID_RENDERER) |
      |                                       |
      |  Received in:                         |
      |  CefRenderProcessHandler::            |
      |    OnProcessMessageReceived()         |
      |                                       |
      |  CefProcessMessage("response")        |
      |<--------------------------------------|
      |  frame->SendProcessMessage(           |
      |    PID_BROWSER)                       |
      |                                       |
      |  Received in:                         |
      |  CefClient::                          |
      |    OnProcessMessageReceived()         |
```

Messages carry a `CefListValue` argument list supporting strings, ints, doubles, booleans, binary data, dictionaries, and nested lists.

### CefMessageRouter — High-Level JS-to-C++ Bridge

CEF provides a built-in generic message router for asynchronous JavaScript-to-C++ communication.

**Setup:**

1. Create `CefMessageRouterBrowserSide` in the browser process with a `CefMessageRouterConfig`
2. Create `CefMessageRouterRendererSide` in the renderer process with the **same** config
3. Register application `Handler` instances with the browser-side router
4. Wire router callbacks into `CefClient` and `CefRenderProcessHandler`

**JavaScript API (injected into `window`):**

```javascript
// Send a query to C++
var request_id = window.cefQuery({
    request: 'my_request_data',
    persistent: false,        // true = keep alive for multiple responses
    onSuccess: function(response) {
        console.log('Got response:', response);
    },
    onFailure: function(error_code, error_message) {
        console.log('Error:', error_code, error_message);
    }
});

// Cancel a pending query
window.cefQueryCancel(request_id);
```

The function names (`cefQuery`, `cefQueryCancel`) are configurable via `CefMessageRouterConfig.js_query_function` and `js_cancel_function`.

**C++ Handler:**

```cpp
class MyHandler : public CefMessageRouterBrowserSide::Handler {
    bool OnQuery(CefRefPtr<CefBrowser> browser,
                 CefRefPtr<CefFrame> frame,
                 int64_t query_id,
                 const CefString& request,
                 bool persistent,
                 CefRefPtr<Callback> callback) override {
        if (request == "get_data") {
            callback->Success("here is the data");
            return true;  // Handled
        }
        return false;  // Pass to next handler
    }
    
    void OnQueryCanceled(CefRefPtr<CefBrowser> browser,
                         CefRefPtr<CefFrame> frame,
                         int64_t query_id) override {
        // Clean up resources for canceled query
    }
};
```

**Message flow:**

```
JavaScript: window.cefQuery({request, onSuccess, onFailure})
  -> Renderer: CefMessageRouterRendererSide intercepts
  -> IPC message to browser process
  -> Browser: CefMessageRouterBrowserSide routes to Handler
  -> Handler.OnQuery() executes, calls Callback.Success/Failure
  -> IPC response to renderer
  -> Renderer: invokes JS onSuccess/onFailure callback
```

Messages exceeding `CefMessageRouterConfig.message_size_threshold` bytes automatically route through shared memory regions for efficiency.

### Custom V8 JavaScript Bindings

For direct V8 integration (lower-level than CefMessageRouter):

1. In `CefRenderProcessHandler::OnContextCreated()`, access the V8 context
2. Create `CefV8Value` objects (functions, objects, arrays)
3. Bind them to `window` or other global objects
4. Implement `CefV8Handler::Execute()` for function callbacks

This runs in the renderer process. To communicate with the browser process, send `CefProcessMessage` from the V8 handler callback.

### Synchronous Requests

Synchronous browser-renderer IPC is discouraged but possible via synchronous `XMLHttpRequest` from the renderer, which blocks the renderer until the browser's network layer responds. The request can be intercepted by `CefRequestHandler::GetResourceHandler()` in the browser process.

---

## 8. Content/Rendering Pipeline

### From HTML to Pixels — The Full Pipeline

The rendering pipeline has 12 stages, split across the main thread, compositor thread, and GPU process:

```
RENDERER PROCESS — Main Thread (TID_RENDERER)
+------------------------------------------------------------------+
| 1. ANIMATE    — Apply CSS animations, modify property trees       |
| 2. STYLE      — Resolve CSS -> computed styles per DOM node       |
| 3. LAYOUT     — Compute positions and sizes -> fragment tree      |
| 4. PRE-PAINT  — Build property trees, invalidate display lists    |
| 5. SCROLL     — Update scroll offsets in property trees           |
| 6. PAINT      — Generate display lists (SkPicture) for tiles      |
| 7. COMMIT     — Copy property trees + display lists to compositor |
+------------------------------------------------------------------+
         | (Commit blocks main thread briefly)
         v
RENDERER PROCESS — Compositor Thread
+------------------------------------------------------------------+
| 8. LAYERIZE   — Break display lists into composited layer lists   |
| 9. RASTER     — Convert display lists -> GPU texture tiles        |
|    (via raster worker threads -> GPU process)                     |
|10. ACTIVATE   — Create compositor frame (quads + render passes)   |
+------------------------------------------------------------------+
         | (Submit compositor frame via CompositorFrameSink)
         v
GPU PROCESS (Viz)
+------------------------------------------------------------------+
|11. AGGREGATE  — Combine frames from ALL render + browser processes|
|12. DRAW       — Execute on GPU hardware -> pixels on screen       |
+------------------------------------------------------------------+
```

### Stage Details

**DOM -> RenderObjects -> RenderLayers -> GraphicsLayers:**

```
DOM Tree                    1:1 visible nodes
  |
  v
RenderObject Tree           Each visible DOM node gets a RenderObject
  |                         (knows how to paint itself)
  v
RenderLayer Tree            Groups RenderObjects sharing coordinate space
  |                         (preserves z-ordering)
  v
GraphicsLayer Tree          RenderLayers that get their own GPU texture:
  |                         - 3D/perspective CSS transforms
  |                         - <video> with accelerated decoding
  |                         - <canvas> with WebGL/3D context
  |                         - Animated opacity or transforms
  |                         - Accelerated CSS filters
  |                         - Overlapping composited layers
  v
cc Layer List               Chrome compositor's final representation
```

**Painting (Display List Recording):**
Blink paints invalidated regions into `SkPicture`-backed `GraphicsContext`. This creates a **display list** (recording of draw commands), not actual pixels. The "interest area" around the viewport bounds what gets recorded.

**Tiling:**
Rather than rasterizing entire layers, the compositor breaks content into **tiles** and rasterizes per-tile. Tiles are prioritized by viewport proximity and estimated time-to-visibility. GPU memory is allocated to tiles based on priority.

**Rasterization (Two Paths):**

1. **Software rasterization** — SkPicture playback targets bitmaps in shared renderer-GPU memory. Occurs on compositor raster worker threads (parallel). Completed tiles upload to GPU as textures.

2. **GPU rasterization (Ganesh/Graphite)** — SkPicture plays back via Skia's GPU backend, generating GL commands sent directly to the GPU process. Tiles become GPU textures in-place. No upload step needed.

**Compositor Thread Independence:**
The compositor thread can scroll and animate independently of the main thread. When the main thread is busy running JavaScript, the compositor continues producing frames. This is why CSS animations and scrolling remain smooth even during heavy JS execution.

**The Commit:**
When the main thread finishes a paint, it "commits" the updated layer trees and SkPicture recordings to the compositor thread. The main thread blocks briefly during commit. The compositor maintains two trees:
- **Pending tree** — New content being rasterized
- **Active tree** — Currently displayed content

The pending tree activates only when visible high-resolution content is fully rasterized. If the user scrolls into unrecorded regions, a checkerboard pattern appears.

**Drawing:**
Drawing traverses the layer hierarchy depth-first, issuing GL commands to render each tile as a textured quad into the framebuffer. The compositor generates quads and render passes, which the Viz display compositor aggregates and executes.

### Property Trees (Transform, Clip, Effect, Scroll)

Instead of propagating transforms/clips down the layer tree, Chromium uses four separate **property trees** that layers reference by index:
- **Transform tree** — Accumulated CSS transforms
- **Clip tree** — Clip rectangles
- **Effect tree** — Opacity, filters, blend modes
- **Scroll tree** — Scroll offsets

This allows efficient invalidation (changing a transform only updates the transform tree node, not all descendant layers).

---

## 9. CEF Build System

### Building from Source

CEF is built from Chromium source using the `automate-git.py` script:

```bash
# Build master branch
python automate-git.py --download-dir=/path/to/download

# Build a specific release branch
python automate-git.py --download-dir=/path/to/download --branch=7680

# 64-bit build
python automate-git.py --download-dir=/path/to/download --branch=7680 --x64-build
```

The script performs:
1. Downloads `depot_tools`, Chromium source, and CEF source
2. Applies CEF patches to Chromium
3. Generates build files using GN (required since branch 2840)
4. Compiles Debug and Release builds using Ninja
5. Runs `make_distrib` to create the binary distribution package

**Requirements:**
- Minimum 8GB RAM (16GB+ recommended)
- ~100GB disk space for full Chromium source + build
- Ninja build system
- Clang compiler (default on all platforms; Windows since branch 3282)
- Platform SDK (Windows SDK, Xcode, etc.)

On subsequent runs, the script does the minimum work necessary. When switching branches, the existing `out` directory is moved to `out_(previousbranch)`.

### Binary Distribution Contents

The binary distribution is what most developers use (downloaded from `cef-builds.spotifycdn.com`):

```
cef_binary_<version>_<platform>/
  |
  |-- CMakeLists.txt              # Build the wrapper + samples
  |-- README.txt                  # Build instructions
  |
  |-- include/                    # CEF C++ and C API headers
  |     |-- cef_app.h, cef_browser.h, cef_client.h, ...
  |     |-- capi/
  |     |     |-- cef_app_capi.h, cef_browser_capi.h, ...
  |     |-- views/
  |     |     |-- cef_window.h, cef_browser_view.h, ...
  |     |-- base/
  |           |-- cef_ref_counted.h, cef_lock.h, ...
  |
  |-- libcef_dll/                 # libcef_dll_wrapper source code
  |
  |-- Release/                    # Release build binaries
  |     |-- libcef.dll            # Core CEF library (Windows)
  |     |   (or libcef.so on Linux, "Chromium Embedded Framework.framework" on macOS)
  |     |-- libEGL.dll            # ANGLE EGL implementation
  |     |-- libGLESv2.dll         # ANGLE GL ES implementation
  |     |-- d3dcompiler_47.dll    # D3D shader compiler (Windows)
  |     |-- vk_swiftshader.dll    # SwiftShader Vulkan (software GL)
  |     |-- vulkan-1.dll          # Vulkan loader
  |     |-- chrome_elf.dll        # Chrome crash reporting
  |     |-- snapshot_blob.bin     # V8 snapshot data
  |     |-- v8_context_snapshot.bin # V8 context snapshot
  |     |-- icudtl.dat            # ICU Unicode data
  |
  |-- Resources/                  # (Windows/Linux only)
  |     |-- chrome_100_percent.pak  # UI resources @1x
  |     |-- chrome_200_percent.pak  # UI resources @2x
  |     |-- resources.pak           # Non-localized resources
  |     |-- locales/
  |           |-- en-US.pak, fr.pak, de.pak, ...  # Per-locale strings
  |
  |-- Debug/                      # Debug build binaries (same files as Release)
  |
  |-- tests/
        |-- cefsimple/            # Minimal browser sample
        |-- cefclient/            # Full-featured sample
        |-- ceftests/             # Unit tests
```

**Required at runtime:** `libcef.dll` (or equivalent), `icudtl.dat`, `v8_context_snapshot.bin`, `snapshot_blob.bin`, resource `.pak` files, at least `en-US.pak` locale.

**Optional:** Only distribute locales you support. If none configured, `en-US` is the default.

---

## 10. How CEF Differs from Electron

Both embed Chromium for desktop applications, but the architecture is fundamentally different.

| Aspect | CEF | Electron |
|--------|-----|----------|
| **Core abstraction** | C API wrapping Chromium Content API | Node.js + Chromium merged |
| **Runtime** | Native (C/C++/Rust/etc. host) | Node.js (JavaScript everywhere) |
| **Chromium integration** | Via Content API (same as Chrome uses) | Builds Chromium as a library, patches it |
| **Does NOT use CEF** | N/A | Correct — Electron wraps Chromium directly, not via CEF |
| **Language support** | Any language with C FFI | JavaScript/TypeScript only |
| **Primary use case** | Embed browser into existing native app | Build entire app in web technologies |
| **Process model** | Chromium multi-process (browser, renderer, GPU) | Same multi-process + Node.js in main process |
| **System access** | Via host application's native code | Via Node.js APIs in main process |
| **API stability** | Stable C API with versioned branches | Electron API changes between major versions |
| **Binary size** | Smaller (no Node.js) | Larger (Chromium + Node.js) |
| **Resource usage** | Generally lower (no Node.js overhead) | Higher (Node.js event loop + Chromium) |
| **Community** | Smaller but well-maintained | Large, extensive ecosystem (npm) |
| **Examples** | Spotify (desktop), Steam, Unreal Engine, Adobe Premiere | VS Code, Discord, Slack, Notion |

**Key architectural difference:** In Electron, the main process runs Node.js and communicates with renderer processes via Electron's `ipcMain`/`ipcRenderer`. In CEF, the main process is your native application, and communication uses CEF's `CefProcessMessage` or Mojo under the hood. CEF gives you lower-level control; Electron gives you developer productivity with web technologies.

---

## 11. Sandboxing

CEF inherits Chromium's sandbox architecture, which restricts process privileges to minimize the impact of security vulnerabilities.

### Sandbox by Process Type

| Process | Sandbox Level | Notes |
|---------|--------------|-------|
| Browser | None | Main process, needs full system access |
| Renderer | Maximum | Untrusted integrity level (S-1-16-0 on Windows). Cannot access filesystem, network, or most OS APIs directly |
| GPU | Partial | Needs graphics driver access; runs at higher integrity than renderer |
| Utility | Varies | Sandbox level depends on the utility task |
| Network | Sandboxed | Isolated network operations |

### Windows Sandbox Implementation

On Windows, Chromium's sandbox:
1. **Restricts the process token** — Removes privileges, lowers integrity level
2. **Job objects** — Limits process resource usage
3. **Alternate desktop** — Isolates window stations
4. **Mitigation policies** — DEP, ASLR, CFG enforcement
5. **Interception** — Hooks certain API calls to enforce policy

Renderers get the most restrictive sandbox: Untrusted integrity level, minimal token privileges, no filesystem or registry access except through IPC to the browser process.

### CEF Sandbox Configuration

```cpp
CefSettings settings;
settings.no_sandbox = true;  // Disable sandbox entirely
```

Or via command line: `--no-sandbox`

**When to disable:**
- Development/debugging (easier process attachment)
- Compatibility issues with certain drivers or environments
- Docker/container environments where the sandbox conflicts with container isolation

**macOS-specific:**
- Uses `CefScopedSandboxContext` for helper processes
- `CefScopedLibraryLoader` loads CEF framework at runtime (required by macOS sandbox)
- Helper app has separate bundle and Info.plist (prevents dock icon, etc.)

**Linux-specific:**
- Zygote process handles sandboxed forking
- Two zygotes: sandboxed (for renderers) and unsandboxed (for processes needing system access)
- `--no-zygote` flag disables the zygote model

### Security Implications

Disabling the sandbox (`--no-sandbox`) means a compromised renderer process has the same privileges as the host application. This is a significant security risk for applications loading untrusted web content.

---

## 12. The GPU Process in Detail

### Purpose

The GPU process exists for two reasons:

1. **Security** — Renderer processes run in a restrictive sandbox and cannot call OS graphics APIs (OpenGL, Direct3D, Vulkan) directly. The GPU process provides controlled access.

2. **Stability** — GPU driver crashes don't crash the browser. The GPU process can crash and restart independently.

### What It Does

```
+-----------------------------------------------------------------------+
|                          GPU PROCESS                                   |
|                                                                        |
|  +-------------------+    +-------------------+                        |
|  | GPU Command Buffer|    | GPU Command Buffer|    (one per client)    |
|  | Server (Renderer1)|    | Server (Renderer2)|                        |
|  +--------+----------+    +--------+----------+                        |
|           |                        |                                   |
|           v                        v                                   |
|  +------------------------------------------------+                   |
|  |            ANGLE Translation Layer              |                   |
|  |  GL ES 2.0/3.0 -> Platform Native API           |                   |
|  |                                                  |                   |
|  |  Windows: -> Direct3D 11 (default)               |                   |
|  |           -> Direct3D 9 (fallback)               |                   |
|  |           -> Vulkan (if available)               |                   |
|  |  Linux:   -> Desktop OpenGL                      |                   |
|  |           -> Vulkan                              |                   |
|  |  macOS:   -> Metal (default)                     |                   |
|  |           -> OpenGL (deprecated)                 |                   |
|  +------------------------------------------------+                   |
|                          |                                             |
|  +------------------------------------------------+                   |
|  |        Viz Display Compositor                   |                   |
|  |  - Aggregates compositor frames from ALL        |                   |
|  |    render processes + browser process           |                   |
|  |  - Produces final composited frame              |                   |
|  +------------------------------------------------+                   |
|                          |                                             |
|  +------------------------------------------------+                   |
|  |        Skia/Ganesh (GPU Rasterization)          |                   |
|  |  - Converts display lists to GPU textures       |                   |
|  |  - Direct GL commands for tile rasterization    |                   |
|  +------------------------------------------------+                   |
|                          |                                             |
|                    [ Screen Output ]                                   |
+-----------------------------------------------------------------------+
```

### GPU Command Buffer

The command buffer is the core mechanism for cross-process GPU access:

```
RENDERER PROCESS                    GPU PROCESS
+------------------+               +------------------+
| GLES2Implementation|             | GLES2DecoderImpl |
| (client-side GL)   |             | (server-side)    |
|                    |             |                    |
| Serializes GL     |  Shared     | Reads commands,   |
| calls into ring   |  Memory     | deserializes,     |
| buffer            |  Ring       | executes real GL  |
|                    |  Buffer    | via ANGLE         |
| glClear() ->      |----------->| -> D3D11/GL call  |
| glDrawArrays() -> |            |                    |
| glTexImage2D() -> |            |                    |
+------------------+              +------------------+
```

**How it works:**
1. The renderer's GL implementation (`GLES2Implementation`) serializes GL ES 2.0 commands into a shared-memory ring buffer
2. The client writes commands at high speed with minimal IPC overhead
3. Periodically, the client signals the GPU process that new commands are available
4. The GPU process reads, validates, and executes commands via ANGLE
5. Most GL calls have no return value, enabling asynchronous operation

**Shared memory for bulk data:**
Textures, vertex arrays, and bitmaps transfer via shared memory regions (not through the command buffer ring). The `gpu::TransferBuffer` manages alignment and position tracking.

**Synchronization primitives:**
- **Mailboxes** — Share textures between command buffers using string identifiers
- **Sync tokens** — Ordered execution guarantees across command buffers (insert on A, wait on B ensures A's commands complete before B continues)

### ANGLE (Almost Native Graphics Layer Engine)

ANGLE translates OpenGL ES 2.0/3.0 API calls to platform-native graphics APIs:

| Platform | Default Backend | Alternatives |
|----------|----------------|-------------|
| Windows | Direct3D 11 | Direct3D 9, Vulkan, OpenGL |
| Linux | Desktop OpenGL | Vulkan |
| macOS | Metal | OpenGL (deprecated) |
| Android | Native GLES | Vulkan |

This allows Chromium to use a single GL ES codebase everywhere while leveraging the best-supported graphics API per platform. On Windows, D3D11 is far more reliable than OpenGL drivers, making ANGLE essential.

### SwiftShader (CPU-Based GL)

SwiftShader is an open-source software implementation of Vulkan and OpenGL ES that runs entirely on the CPU. No GPU hardware required.

**SwANGLE = ANGLE + SwiftShader Vulkan:**
ANGLE uses SwiftShader's Vulkan implementation as its backend, providing a full GL ES -> Vulkan -> CPU software pipeline.

**Use cases:**
- Systems without GPUs (VMs, CI servers)
- Blocklisted GPU drivers
- WebGL fallback when hardware GL fails

**Security warning:** SwiftShader uses JIT-compiled code in the GPU process, making it a high-security risk. Automatic WebGL fallback to SwiftShader has been deprecated. Explicit opt-in with `--enable-unsafe-swiftshader` is required.

**Command-line control:**
```
--use-gl=angle --use-angle=swiftshader           # SwANGLE as GL ES driver
--use-gl=angle --use-angle=swiftshader-webgl      # SwANGLE for WebGL only
--use-vulkan=swiftshader                          # SwiftShader Vulkan directly
```

### GPU Fallback Stack

When the GPU process crashes repeatedly, Chrome falls through a stack of increasingly degraded modes:

```
                        Normal startup
                             |
                             v
                   +-------------------+
                   | HARDWARE_VULKAN   |  <-- Try Vulkan first (Linux)
                   +-------------------+
                             |
                        3 crashes
                             v
                   +-------------------+
                   | HARDWARE_GL       |  <-- Fall back to OpenGL
                   +-------------------+
                             |
                        3 crashes
                             v
                   +-------------------+
                   | SWIFTSHADER       |  <-- Software GL (CPU)
                   +-------------------+  <-- Hardware disabled,
                             |                SwiftShader for WebGL
                        3 crashes
                             v
                   +-------------------+
                   | DISPLAY_COMPOSITOR|  <-- GPU process only does
                   +-------------------+      display compositing
                             |              No acceleration at all
                        Crashes
                             v
                   +-------------------+
                   | BROWSER CRASH     |  <-- Stack empty, give up
                   +-------------------+
```

The fallback modes are stored as a stack in `GpuDataManagerImplPrivate::fallback_modes_`. Crash counting is tracked by `GpuProcessHost::RecordProcessCrash()`. Crashes are "forgiven" after elapsed time intervals (`GetForgiveMinutes()`). The crash limit is `kGpuFallbackCrashCount` (typically 3 in a short timeframe).

Platform variations:
- **Android** — Requires hardware acceleration; no SwiftShader/software fallback (except Chromecast audio-only)
- **Fuchsia** — Always expects Vulkan; no GL fallback
- **Windows/Linux** — Full fallback stack available

### Context Virtualization

The GPU process does not necessarily create a real driver-level GL context per client. It can share a single real context among multiple clients, saving and restoring GL state ("shadowed state") when switching between clients. This addresses:
- Slow GL context switches on some drivers
- Synchronization bugs with FBO rendering across contexts
- Driver crashes with share groups

Context virtualization is selectively enabled via the GPU blocklist based on known driver issues.

### What Happens When the GPU Process Crashes

1. Chromium detects the process handle signal
2. Crash count increments in `GpuProcessHost`
3. If under the limit: GPU process is restarted in the same mode
4. If at the limit: next mode is popped from the fallback stack
5. All renderer processes are notified of the context loss
6. Renderers re-create their GL contexts and re-upload resources
7. Compositing resumes with the new GPU process

Web content receives a `webglcontextlost` event for WebGL canvases. Well-behaved applications listen for `webglcontextrestored` and re-initialize.

---

## Sources

- [CEF General Usage Documentation](https://chromiumembedded.github.io/cef/general_usage.html)
- [CEF Branches and Building](https://chromiumembedded.github.io/cef/branches_and_building.html)
- [Chromium Multi-Process Architecture](https://www.chromium.org/developers/design-documents/multi-process-architecture/)
- [GPU Accelerated Compositing in Chrome](https://www.chromium.org/developers/design-documents/gpu-accelerated-compositing-in-chrome/)
- [RenderingNG Architecture (Chrome for Developers)](https://developer.chrome.com/docs/chromium/renderingng-architecture)
- [Chromium Graphics Overview](https://www.chromium.org/developers/design-documents/chromium-graphics/)
- [How cc Works](https://chromium.googlesource.com/chromium/src/+/lkgr/docs/how_cc_works.md)
- [Viz README](https://chromium.googlesource.com/chromium/src/+/HEAD/components/viz/)
- [Chromium Mojo README](https://chromium.googlesource.com/chromium/src/+/HEAD/mojo/README.md)
- [SwiftShader in Chromium](https://chromium.googlesource.com/chromium/src/+/HEAD/docs/gpu/swiftshader.md)
- [GPU Fallback Modes](https://chromium.googlesource.com/chromium/src/+/60b3c74b7f2ca17a28907fb0b40d9dabeaa48326/content/browser/gpu/fallback.md)
- [CEF Message Router Header](https://github.com/chromiumembedded/cef/blob/master/include/wrapper/cef_message_router.h)
- [cef-rs (Tauri Apps) DeepWiki](https://deepwiki.com/tauri-apps/cef-rs)
- [cef-ui (Hytopia)](https://github.com/hytopiagg/cef-ui)
- [CefSharp](https://github.com/cefsharp/CefSharp)
- [CefGlue](https://github.com/OutSystems/CefGlue)
- [CEF4Delphi](https://github.com/salvadordf/CEF4Delphi)
- [cefcapi C API Example](https://github.com/cztomczak/cefcapi)
- [CEF Wikipedia](https://en.wikipedia.org/wiki/Chromium_Embedded_Framework)
- [Chromium Sandbox Diagnostics](https://www.chromium.org/Home/chromium-security/articles/chrome-sandbox-diagnostics-for-windows/)
- [CEF Automated Builds](https://cef-builds.spotifycdn.com/index.html)
- [Electron vs CEF Comparison (Oreate AI)](https://www.oreateai.com/blog/electron-vs-cef-navigating-the-chromium-landscape-for-desktop-apps/a339a8fc18c70c7b798df7805f075221)
