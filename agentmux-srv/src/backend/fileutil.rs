// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! File utilities: path normalization, MIME type detection, init script validation.
//! Port of Go's pkg/util/fileutil/.


use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

// ---- Constants ----

/// Windows symlink file attribute flags.
#[cfg(target_os = "windows")]
const WIN_FLAG_SOFTLINK: u32 = 0x8000; // FILE_ATTRIBUTE_REPARSE_POINT
#[cfg(target_os = "windows")]
const WIN_FLAG_JUNCTION: u32 = 0x80; // FILE_ATTRIBUTE_JUNCTION

/// System binary directories that indicate a command, not a script path.
const SYSTEM_BIN_DIRS: &[&str] = &[
    "/bin/",
    "/usr/bin/",
    "/usr/local/bin/",
    "/opt/bin/",
    "/sbin/",
    "/usr/sbin/",
];

// ---- Path utilities ----

/// Normalize a file path to an absolute path.
///
/// - Expands `~` to home directory
/// - Converts relative paths to absolute
/// - Preserves trailing slashes from the original input
pub fn fix_path(path: &str) -> Result<String, String> {
    let orig = path;
    let resolved = if let Some(rest) = path.strip_prefix('~') {
        let home = dirs::home_dir().ok_or_else(|| "cannot determine home directory".to_string())?;
        home.join(rest.trim_start_matches('/'))
            .to_string_lossy()
            .to_string()
    } else if !Path::new(path).is_absolute() {
        std::env::current_dir()
            .map_err(|e| format!("cannot get current dir: {}", e))?
            .join(path)
            .to_string_lossy()
            .to_string()
    } else {
        path.to_string()
    };

    // Preserve trailing slash from original
    if orig.ends_with('/') && !resolved.ends_with('/') {
        Ok(format!("{}/", resolved))
    } else {
        Ok(resolved)
    }
}

/// Check if a Windows symlink path points to a directory.
///
/// Uses file mode bits to detect reparse points and junctions.
#[cfg(target_os = "windows")]
pub fn win_symlink_dir(path: &str, mode_bits: u32) -> bool {
    let flags = mode_bits >> 12;

    if flags == WIN_FLAG_SOFTLINK {
        // Heuristic: if it has a file extension after the last /, treat as file
        let has_ext = path.rfind('.').map_or(false, |dot_pos| {
            path.rfind('/').map_or(true, |slash_pos| dot_pos > slash_pos)
        });
        !has_ext
    } else if flags == WIN_FLAG_JUNCTION {
        true
    } else {
        false
    }
}

#[cfg(not(target_os = "windows"))]
pub fn win_symlink_dir(_path: &str, _mode_bits: u32) -> bool {
    false
}

// ---- MIME type detection ----

/// Detect the MIME type of a file.
///
/// Detection order:
/// 1. Check file type (directory, pipe, device)
/// 2. Check static MIME type map by extension
/// 3. If `extended`, read first 512 bytes for content-based detection
/// 4. Returns empty string on detection failure
pub fn detect_mime_type(path: &str, extended: bool) -> String {
    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return String::new(),
    };

    if meta.is_dir() {
        return "directory".to_string();
    }

    // Check by extension first
    if let Some(ext) = Path::new(path).extension().and_then(|e| e.to_str()) {
        let ext_with_dot = format!(".{}", ext.to_lowercase());
        if let Some(mime) = STATIC_MIME_TYPE_MAP.get(ext_with_dot.as_str()) {
            return mime.to_string();
        }
    }

    // Empty files are text/plain
    if meta.len() == 0 {
        return "text/plain".to_string();
    }

    if !extended {
        return String::new();
    }

    // Content-based detection: read first 512 bytes
    match fs::read(path) {
        Ok(data) => {
            let sample = if data.len() > 512 { &data[..512] } else { &data };
            detect_content_type(sample)
        }
        Err(_) => String::new(),
    }
}

/// Detect MIME type by file extension only (fast path for directory listings).
pub fn detect_mime_type_by_extension(path: &str, is_dir: bool) -> String {
    if is_dir {
        return "directory".to_string();
    }
    if let Some(ext) = Path::new(path).extension().and_then(|e| e.to_str()) {
        let ext_with_dot = format!(".{}", ext.to_lowercase());
        if let Some(mime) = STATIC_MIME_TYPE_MAP.get(ext_with_dot.as_str()) {
            return mime.to_string();
        }
    }
    String::new()
}

