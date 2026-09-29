// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Real GL check behind the Linux `HwGl` GPU tier
// (SPEC_LINUX_GPU_BACKEND_PRECEDENCE_2026_06_13 §7, finally built).
//
// The HwGl tier forces `--use-angle=gl --ignore-gpu-blocklist`, which turns off
// Chromium's own fallback to SwiftShader. Passing that on a GPU with no working
// GL crashes the GPU process on every attempt and nothing ever paints; on a
// frameless transparent window that is an invisible app
// (docs/retro/RETRO_INVISIBLE_WINDOW_ON_DEAD_GPU_2026_09_29.md). So HwGl is
// granted only when a throwaway EGL context actually reports a hardware
// renderer.
//
// The probe runs in a re-exec of this binary (same pattern as the userns probe
// in `linux_sandbox`), so the Mesa driver, and whatever threads llvmpipe
// starts, never load into the browser process, and a wedged driver is cut off
// by the timeout rather than hanging startup.

/// Re-exec flag: print the EGL `GL_RENDERER` string on stdout and exit.
pub(crate) const INTERNAL_PROBE_GL_FLAG: &str = "--internal-probe-gl-renderer";

/// How long the re-exec'd probe may take before it counts as "no hardware GL".
const GL_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// Renderer substrings that mean "software", whatever the device node says.
const SOFTWARE_RENDERER_MARKERS: &[&str] =
    &["llvmpipe", "softpipe", "swrast", "swiftshader", "software rasterizer", "lavapipe"];

/// Whether a `GL_RENDERER` string names a hardware renderer. An empty string
/// (the probe produced nothing) is not hardware.
pub(crate) fn is_hardware_gl_renderer(renderer: &str) -> bool {
    let r = renderer.trim().to_ascii_lowercase();
    !r.is_empty() && !SOFTWARE_RENDERER_MARKERS.iter().any(|m| r.contains(m))
}

/// Run the probe in a child process and classify its answer. Any failure
/// (spawn error, timeout, non-zero exit, no output) means "not confirmed", so
/// the caller falls back to the Software tier and Chromium's own fallback.
pub(crate) fn hardware_gl_confirmed() -> bool {
    use std::io::Read;
    use std::process::{Command, Stdio};

    let Ok(exe) = std::env::current_exe() else { return false };
    let Ok(mut child) = Command::new(exe)
        .arg(INTERNAL_PROBE_GL_FLAG)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let deadline = std::time::Instant::now() + GL_PROBE_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let mut renderer = String::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_string(&mut renderer);
    }
    let hardware = status.is_some_and(|s| s.success()) && is_hardware_gl_renderer(&renderer);
    tracing::info!(
        renderer = renderer.trim(),
        timed_out = status.is_none(),
        hardware,
        "GL renderer probe for the hw-gl GPU tier"
    );
    hardware
}

/// Probe-mode entry point: called first thing in `run()` when the process was
/// re-exec'd with [`INTERNAL_PROBE_GL_FLAG`]. Prints the renderer and exits 0,
/// or exits 1 if no GL context could be made.
pub(crate) fn run_internal_gl_probe_and_exit() -> ! {
    match unsafe { egl::renderer() } {
        Some(r) => {
            println!("{r}");
            std::process::exit(0)
        }
        None => std::process::exit(1),
    }
}

/// A surfaceless EGL context, just long enough to read `GL_RENDERER`. Loaded
/// with `dlopen` so there is no link-time GL dependency.
mod egl {
    use std::ffi::{c_char, c_void, CStr};

    type Ptr = *mut c_void;
    const EGL_NONE: i32 = 0x3038;
    const EGL_EXTENSIONS: i32 = 0x3055;
    const EGL_PLATFORM_SURFACELESS_MESA: u32 = 0x31DD;
    const EGL_OPENGL_ES_API: u32 = 0x30A0;
    const EGL_RENDERABLE_TYPE: i32 = 0x3040;
    const EGL_OPENGL_ES2_BIT: i32 = 0x0004;
    const EGL_SURFACE_TYPE: i32 = 0x3033;
    const EGL_CONTEXT_CLIENT_VERSION: i32 = 0x3098;
    const GL_RENDERER: u32 = 0x1F01;

    type QueryString = extern "C" fn(Ptr, i32) -> *const c_char;
    type GetDisplay = extern "C" fn(Ptr) -> Ptr;
    type GetPlatformDisplay = extern "C" fn(u32, Ptr, *const isize) -> Ptr;
    type Initialize = extern "C" fn(Ptr, *mut i32, *mut i32) -> u32;
    type BindApi = extern "C" fn(u32) -> u32;
    type ChooseConfig = extern "C" fn(Ptr, *const i32, *mut Ptr, i32, *mut i32) -> u32;
    type CreateContext = extern "C" fn(Ptr, Ptr, Ptr, *const i32) -> Ptr;
    type MakeCurrent = extern "C" fn(Ptr, Ptr, Ptr, Ptr) -> u32;
    type GetProcAddress = extern "C" fn(*const c_char) -> Ptr;
    type GetString = extern "C" fn(u32) -> *const c_char;

