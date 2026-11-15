// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the page may ask the icon route for. Every field is enumerated or
//! validated here, so nothing the page sends reaches the operating system as
//! anything but a file type and a size (spec §4.1, §6).

/// The sizes an icon can be asked for, in pixels: CSS size times the display
/// scale and the pane's zoom, snapped up to one of these by the page.
pub const PX_SIZES: [u32; 10] = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256];

/// The longest extension accepted (without its dot).
pub const MAX_EXT_LEN: usize = 24;

/// What an icon is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A file of some type, by extension. An empty extension is a file without one.
    File,
    /// A folder (the generic one; a folder with its own icon is a later phase).
    Dir,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::File => "file",
            Kind::Dir => "dir",
        }
    }
}

/// A validated request for the icon of a file type.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct IconRequest {
    pub kind: Kind,
    /// Lower case, no dot, `[a-z0-9_+-]`; empty for a folder or a file with no extension.
    pub ext: String,
    pub px: u32,
}

impl IconRequest {
    /// Parse the route's three query parameters; the error is the sentence the 400 carries.
    pub fn parse(kind: Option<&str>, ext: Option<&str>, px: Option<&str>) -> Result<Self, &'static str> {
        let kind = match kind {
            Some("file") => Kind::File,
            Some("dir") => Kind::Dir,
            _ => return Err("kind must be file or dir"),
        };
        let ext = ext.unwrap_or("");
        if ext.len() > MAX_EXT_LEN
            || !ext.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'+' | b'-'))
        {
            return Err("ext must be up to 24 letters, digits, _, + or -");
        }
        if kind == Kind::Dir && !ext.is_empty() {
            return Err("a dir has no ext");
        }
        let px = px.and_then(|p| p.parse::<u32>().ok()).filter(|p| PX_SIZES.contains(p)).ok_or("px is not an offered size")?;
        Ok(IconRequest { kind, ext: ext.to_ascii_lowercase(), px })
    }

    /// The name the shell is asked about: it never exists, the shell only reads its type.
    pub fn shell_name(&self) -> String {
        match (self.kind, self.ext.as_str()) {
            (Kind::Dir, _) => "folder".to_string(),
            (Kind::File, "") => "file".to_string(),
            (Kind::File, ext) => format!("file.{ext}"),
        }
    }
}

/// The native icon list that covers `px`, as (list number, its pixel size): the
/// smallest one at least that big, so the picture is only ever scaled down.
/// 0 is the shell's large list (32), 1 small (16), 2 extra large (48), 4 jumbo (256).
pub fn native_list(px: u32) -> (i32, u32) {
    match px {
        0..=16 => (1, 16),
        17..=32 => (0, 32),
        33..=48 => (2, 48),
        _ => (4, 256),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(kind: &str, ext: &str, px: &str) -> Result<IconRequest, &'static str> {
        IconRequest::parse(Some(kind), Some(ext), Some(px))
    }

    #[test]
    fn a_good_request_is_lowercased_and_named_for_the_shell() {
        let r = parse("file", "RS", "16").unwrap();
        assert_eq!((r.kind, r.ext.as_str(), r.px), (Kind::File, "rs", 16));
        assert_eq!(r.shell_name(), "file.rs");
        assert_eq!(parse("file", "", "32").unwrap().shell_name(), "file");
        assert_eq!(parse("dir", "", "48").unwrap().shell_name(), "folder");
    }

    #[test]
    fn an_extension_cannot_carry_a_path_or_a_dot() {
        for bad in ["..", "a.b", "a/b", "a\\b", "a b", "a:b", "é", "x\0", &"a".repeat(25)] {
            assert!(parse("file", bad, "16").is_err(), "{bad:?} must be refused");
        }
        assert!(parse("file", "c++", "16").is_ok());
        assert!(parse("file", "7z", "16").is_ok());
        assert!(parse("file", "tar-gz_x", "16").is_ok());
    }

    #[test]
    fn only_the_offered_sizes_and_kinds_are_accepted() {
        for px in PX_SIZES {
            assert!(parse("file", "rs", &px.to_string()).is_ok());
        }
        for bad in ["0", "15", "17", "257", "-16", "16.0", "", "1000000"] {
            assert!(parse("file", "rs", bad).is_err(), "px {bad:?}");
        }
        assert!(IconRequest::parse(Some("file"), Some("rs"), None).is_err());
        assert!(IconRequest::parse(None, Some("rs"), Some("16")).is_err());
        assert!(parse("drive", "", "16").is_err());
        assert!(parse("dir", "rs", "16").is_err(), "a folder has no extension");
    }

    #[test]
    fn a_picture_is_taken_from_the_smallest_list_that_is_big_enough() {
        let got: Vec<(u32, u32)> = PX_SIZES.iter().map(|&p| (p, native_list(p).1)).collect();
        assert_eq!(
            got,
            [(16, 16), (20, 32), (24, 32), (32, 32), (40, 48), (48, 48), (64, 256), (96, 256), (128, 256), (256, 256)]
        );
        assert!(PX_SIZES.iter().all(|&p| native_list(p).1 >= p), "never scaled up");
    }
}
