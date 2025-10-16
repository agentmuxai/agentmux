fn main() {
    // Embed Windows resources (splash screen)
    #[cfg(windows)]
    {
        embed_resource::compile("resources/splash.rc", embed_resource::NONE);
    }

    tauri_build::build()
}