/// Simple content-type detection from file header bytes.
///
/// Checks for common file signatures (magic bytes).
fn detect_content_type(data: &[u8]) -> String {
    if data.is_empty() {
        return String::new();
    }

    // Check known magic bytes
    if data.starts_with(b"%PDF") {
        return "application/pdf".to_string();
    }
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return "image/png".to_string();
    }
    if data.starts_with(b"\xff\xd8\xff") {
        return "image/jpeg".to_string();
    }
    if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        return "image/gif".to_string();
    }
    if data.starts_with(b"PK\x03\x04") {
        return "application/zip".to_string();
    }
    if data.starts_with(b"\x1f\x8b") {
        return "application/gzip".to_string();
    }
    if data.len() >= 4 && &data[..4] == b"\x7fELF" {
        return "application/x-executable".to_string();
    }
    if data.starts_with(b"RIFF") && data.len() >= 12 && &data[8..12] == b"WEBP" {
        return "image/webp".to_string();
    }
    if data.starts_with(b"<!DOCTYPE html") || data.starts_with(b"<html") {
        return "text/html".to_string();
    }

    // Check if content looks like text (no null bytes in first 512 bytes)
    let is_text = data
        .iter()
        .all(|&b| b >= 0x20 || matches!(b, b'\n' | b'\r' | b'\t'));
    if is_text {
        return "text/plain".to_string();
    }

    // application/octet-stream is considered a detection failure
    String::new()
}

// ---- Init script detection ----

/// Determine if a string is a path to a script file (vs inline script content).
///
/// Returns true if the input looks like a file path:
/// - Must not contain newlines
/// - Must not contain suspicious shell characters (`;`, `#`, `|`, etc.)
/// - Must not contain command-line flags (`--flag`)
/// - Must be an absolute path or start with `~/`
/// - Must not start with a system binary directory
pub fn is_init_script_path(input: &str) -> bool {
    if input.is_empty() || input.contains('\n') {
        return false;
    }

    // Reject strings with suspicious shell characters
    if input
        .bytes()
        .any(|b| matches!(b, b':' | b';' | b'#' | b'!' | b'&' | b'$' | b'\t' | b'%' | b'=' | b'"' | b'|' | b'>' | b'{' | b'}'))
    {
        return false;
    }

    // Reject strings with command-line flags
    if has_flag_pattern(input) {
        return false;
    }

    // Accept home directory paths
    if input.starts_with("~/") {
        return true;
    }

    // Must be absolute path
    if !Path::new(input).is_absolute() {
        return false;
    }

    // Reject system binary directories
    let normalized = input.replace('\\', "/");
    for bin_dir in SYSTEM_BIN_DIRS {
        if normalized.starts_with(bin_dir) {
            return false;
        }
    }

    true
}

/// Check if string contains a command-line flag pattern (` -x` or ` --xxx`).
fn has_flag_pattern(s: &str) -> bool {
    let bytes = s.as_bytes();
    for i in 0..bytes.len().saturating_sub(2) {
        if bytes[i] == b' ' && bytes[i + 1] == b'-' {
            // Match ` -x` (short flag) or ` --x` (long flag)
            if i + 2 < bytes.len() {
                if bytes[i + 2].is_ascii_alphanumeric() {
                    return true;
                }
                // Long flag: ` --x`
                if bytes[i + 2] == b'-'
                    && i + 3 < bytes.len()
                    && bytes[i + 3].is_ascii_alphanumeric()
                {
                    return true;
                }
            }
        }
    }
    false
}

// ---- Static MIME type map ----

