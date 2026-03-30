fn main() {
    #[cfg(target_os = "windows")]
    {
        // Use the same icon as agentmux-cef
        let icon_path = std::path::Path::new("../agentmux-cef/resources/win/agentmux.ico");
        if icon_path.exists() {
            let _ = winres::WindowsResource::new()
                .set_icon(icon_path.to_str().unwrap())
                .compile();
        }
    }
}
