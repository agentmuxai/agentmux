fn main() {
    #[cfg(target_os = "windows")]
    {
        let version = std::env::var("CARGO_PKG_VERSION").unwrap();
        let mut res = winres::WindowsResource::new();
        res.set("FileDescription", &format!("AgentMux v{}", version));
        res.set("ProductName", "AgentMux");
        res.set("CompanyName", "AgentMux");
        res.set("InternalName", "agentmux-launcher");
        let icon_path = std::path::Path::new("../agentmux-cef/resources/win/agentmux.ico");
        if icon_path.exists() {
            res.set_icon(icon_path.to_str().unwrap());
        }
        res.compile().expect("winres compile failed");

        // Decode the brain logo PNG (transparent background, 256×256) to
        // raw BGRA pre-multiplied bytes for the splash renderer. The
        // bytes get `include_bytes!`-d into splash.rs — no PNG decoder
        // shipped in the runtime binary.
        //
        // Pre-multiplied alpha is required by `UpdateLayeredWindow`'s
        // BLENDFUNCTION (AC_SRC_ALPHA): each channel must already be
        // `channel * alpha / 255`. Doing the multiplication here once
        // at compile time avoids 65k mul-then-div ops per frame on the
        // splash thread.
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

        // Convert RGBA straight → BGRA pre-multiplied, in place.
        for px in buf.chunks_exact_mut(4) {
            let (r, g, b, a) = (px[0], px[1], px[2], px[3]);
            let pre = |c: u8| ((c as u16 * a as u16 + 127) / 255) as u8;
            px[0] = pre(b);
            px[1] = pre(g);
            px[2] = pre(r);
            px[3] = a;
        }

        let out_dir = std::env::var("OUT_DIR").unwrap();
        let bgra_path = std::path::Path::new(&out_dir).join("brain_bgra.bin");
        std::fs::write(&bgra_path, &buf).expect("write brain_bgra.bin");

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
}
