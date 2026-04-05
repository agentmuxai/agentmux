fn main() {
    #[cfg(target_os = "windows")]
    {
        let version = std::env::var("CARGO_PKG_VERSION").unwrap();
        let mut res = winres::WindowsResource::new();
        res.set("FileDescription", &format!("AgentMux Shell v{}", version));
        res.set("ProductName", "AgentMux");
        res.set("CompanyName", "AgentMux");
        res.set("InternalName", "wsh");
        res.compile().expect("winres compile failed");
    }
}
