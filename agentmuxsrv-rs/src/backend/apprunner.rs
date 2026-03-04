// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! WaveApp framework types.
//! Port of Go's pkg/waveapp/.
//!
//! Provides the application configuration types, file handler options,
//! and streaming response chunking used by VDOM-based applications.
//! The runtime (Client, RPC communication, component registration)
//! is deferred until the sidecar is replaced.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};

use super::vdom::{VDomBackendOpts, VDomTargetToolbar};

// ---- Constants ----

/// Maximum chunk size for streaming responses (64KB).
pub const MAX_CHUNK_SIZE: usize = 64 * 1024;

/// Default root component name.
pub const DEFAULT_ROOT_COMPONENT: &str = "App";

/// Default new block flag.
pub const DEFAULT_NEW_BLOCK_FLAG: &str = "n";

// ---- Application Options ----

/// Application initialization options.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppOpts {
    /// Close the application on Ctrl+C.
    #[serde(default)]
    pub close_on_ctrl_c: bool,

    /// Enable global keyboard event capture.
    #[serde(default)]
    pub global_keyboard_events: bool,

    /// Global CSS styles (raw CSS bytes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub global_styles: Option<Vec<u8>>,

    /// Root component name (defaults to "App").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_component_name: Option<String>,

    /// Whether to open in a new block.
    #[serde(default)]
    pub target_new_block: bool,

    /// Toolbar target configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_toolbar: Option<VDomTargetToolbar>,
}

impl AppOpts {
    /// Get the root component name, defaulting to "App".
    pub fn root_component(&self) -> &str {
        self.root_component_name
            .as_deref()
            .unwrap_or(DEFAULT_ROOT_COMPONENT)
    }

    /// Convert to VDomBackendOpts for the protocol.
    pub fn to_backend_opts(&self) -> VDomBackendOpts {
        VDomBackendOpts {
            closeonctrlc: if self.close_on_ctrl_c {
                Some(true)
            } else {
                None
            },
            globalkeyboardevents: if self.global_keyboard_events {
                Some(true)
            } else {
                None
            },
            globalstyles: self
                .global_styles
                .as_ref()
                .and_then(|s| String::from_utf8(s.clone()).ok()),
        }
    }
}

// ---- File Handler ----

/// Options for serving file content.
#[derive(Debug, Clone, Default)]
pub struct FileHandlerOption {
    /// Optional file path to serve.
    pub file_path: Option<String>,
    /// Optional raw byte content.
    pub data: Option<Vec<u8>>,
    /// Optional MIME type override.
    pub mime_type: Option<String>,
    /// Optional ETag for caching.
    pub etag: Option<String>,
}

impl FileHandlerOption {
    /// Create from raw bytes with optional MIME type.
    pub fn from_bytes(data: Vec<u8>, mime_type: Option<&str>) -> Self {
        Self {
            data: Some(data),
            mime_type: mime_type.map(|s| s.to_string()),
            ..Default::default()
        }
    }

    /// Create from a file path.
    pub fn from_path(path: &str) -> Self {
        Self {
            file_path: Some(path.to_string()),
            ..Default::default()
        }
    }

    /// Determine the MIME type for this file option.
    ///
    /// Priority: explicit mime_type > extension-based detection > "application/octet-stream".
    pub fn determine_mime_type(&self) -> String {
        if let Some(ref mt) = self.mime_type {
            return mt.clone();
        }

        if let Some(ref path) = self.file_path {
            let ext = path.rsplit('.').next().unwrap_or("");
            if !ext.is_empty() {
                if let Some(mime) = crate::backend::fileutil::get_mime_type(ext) {
                    return mime.to_string();
                }
            }
        }

        "application/octet-stream".to_string()
    }
}

// ---- Streaming Response ----

/// A buffer for chunked response transmission.
///
/// Accumulates data and yields chunks of MAX_CHUNK_SIZE for efficient
/// wire transmission.
pub struct StreamingBuffer {
    buffer: Vec<u8>,
    chunks: Vec<Vec<u8>>,
    header_sent: bool,
    status_code: u16,
    headers: Vec<(String, String)>,
}

impl StreamingBuffer {
    /// Create a new streaming buffer.
    pub fn new() -> Self {
        Self {
            buffer: Vec::new(),
            chunks: Vec::new(),
            header_sent: false,
            status_code: 200,
            headers: Vec::new(),
        }
    }

    /// Set the HTTP status code.
    pub fn set_status(&mut self, code: u16) {
        self.status_code = code;
    }

    /// Get the status code.
    pub fn status_code(&self) -> u16 {
        self.status_code
    }

    /// Add a response header.
    pub fn add_header(&mut self, name: &str, value: &str) {
        self.headers.push((name.to_string(), value.to_string()));
    }

    /// Get all headers.
    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    /// Write data to the buffer.
    ///
    /// When the internal buffer reaches MAX_CHUNK_SIZE, a chunk is
    /// automatically flushed.
    pub fn write(&mut self, data: &[u8]) {
        self.buffer.extend_from_slice(data);

        while self.buffer.len() >= MAX_CHUNK_SIZE {
            let chunk: Vec<u8> = self.buffer.drain(..MAX_CHUNK_SIZE).collect();
            self.chunks.push(chunk);
        }
    }

    /// Flush the remaining buffer as a final chunk.
    pub fn close(&mut self) {
        if !self.buffer.is_empty() {
            let chunk = std::mem::take(&mut self.buffer);
            self.chunks.push(chunk);
        }
    }