/// Static map of file extensions to MIME types.
/// Contains 200+ common file extensions for code, media, documents, and archives.
static STATIC_MIME_TYPE_MAP: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
    let entries: &[(&str, &str)] = &[
        // Application types
        (".json", "application/json"),
        (".jsonl", "application/jsonl"),
        (".json5", "application/json5"),
        (".pdf", "application/pdf"),
        (".zip", "application/zip"),
        (".gz", "application/gzip"),
        (".tar", "application/x-tar"),
        (".tgz", "application/x-tar+gzip"),
        (".bz2", "application/x-bzip2"),
        (".xz", "application/x-xz"),
        (".7z", "application/x-7z-compressed"),
        (".rar", "application/x-rar-compressed"),
        (".jar", "application/java-archive"),
        (".war", "application/java-archive"),
        (".ear", "application/java-archive"),
        (".wasm", "application/wasm"),
        (".exe", "application/x-executable"),
        (".dll", "application/x-sharedlib"),
        (".so", "application/x-sharedlib"),
        (".dylib", "application/x-sharedlib"),
        (".dmg", "application/x-apple-diskimage"),
        (".iso", "application/x-iso9660-image"),
        (".deb", "application/x-debian-package"),
        (".rpm", "application/x-rpm"),
        (".msi", "application/x-msi"),
        (".sqlite", "application/x-sqlite3"),
        (".db", "application/x-sqlite3"),
        (".woff", "font/woff"),
        (".woff2", "font/woff2"),
        (".ttf", "font/ttf"),
        (".otf", "font/otf"),
        (".eot", "application/vnd.ms-fontobject"),
        // Document types
        (".xml", "application/xml"),
        (".xsl", "application/xml"),
        (".xslt", "application/xslt+xml"),
        (".rss", "application/rss+xml"),
        (".atom", "application/atom+xml"),
        (".svg", "image/svg+xml"),
        (".docx", "application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
        (".xlsx", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
        (".pptx", "application/vnd.openxmlformats-officedocument.presentationml.presentation"),
        (".doc", "application/msword"),
        (".xls", "application/vnd.ms-excel"),
        (".ppt", "application/vnd.ms-powerpoint"),
        (".odt", "application/vnd.oasis.opendocument.text"),
        (".ods", "application/vnd.oasis.opendocument.spreadsheet"),
        (".odp", "application/vnd.oasis.opendocument.presentation"),
        (".epub", "application/epub+zip"),
        (".rtf", "application/rtf"),
        (".csv", "text/csv"),
        (".tsv", "text/tab-separated-values"),
        // Image types
        (".png", "image/png"),
        (".jpg", "image/jpeg"),
        (".jpeg", "image/jpeg"),
        (".gif", "image/gif"),
        (".bmp", "image/bmp"),
        (".ico", "image/x-icon"),
        (".tiff", "image/tiff"),
        (".tif", "image/tiff"),
        (".webp", "image/webp"),
        (".avif", "image/avif"),
        (".heic", "image/heic"),
        (".heif", "image/heif"),
        (".psd", "image/vnd.adobe.photoshop"),
        (".ai", "application/postscript"),
        (".eps", "application/postscript"),
        // Audio types
        (".mp3", "audio/mpeg"),
        (".wav", "audio/wav"),
        (".ogg", "audio/ogg"),
        (".flac", "audio/flac"),
        (".aac", "audio/aac"),
        (".m4a", "audio/mp4"),
        (".wma", "audio/x-ms-wma"),
        (".opus", "audio/opus"),
        (".mid", "audio/midi"),
        (".midi", "audio/midi"),
        // Video types
        (".mp4", "video/mp4"),
        (".webm", "video/webm"),
        (".avi", "video/x-msvideo"),
        (".mov", "video/quicktime"),
        (".wmv", "video/x-ms-wmv"),
        (".flv", "video/x-flv"),
        (".mkv", "video/x-matroska"),
        (".m4v", "video/mp4"),
        (".ts", "video/mp2t"), // note: also TypeScript, but extension check alone is ambiguous
        (".ogv", "video/ogg"),
        // Text/code types
        (".txt", "text/plain"),
        (".md", "text/markdown"),
        (".markdown", "text/markdown"),
        (".html", "text/html"),
        (".htm", "text/html"),
        (".css", "text/css"),
        (".scss", "text/x-scss"),
        (".sass", "text/x-sass"),
        (".less", "text/x-less"),
        (".js", "text/javascript"),
        (".mjs", "text/javascript"),
        (".cjs", "text/javascript"),
        (".jsx", "text/jsx"),
        (".ts", "text/typescript"),
        (".tsx", "text/tsx"),
        (".py", "text/x-python"),
        (".rb", "text/x-ruby"),
        (".rs", "text/x-rustsrc"),
        (".go", "text/x-go"),
        (".java", "text/x-java"),
        (".kt", "text/x-kotlin"),
        (".kts", "text/x-kotlin"),
        (".scala", "text/x-scala"),
        (".c", "text/x-csrc"),
        (".h", "text/x-chdr"),
        (".cpp", "text/x-c++src"),
        (".cxx", "text/x-c++src"),
        (".cc", "text/x-c++src"),
        (".hpp", "text/x-c++hdr"),
        (".hxx", "text/x-c++hdr"),
        (".cs", "text/x-csharp"),
        (".swift", "text/x-swift"),
        (".m", "text/x-objectivec"),
        (".mm", "text/x-objectivec"),
        (".r", "text/x-r"),
        (".R", "text/x-r"),
        (".pl", "text/x-perl"),
        (".pm", "text/x-perl"),
        (".php", "text/x-php"),
        (".lua", "text/x-lua"),
        (".sh", "text/x-shellscript"),
        (".bash", "text/x-shellscript"),
        (".zsh", "text/x-shellscript"),
        (".fish", "text/x-shellscript"),
        (".ps1", "text/x-powershell"),
        (".psm1", "text/x-powershell"),
        (".bat", "text/x-batch"),
        (".cmd", "text/x-batch"),
        (".sql", "text/x-sql"),
        (".graphql", "text/x-graphql"),
        (".gql", "text/x-graphql"),
        (".proto", "text/x-protobuf"),
        (".tf", "text/x-terraform"),
        (".hcl", "text/x-hcl"),
        (".zig", "text/x-zig"),
        (".nim", "text/x-nim"),
        (".v", "text/x-v"),
        (".d", "text/x-d"),
        (".dart", "text/x-dart"),
        (".ex", "text/x-elixir"),
        (".exs", "text/x-elixir"),
        (".erl", "text/x-erlang"),
        (".hrl", "text/x-erlang"),
        (".hs", "text/x-haskell"),
        (".lhs", "text/x-haskell"),
        (".ml", "text/x-ocaml"),
        (".mli", "text/x-ocaml"),
        (".clj", "text/x-clojure"),
        (".cljs", "text/x-clojurescript"),
        (".lisp", "text/x-lisp"),
        (".el", "text/x-emacs-lisp"),
        (".vim", "text/x-vim"),
        (".asm", "text/x-asm"),
        (".s", "text/x-asm"),
        // Config/data files
        (".yaml", "text/yaml"),
        (".yml", "text/yaml"),
        (".toml", "text/toml"),
        (".ini", "text/x-ini"),
        (".cfg", "text/x-ini"),
        (".conf", "text/x-ini"),
        (".properties", "text/x-java-properties"),
        (".env", "text/x-dotenv"),
        (".gitignore", "text/x-gitignore"),
        (".dockerignore", "text/x-dockerignore"),
        (".editorconfig", "text/x-editorconfig"),
        (".lock", "text/plain"),
        (".log", "text/plain"),
        // Notebook/docs
        (".ipynb", "application/x-ipynb+json"),
        (".tex", "text/x-latex"),
        (".bib", "text/x-bibtex"),
        (".rst", "text/x-rst"),
        (".org", "text/x-org"),
        (".adoc", "text/asciidoc"),
        // Containerization/IaC
        (".dockerfile", "text/x-dockerfile"),
        (".vagrantfile", "text/x-ruby"),
        // Misc
        (".wasm", "application/wasm"),
        (".map", "application/json"),
    ];

    entries.iter().copied().collect()
});

