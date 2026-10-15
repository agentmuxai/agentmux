// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Same supportedOS manifest as the host (crates/cef/build.rs) — the launcher
// is the top-level exe and child processes can inherit OS-version behavior, so
// keep both manifested to report the true Windows version. See the host
// build.rs for the GPU-init rationale.
#[cfg(target_os = "windows")]
const WINDOWS_APP_MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{e2011457-1546-43c5-a5fe-008deee3d3f0}"/>
      <supportedOS Id="{35138b9a-5d96-4fbd-8e2d-a2440225f93a}"/>
      <supportedOS Id="{4a2f28e3-53b9-4441-ba9c-d69d4a4a6e38}"/>
      <supportedOS Id="{1f676c76-80e1-4239-95bb-83d0f6d0da78}"/>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
    </application>
  </compatibility>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
</assembly>
"#;

fn main() {
    // AGENTMUX_BUILD_LABEL is injected by package.sh and includes a per-build
    // timestamp stamp. Tracking it here forces a recompile of agentmux-launcher
    // (only) when the label changes between local builds, so the baked
    // option_env!("AGENTMUX_BUILD_LABEL") in main.rs reflects the new stamp.
    // We do NOT track this in crates/common/build.rs because common is a
    // foundational crate — recompiling it every build destroys incremental caching.
    println!("cargo:rerun-if-env-changed=AGENTMUX_BUILD_LABEL");

    #[cfg(target_os = "windows")]
    {
        let version = std::env::var("CARGO_PKG_VERSION").unwrap();
        let mut res = winres::WindowsResource::new();
        res.set("FileDescription", &format!("AgentMux v{}", version));
        res.set("ProductName", "AgentMux");
        res.set("CompanyName", "AgentMux Corp.");
        res.set("InternalName", "agentmux-launcher");
        let icon_path = std::path::Path::new("../cef/resources/win/agentmux.ico");
        if icon_path.exists() {
            res.set_icon(icon_path.to_str().unwrap());
        }
        res.set_manifest(WINDOWS_APP_MANIFEST);
        res.compile().expect("winres compile failed");

        // Read the brain logo PNG's header (transparent background, RGBA,
        // 256×256) so splash.rs gets its dimensions as consts. The pixels
        // themselves are NOT decoded here: splash.rs embeds the PNG (about
        // 15 KB) and decodes it once on the first frame, instead of baking
        // a 256 KB raw bitmap into the exe. The runtime `png` decoder is
        // already linked for the tray icon, so this costs no new code.
        let png_path = std::path::Path::new("resources/brain.png");
        println!("cargo:rerun-if-changed={}", png_path.display());
        let f = std::fs::File::open(png_path).expect("resources/brain.png not found");
        let decoder = png::Decoder::new(f);
        let reader = decoder.read_info().expect("png header");
        let info = reader.info().clone();
        assert_eq!(
            info.color_type,
            png::ColorType::Rgba,
            "splash brain.png must be RGBA (transparent background)"
        );
        assert_eq!(
            info.bit_depth,
            png::BitDepth::Eight,
            "splash brain.png must be 8-bit"
        );
        let out_dir = std::env::var("OUT_DIR").unwrap();

        // Emit dimensions as Rust consts so splash.rs `include!`-s them
        // and stays in lockstep with whatever brain.png actually contains.
        // Hardcoded values would silently desync if the PNG is regenerated
        // at a different size.
        let dims_rs = format!(
            "pub const BRAIN_W: i32 = {};\npub const BRAIN_H: i32 = {};\n",
            info.width, info.height
        );
        let dims_path = std::path::Path::new(&out_dir).join("brain_dims.rs");
        std::fs::write(&dims_path, dims_rs).expect("write brain_dims.rs");
    }

    #[cfg(target_os = "linux")]
    {
        // Decode the brain logo PNG (RGBA, transparent background) to raw
        // *straight* (non-pre-multiplied) RGBA8 bytes for the Linux splash
        // backends (X11 pixmap + Wayland wl_shm). The backends alpha-blend the
        // brain over the solid backdrop themselves each frame, so they need the
        // un-pre-multiplied source. Decoding at compile time keeps a PNG decoder
        // out of the launcher runtime binary (mirrors the Windows arm above).
        let png_path = std::path::Path::new("resources/brain.png");
        println!("cargo:rerun-if-changed={}", png_path.display());
        let f = std::fs::File::open(png_path).expect("resources/brain.png not found");
        let decoder = png::Decoder::new(f);
        let mut reader = decoder.read_info().expect("png header");
        let info = reader.info().clone();
        assert_eq!(
            info.color_type,
            png::ColorType::Rgba,
            "splash brain.png must be RGBA (transparent background)"
        );
        assert_eq!(
            info.bit_depth,
            png::BitDepth::Eight,
            "splash brain.png must be 8-bit"
        );
        let mut buf = vec![0u8; reader.output_buffer_size()];
        reader.next_frame(&mut buf).expect("png decode");

        let out_dir = std::env::var("OUT_DIR").unwrap();
        let rgba_path = std::path::Path::new(&out_dir).join("brain_rgba.bin");
        std::fs::write(&rgba_path, &buf).expect("write brain_rgba.bin");

        // Same brain_dims.rs the X11/Wayland backends `include!`, in lockstep
        // with the actual PNG size.
        let dims_rs = format!(
            "pub const BRAIN_W: i32 = {};\npub const BRAIN_H: i32 = {};\n",
            info.width, info.height
        );
        let dims_path = std::path::Path::new(&out_dir).join("brain_dims.rs");
        std::fs::write(&dims_path, dims_rs).expect("write brain_dims.rs");
    }
}