    unsafe fn sym<T: Copy>(lib: Ptr, name: &CStr) -> Option<T> {
        let p = libc::dlsym(lib, name.as_ptr());
        (!p.is_null()).then(|| std::mem::transmute_copy::<Ptr, T>(&p))
    }

    pub(super) unsafe fn renderer() -> Option<String> {
        let egl = libc::dlopen(c"libEGL.so.1".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
        if egl.is_null() {
            return None;
        }
        let query_string: QueryString = sym(egl, c"eglQueryString")?;
        let get_display: GetDisplay = sym(egl, c"eglGetDisplay")?;
        let initialize: Initialize = sym(egl, c"eglInitialize")?;
        let bind_api: BindApi = sym(egl, c"eglBindAPI")?;
        let choose_config: ChooseConfig = sym(egl, c"eglChooseConfig")?;
        let create_context: CreateContext = sym(egl, c"eglCreateContext")?;
        let make_current: MakeCurrent = sym(egl, c"eglMakeCurrent")?;
        let get_proc: GetProcAddress = sym(egl, c"eglGetProcAddress")?;

        // Prefer the Mesa surfaceless platform: it needs no X or Wayland
        // connection and uses the GPU the render node belongs to.
        let client_exts = query_string(std::ptr::null_mut(), EGL_EXTENSIONS);
        let surfaceless = !client_exts.is_null()
            && CStr::from_ptr(client_exts).to_string_lossy().contains("EGL_MESA_platform_surfaceless");
        let platform_display: Option<GetPlatformDisplay> = sym(egl, c"eglGetPlatformDisplay");
        let display = match (surfaceless, platform_display) {
            (true, Some(get_platform_display)) => {
                let none = [EGL_NONE as isize];
                get_platform_display(EGL_PLATFORM_SURFACELESS_MESA, std::ptr::null_mut(), none.as_ptr())
            }
            _ => get_display(std::ptr::null_mut()),
        };
        if display.is_null() {
            return None;
        }
        let (mut major, mut minor) = (0, 0);
        if initialize(display, &mut major, &mut minor) == 0 || bind_api(EGL_OPENGL_ES_API) == 0 {
            return None;
        }
        let mut config: Ptr = std::ptr::null_mut();
        let mut count = 0;
        let pbufferless = [EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT, EGL_SURFACE_TYPE, 0, EGL_NONE];
        let any_surface = [EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT, EGL_NONE];
        let found = (choose_config(display, pbufferless.as_ptr(), &mut config, 1, &mut count) != 0 && count > 0)
            || (choose_config(display, any_surface.as_ptr(), &mut config, 1, &mut count) != 0 && count > 0);
        if !found {
            return None;
        }
        let ctx_attrs = [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];
        let context = create_context(display, config, std::ptr::null_mut(), ctx_attrs.as_ptr());
        if context.is_null() || make_current(display, std::ptr::null_mut(), std::ptr::null_mut(), context) == 0 {
            return None;
        }
        // Core entry points aren't always exposed through eglGetProcAddress;
        // fall back to libGLESv2 directly.
        let mut get_string = get_proc(c"glGetString".as_ptr());
        if get_string.is_null() {
            let gles = libc::dlopen(c"libGLESv2.so.2".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
            if !gles.is_null() {
                get_string = libc::dlsym(gles, c"glGetString".as_ptr());
            }
        }
        if get_string.is_null() {
            return None;
        }
        let get_string: GetString = std::mem::transmute_copy::<Ptr, GetString>(&get_string);
        let r = get_string(GL_RENDERER);
        // No teardown: this process exits right after printing.
        (!r.is_null()).then(|| CStr::from_ptr(r).to_string_lossy().into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::is_hardware_gl_renderer;

    #[test]
    fn software_renderers_never_count_as_hardware_gl() {
        for r in [
            "llvmpipe (LLVM 19.1.7, 256 bits)",
            "softpipe",
            "Mesa X11 swrast",
            "Google SwiftShader",
            "Software Rasterizer",
            "",
            "   ",
        ] {
            assert!(!is_hardware_gl_renderer(r), "{r:?} must not be hardware");
        }
    }

    #[test]
    fn real_gpus_count_as_hardware_gl() {
        for r in [
            "SVGA3D; build: RELEASE;  LLVM;",
            "Mesa Intel(R) UHD Graphics 630 (CFL GT2)",
            "AMD Radeon RX 7800 XT (radeonsi, navi32, LLVM 19.1.7, DRM 3.59)",
            "NVIDIA GeForce RTX 3060/PCIe/SSE2",
        ] {
            assert!(is_hardware_gl_renderer(r), "{r:?} must be hardware");
        }
    }
}