    /// Take all accumulated chunks.
    pub fn take_chunks(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.chunks)
    }

    /// Check if any chunks are ready.
    pub fn has_chunks(&self) -> bool {
        !self.chunks.is_empty()
    }

    /// Check if headers have been sent.
    pub fn header_sent(&self) -> bool {
        self.header_sent
    }

    /// Mark headers as sent.
    pub fn mark_header_sent(&mut self) {
        self.header_sent = true;
    }
}

impl Default for StreamingBuffer {
    fn default() -> Self {
        Self::new()
    }
}

// ---- Tests ----

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_opts_defaults() {
        let opts = AppOpts::default();
        assert!(!opts.close_on_ctrl_c);
        assert!(!opts.global_keyboard_events);
        assert_eq!(opts.root_component(), DEFAULT_ROOT_COMPONENT);
    }

    #[test]
    fn test_app_opts_custom_root() {
        let opts = AppOpts {
            root_component_name: Some("MyApp".to_string()),
            ..Default::default()
        };
        assert_eq!(opts.root_component(), "MyApp");
    }

    #[test]
    fn test_app_opts_to_backend_opts() {
        let opts = AppOpts {
            close_on_ctrl_c: true,
            global_keyboard_events: true,
            global_styles: Some(b".body { color: red; }".to_vec()),
            ..Default::default()
        };

        let backend = opts.to_backend_opts();
        assert_eq!(backend.closeonctrlc, Some(true));
        assert_eq!(backend.globalkeyboardevents, Some(true));
        assert!(backend.globalstyles.unwrap().contains("color: red"));
    }

    #[test]
    fn test_app_opts_to_backend_opts_minimal() {
        let opts = AppOpts::default();
        let backend = opts.to_backend_opts();
        assert!(backend.closeonctrlc.is_none());
        assert!(backend.globalkeyboardevents.is_none());
        assert!(backend.globalstyles.is_none());
    }

    #[test]
    fn test_app_opts_serde() {
        let opts = AppOpts {
            close_on_ctrl_c: true,
            target_new_block: true,
            ..Default::default()
        };

        let json = serde_json::to_string(&opts).unwrap();
        let parsed: AppOpts = serde_json::from_str(&json).unwrap();
        assert!(parsed.close_on_ctrl_c);
        assert!(parsed.target_new_block);
    }

    #[test]
    fn test_file_handler_from_bytes() {
        let opt = FileHandlerOption::from_bytes(b"<html></html>".to_vec(), Some("text/html"));
        assert_eq!(opt.mime_type.as_deref(), Some("text/html"));
        assert_eq!(opt.data.unwrap(), b"<html></html>");
    }

    #[test]
    fn test_file_handler_from_path() {
        let opt = FileHandlerOption::from_path("/app/styles.css");
        assert_eq!(opt.file_path.as_deref(), Some("/app/styles.css"));
        assert!(opt.data.is_none());
    }

    #[test]
    fn test_file_handler_determine_mime() {
        let opt = FileHandlerOption {
            mime_type: Some("text/plain".to_string()),
            ..Default::default()
        };
        assert_eq!(opt.determine_mime_type(), "text/plain");
    }

    #[test]
    fn test_file_handler_determine_mime_from_ext() {
        let opt = FileHandlerOption::from_path("/app/style.css");
        assert_eq!(opt.determine_mime_type(), "text/css");
    }

    #[test]
    fn test_file_handler_determine_mime_fallback() {
        let opt = FileHandlerOption::default();
        assert_eq!(opt.determine_mime_type(), "application/octet-stream");
    }

    #[test]
    fn test_streaming_buffer_small_write() {
        let mut buf = StreamingBuffer::new();
        buf.write(b"hello world");
        buf.close();

        let chunks = buf.take_chunks();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], b"hello world");
    }

    #[test]
    fn test_streaming_buffer_large_write() {
        let mut buf = StreamingBuffer::new();
        // Write more than one chunk
        let data = vec![0u8; MAX_CHUNK_SIZE + 100];
        buf.write(&data);
        buf.close();

        let chunks = buf.take_chunks();
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), MAX_CHUNK_SIZE);
        assert_eq!(chunks[1].len(), 100);
    }

    #[test]
    fn test_streaming_buffer_multiple_writes() {
        let mut buf = StreamingBuffer::new();
        let half = MAX_CHUNK_SIZE / 2;

        buf.write(&vec![1u8; half]);
        assert!(!buf.has_chunks()); // Not enough for a full chunk

        buf.write(&vec![2u8; half + 100]);
        assert!(buf.has_chunks()); // Now we have a full chunk

        buf.close();
        let chunks = buf.take_chunks();
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), MAX_CHUNK_SIZE);
        assert_eq!(chunks[1].len(), 100);
    }

    #[test]
    fn test_streaming_buffer_empty() {
        let mut buf = StreamingBuffer::new();
        buf.close();

        let chunks = buf.take_chunks();
        assert_eq!(chunks.len(), 0); // No data written, no chunks
    }

    #[test]
    fn test_streaming_buffer_headers() {
        let mut buf = StreamingBuffer::new();
        buf.set_status(404);
        buf.add_header("Content-Type", "text/html");
        buf.add_header("Cache-Control", "no-cache");

        assert_eq!(buf.status_code(), 404);
        assert_eq!(buf.headers().len(), 2);
        assert!(!buf.header_sent());

        buf.mark_header_sent();
        assert!(buf.header_sent());
    }

    #[test]
    fn test_streaming_buffer_exact_chunk() {
        let mut buf = StreamingBuffer::new();
        buf.write(&vec![0u8; MAX_CHUNK_SIZE]);
        // Exactly one chunk should be ready
        assert!(buf.has_chunks());

        buf.close();
        let chunks = buf.take_chunks();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].len(), MAX_CHUNK_SIZE);
    }
}
