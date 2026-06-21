// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Wayland splash backend — a software-drawn (`wl_shm`) `xdg_toplevel` showing
//! the pulsing brain, for native-Wayland sessions (the default since #1611).
//!
//! Wayland deliberately denies clients window positioning and "always on top",
//! and GNOME/Mutter does not implement `wlr-layer-shell`, so this is a
//! best-effort splash: a borderless `xdg_toplevel` the compositor places (Mutter
//! centers small toplevels) and which we dismiss the instant the host paints.
//! No GPU/EGL — a shared-memory buffer we blit each frame. See
//! docs/specs/SPEC_LINUX_SPLASH_SESSION_AWARE_2026_06_20.md §3, §6.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_output, delegate_registry, delegate_shm, delegate_xdg_shell,
    delegate_xdg_window,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        xdg::{
            window::{Window, WindowConfigure, WindowDecorations, WindowHandler},
            XdgShell,
        },
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};
use smithay_client_toolkit::reexports::client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_shm, wl_surface},
    Connection, QueueHandle,
};

use super::{
    fade_alpha, min_hold, pulse_alpha, render_frame, BRAIN_H, BRAIN_W, CORNER_RADIUS_PX,
    DISMISS_TIMEOUT, PADDING,
};

pub(super) fn run(ready_file: &Path) -> Result<(), Box<dyn Error>> {
    let conn = Connection::connect_to_env()?;
    let (globals, mut event_queue) = registry_queue_init::<SplashState>(&conn)?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)?;
    let shm = Shm::bind(&globals, &qh)?;
    let xdg_shell = XdgShell::bind(&globals, &qh)?;

    let w = (BRAIN_W + PADDING * 2) as u32;
    let h = (BRAIN_H + PADDING * 2) as u32;

    let surface = compositor.create_surface(&qh);
    let window = xdg_shell.create_window(surface, WindowDecorations::None, &qh);
    window.set_title("AgentMux");
    // Match the host's app_id so the compositor associates the two.
    window.set_app_id("ai.agentmux.AgentMux");
    window.set_min_size(Some((w, h)));
    window.set_max_size(Some((w, h)));
    window.commit();

    let pool = SlotPool::new((w * h * 4) as usize, &shm)?;

    let mut state = SplashState {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        shm,
        pool,
        window,
        width: w,
        height: h,
        start: Instant::now(),
        min_hold: min_hold(),
        ready_file: ready_file.to_path_buf(),
        fade_start: None,
        exit: false,
    };

    while !state.exit {
        event_queue.blocking_dispatch(&mut state)?;
    }
    Ok(())
}

struct SplashState {
    registry_state: RegistryState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    window: Window,
    width: u32,
    height: u32,
    start: Instant,
    min_hold: Duration,
    ready_file: PathBuf,
    fade_start: Option<Instant>,
    exit: bool,
}

impl SplashState {
    fn should_dismiss(&self) -> bool {
        let elapsed = self.start.elapsed();
        (self.ready_file.exists() && elapsed >= self.min_hold) || elapsed >= DISMISS_TIMEOUT
    }

    /// Paint one frame and request the next frame callback, or set `exit` once
    /// the dismiss condition is met. Driven by the compositor's frame callbacks
    /// (which also pace the ~60 fps pulse).
    fn draw(&mut self, qh: &QueueHandle<Self>) {
        let now = Instant::now();
        // Begin the fade-out the first time the dismiss condition holds; tear
        // down once it completes.
        if self.fade_start.is_none() && self.should_dismiss() {
            self.fade_start = Some(now);
        }
        let window_alpha = fade_alpha(self.fade_start, now);
        if self.fade_start.is_some() && window_alpha <= 0.0 {
            self.exit = true;
            return;
        }

        let (w, h) = (self.width as i32, self.height as i32);
        let stride = w * 4;
        let (buffer, canvas) = match self
            .pool
            .create_buffer(w, h, stride, wl_shm::Format::Argb8888)
        {
            Ok(b) => b,
            Err(_) => {
                self.exit = true;
                return;
            }
        };

        let brain_alpha = pulse_alpha(self.start.elapsed().as_secs_f32());
        // wl_shm ARGB8888 is native-endian 0xAARRGGBB → pre-multiplied B,G,R,A on LE.
        render_frame(
            canvas,
            w,
            h,
            brain_alpha,
            window_alpha,
            CORNER_RADIUS_PX,
            /* bgr = */ true,
        );

        let surface = self.window.wl_surface();
        surface.damage_buffer(0, 0, w, h);
        surface.frame(qh, surface.clone());
        let _ = buffer.attach_to(surface);
        self.window.commit();
    }
}

impl CompositorHandler for SplashState {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: i32,
    ) {
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn frame(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: u32,
    ) {
        self.draw(qh);
    }
    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl WindowHandler for SplashState {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {
        self.exit = true;
    }
    fn configure(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        _: &Window,
        _: WindowConfigure,
        _: u32,
    ) {
        // First (and every) configure: (re)paint, which kicks the frame loop.
        self.draw(qh);
    }
}

impl OutputHandler for SplashState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ShmHandler for SplashState {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for SplashState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}

delegate_compositor!(SplashState);
delegate_output!(SplashState);
delegate_xdg_shell!(SplashState);
delegate_xdg_window!(SplashState);
delegate_shm!(SplashState);
delegate_registry!(SplashState);
