// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `SystemDrive` for a Windows srv started with a bare environment (a headless
//! srv under a service manager, `env -i`). The shell's icon and thumbnail caches
//! live in `%SystemDrive%\ProgramData\Microsoft\Windows\Caches`, and the shell
//! expands that name from the process environment, not from the registry. With
//! the variable missing, the text stays as it is and the shell makes a folder
//! literally called `%SystemDrive%` in the current directory (#2524). srv sets
//! the variable itself, from the Windows folder, so every child it starts
//! (the file-icon helper first) inherits a real drive.

/// The drive of a Windows folder path, as `C:`: a letter, a colon, nothing else.
/// `None` for a path that doesn't start that way (a UNC path, an empty string).
pub fn drive_of(path: &str) -> Option<String> {
    let mut chars = path.chars();
    let letter = chars.next().filter(char::is_ascii_alphabetic)?;
    if chars.next() != Some(':') {
        return None;
    }
    Some(format!("{}:", letter.to_ascii_uppercase()))
}

/// Give this process a `SystemDrive` when it has none. Call it before any
/// thread or child exists. A value that is set, whatever it holds, is left alone.
pub fn ensure() {
    if std::env::var_os("SystemDrive").is_some() {
        return;
    }
    if let Some(drive) = system_drive() {
        std::env::set_var("SystemDrive", drive);
    }
}

/// The drive Windows is installed on, from the Windows API (never from the
/// environment, which is what is missing).
#[cfg(windows)]
pub fn system_drive() -> Option<String> {
    use windows_sys::Win32::System::SystemInformation::GetSystemWindowsDirectoryW;
    let mut buf = [0u16; 260];
    // SAFETY: `buf` is valid for `buf.len()` u16s; the call writes at most that many.
    let len = unsafe { GetSystemWindowsDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if len == 0 || len > buf.len() {
        return None;
    }
    drive_of(&String::from_utf16_lossy(&buf[..len]))
}

#[cfg(not(windows))]
pub fn system_drive() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::drive_of;

    #[test]
    fn a_windows_folder_gives_its_drive_in_capitals() {
        assert_eq!(drive_of(r"C:\Windows").as_deref(), Some("C:"));
        assert_eq!(drive_of(r"d:\WINNT").as_deref(), Some("D:"));
        assert_eq!(drive_of("E:").as_deref(), Some("E:"));
    }

    #[test]
    fn a_path_without_a_drive_gives_none() {
        for p in ["", r"\\server\share\Windows", "Windows", "1:\\x", "C", "%SystemDrive%"] {
            assert_eq!(drive_of(p), None, "{p:?}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn the_windows_api_names_a_real_drive() {
        let drive = super::system_drive().expect("the Windows folder has a drive");
        assert_eq!(drive.len(), 2);
        assert!(std::path::Path::new(&format!("{drive}\\")).exists(), "{drive} is not a drive here");
    }
}
