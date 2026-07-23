// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Linux-only GPU capability probing — drives ANGLE backend selection
// in `on_before_command_line_processing` (see the parent `app` module's
// `wrap_app!` block). Split out of `app.rs` (now `app/mod.rs`).

/// GPU capability tier, measured once at startup to drive ANGLE backend
/// selection. Precedence: hardware Vulkan → hardware GL → software (SwiftShader).
/// ANGLE is only retargeted, never bypassed. See the GPU block in
/// `on_before_command_line_processing` and
/// docs/specs/SPEC_LINUX_GPU_BACKEND_PRECEDENCE_2026_06_13.md.
#[cfg(target_os = "linux")]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum GpuTier {
    /// A hardware Vulkan device is present — leave Chromium's default (Vulkan).
    HwVulkan,
    /// No hardware Vulkan but a DRM render node exists — route ANGLE to GL and
    /// override the GPU blocklist (the VMware/SVGA3D case).
    HwGl,
    /// Neither — leave Chromium's default (software SwiftShader).
    Software,
}

#[cfg(target_os = "linux")]
impl GpuTier {
    fn as_str(self) -> &'static str {
        match self {
            GpuTier::HwVulkan => "hw-vulkan",
            GpuTier::HwGl => "hw-gl",
            GpuTier::Software => "software",
        }
    }
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "hw-vulkan" => Some(GpuTier::HwVulkan),
            "hw-gl" => Some(GpuTier::HwGl),
            "software" => Some(GpuTier::Software),
            _ => None,
        }
    }
}

/// Resolve the GPU tier once. `on_before_command_line_processing` runs per
/// process; the browser process (which starts first, before any child is
/// spawned) finds `AGENTMUX_GPU_TIER` unset, probes the hardware, and publishes
/// the result. Child processes (gpu/renderer/utility) inherit that env var and
/// read it back — so the `VkInstance` probe runs exactly once, in the browser.
#[cfg(target_os = "linux")]
pub(crate) fn detect_gpu_tier() -> GpuTier {
    if let Ok(v) = std::env::var("AGENTMUX_GPU_TIER") {
        if let Some(t) = GpuTier::from_str(&v) {
            return t;
        }
    }
    let tier = if has_hardware_vulkan() {
        GpuTier::HwVulkan
    } else if has_drm_render_node() {
        GpuTier::HwGl
    } else {
        GpuTier::Software
    };
    // Publish for child processes (inherited through the environment on spawn).
    std::env::set_var("AGENTMUX_GPU_TIER", tier.as_str());
    tracing::info!(tier = tier.as_str(), "resolved GPU tier for ANGLE selection");
    tier
}

/// True if a *hardware* Vulkan device is present. Enumerates Vulkan physical
/// devices and accepts any whose `device_type` is not `CPU` — llvmpipe/lavapipe/
/// SwiftShader all report `CPU`. Fully defensive: any load/create/enumerate
/// failure ⇒ false (we then fall through to the GL check).
#[cfg(target_os = "linux")]
fn has_hardware_vulkan() -> bool {
    use ash::vk;
    let entry = match unsafe { ash::Entry::load() } {
        Ok(e) => e,
        Err(_) => return false,
    };
    let app_info = vk::ApplicationInfo::default().api_version(vk::API_VERSION_1_0);
    let create_info = vk::InstanceCreateInfo::default().application_info(&app_info);
    let instance = match unsafe { entry.create_instance(&create_info, None) } {
        Ok(i) => i,
        Err(_) => return false,
    };
    let has_hw = unsafe { instance.enumerate_physical_devices() }
        .map(|devices| {
            devices.iter().any(|&d| {
                unsafe { instance.get_physical_device_properties(d) }.device_type
                    != vk::PhysicalDeviceType::CPU
            })
        })
        .unwrap_or(false);
    unsafe { instance.destroy_instance(None) };
    has_hw
}

/// True if a DRM render node (`/dev/dri/renderD*`) exists — a kernel GPU with a
/// render node, i.e. a real hardware GL path (vmwgfx on VMware, i915/amdgpu/
/// nvidia on bare metal). Heuristic; the spec's §7 upgrade path tightens this to
/// a `GL_RENDERER` software-marker check.
#[cfg(target_os = "linux")]
fn has_drm_render_node() -> bool {
    std::fs::read_dir("/dev/dri")
        .map(|rd| {
            rd.flatten()
                .any(|e| e.file_name().to_string_lossy().starts_with("renderD"))
        })
        .unwrap_or(false)
}
