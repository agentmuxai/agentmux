// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The OS name the frontend uses (Node's `process.platform` values), from
//! Rust's `std::env::consts::OS`.

/// `"darwin"`, `"win32"`, `"linux"`, ... for the OS this process runs on.
pub fn platform_name() -> &'static str {
    from_rust_os(std::env::consts::OS)
}

fn from_rust_os(os: &'static str) -> &'static str {
    match os {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_rust_names_to_the_frontend_names() {
        assert_eq!(from_rust_os("macos"), "darwin");
        assert_eq!(from_rust_os("windows"), "win32");
        assert_eq!(from_rust_os("linux"), "linux");
        assert_eq!(from_rust_os("freebsd"), "freebsd");
    }
}