/// Look up the MIME type for a file extension.
pub fn get_mime_type(extension: &str) -> Option<&'static str> {
    let ext = if extension.starts_with('.') {
        extension.to_lowercase()
    } else {
        format!(".{}", extension.to_lowercase())
    };
    STATIC_MIME_TYPE_MAP.get(ext.as_str()).copied()
}

// ---- Tests ----

#[cfg(test)]
mod tests {
    use super::*;

    // -- fix_path tests --

    #[test]
    fn test_fix_path_absolute() {
        let result = fix_path("/usr/bin/bash").unwrap();
        assert_eq!(result, "/usr/bin/bash");
    }

    #[test]
    fn test_fix_path_tilde() {
        let result = fix_path("~/Documents").unwrap();
        assert!(result.ends_with("Documents"));
        assert!(!result.starts_with('~'));
    }

    #[test]
    fn test_fix_path_trailing_slash() {
        let result = fix_path("/tmp/dir/").unwrap();
        assert!(result.ends_with('/'));
    }

    #[test]
    fn test_fix_path_no_trailing_slash() {
        let result = fix_path("/tmp/dir").unwrap();
        assert!(!result.ends_with('/'));
    }

    #[test]
    fn test_fix_path_relative() {
        let result = fix_path("relative/path").unwrap();
        assert!(Path::new(&result).is_absolute());
    }

    // -- MIME type tests --

    #[test]
    fn test_get_mime_type_known() {
        assert_eq!(get_mime_type(".json"), Some("application/json"));
        assert_eq!(get_mime_type(".py"), Some("text/x-python"));
        assert_eq!(get_mime_type(".rs"), Some("text/x-rustsrc"));
        assert_eq!(get_mime_type(".png"), Some("image/png"));
        assert_eq!(get_mime_type(".mp4"), Some("video/mp4"));
    }

    #[test]
    fn test_get_mime_type_case_insensitive() {
        assert_eq!(get_mime_type(".JSON"), Some("application/json"));
        assert_eq!(get_mime_type(".Py"), Some("text/x-python"));
    }

    #[test]
    fn test_get_mime_type_without_dot() {
        assert_eq!(get_mime_type("json"), Some("application/json"));
        assert_eq!(get_mime_type("py"), Some("text/x-python"));
    }

    #[test]
    fn test_get_mime_type_unknown() {
        assert_eq!(get_mime_type(".xyz123unknown"), None);
    }

    #[test]
    fn test_detect_mime_type_by_extension() {
        assert_eq!(detect_mime_type_by_extension("/foo/bar.json", false), "application/json");
        assert_eq!(detect_mime_type_by_extension("/foo/bar/", true), "directory");
        assert_eq!(detect_mime_type_by_extension("/foo/unknown", false), "");
    }

    // -- content detection tests --

    #[test]
    fn test_detect_content_type_png() {
        assert_eq!(
            detect_content_type(b"\x89PNG\r\n\x1a\n"),
            "image/png"
        );
    }

    #[test]
    fn test_detect_content_type_jpeg() {
        assert_eq!(detect_content_type(b"\xff\xd8\xff\xe0"), "image/jpeg");
    }

    #[test]
    fn test_detect_content_type_pdf() {
        assert_eq!(detect_content_type(b"%PDF-1.4"), "application/pdf");
    }

    #[test]
    fn test_detect_content_type_text() {
        assert_eq!(
            detect_content_type(b"Hello, world!\nThis is text."),
            "text/plain"
        );
    }

    #[test]
    fn test_detect_content_type_binary_control_chars() {
        // Data with null bytes and control chars should NOT be detected as text
        assert_eq!(detect_content_type(b"hello\x00world"), "");
        assert_eq!(detect_content_type(b"\x01\x02\x03"), "");
    }

    #[test]
    fn test_detect_content_type_empty() {
        assert_eq!(detect_content_type(b""), "");
    }

    // -- is_init_script_path tests --

    #[test]
    fn test_is_init_script_path_absolute() {
        assert!(is_init_script_path("/home/user/.bashrc"));
        assert!(is_init_script_path("/etc/profile.d/custom.sh"));
    }

    #[test]
    fn test_is_init_script_path_home() {
        assert!(is_init_script_path("~/.bashrc"));
        assert!(is_init_script_path("~/scripts/init.sh"));
    }

    #[test]
    fn test_is_init_script_path_system_bin() {
        assert!(!is_init_script_path("/bin/bash"));
        assert!(!is_init_script_path("/usr/bin/env"));
        assert!(!is_init_script_path("/usr/local/bin/node"));
    }

    #[test]
    fn test_is_init_script_path_inline_script() {
        assert!(!is_init_script_path("echo hello"));
        assert!(!is_init_script_path("export FOO=bar"));
        assert!(!is_init_script_path("cat /etc/hosts | grep localhost"));
    }

    #[test]
    fn test_is_init_script_path_with_flags() {
        assert!(!is_init_script_path("/usr/bin/python3 --version"));
        assert!(!is_init_script_path("node -e 'code'"));
    }

    #[test]
    fn test_is_init_script_path_empty() {
        assert!(!is_init_script_path(""));
    }

    #[test]
    fn test_is_init_script_path_newlines() {
        assert!(!is_init_script_path("line1\nline2"));
    }

    #[test]
    fn test_is_init_script_path_relative() {
        assert!(!is_init_script_path("relative/path.sh"));
    }

    // -- has_flag_pattern tests --

    #[test]
    fn test_has_flag_pattern() {
        assert!(has_flag_pattern("cmd --flag"));
        assert!(has_flag_pattern("cmd -v"));
        assert!(!has_flag_pattern("/path/to/file"));
        assert!(!has_flag_pattern("no-flags-here"));
    }

    // -- win_symlink_dir tests --

    #[test]
    fn test_win_symlink_dir() {
        // On non-Windows, always returns false
        assert!(!win_symlink_dir("/some/path", 0));
    }
}
