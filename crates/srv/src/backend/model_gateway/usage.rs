// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a response used, read from the bytes as they pass: the usage block of a Messages response, or of
//! the `message_start` and `message_delta` events of a stream. Nothing else in the body is kept.

use crate::agents::types::TokenCounts;
use serde_json::Value;

/// A body larger than this is not buffered to read its usage (a stream is read line by line instead).
const MAX_JSON_BODY: usize = 8 * 1024 * 1024;
/// A single stream line longer than this is dropped rather than held.
const MAX_LINE: usize = 1024 * 1024;

pub struct UsageTap {
    stream: bool,
    pending: Vec<u8>,
    body: Vec<u8>,
    counts: TokenCounts,
    model: Option<String>,
}

impl UsageTap {
    /// `stream`: the response is `text/event-stream`.
    pub fn new(stream: bool) -> Self {
        Self { stream, pending: Vec::new(), body: Vec::new(), counts: TokenCounts::default(), model: None }
    }

    pub fn push(&mut self, chunk: &[u8]) {
        if !self.stream {
            if self.body.len() + chunk.len() <= MAX_JSON_BODY {
                self.body.extend_from_slice(chunk);
            }
            return;
        }
        self.pending.extend_from_slice(chunk);
        while let Some(end) = self.pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=end).collect();
            self.line(&line);
        }
        if self.pending.len() > MAX_LINE {
            self.pending.clear();
        }
    }

    fn line(&mut self, line: &[u8]) {
        let Ok(text) = std::str::from_utf8(line) else { return };
        let Some(data) = text.trim().strip_prefix("data:") else { return };
        if let Ok(event) = serde_json::from_str::<Value>(data.trim()) {
            self.event(&event);
        }
    }

    fn event(&mut self, event: &Value) {
        match event.get("type").and_then(Value::as_str) {
            Some("message_start") => {
                let message = event.get("message").unwrap_or(&Value::Null);
                self.model = message.get("model").and_then(Value::as_str).map(str::to_string);
                self.usage(message.get("usage"));
            }
            Some("message_delta") => self.usage(event.get("usage")),
            _ => {}
        }
    }

    /// Later counts replace earlier ones: a stream reports running totals, and a field it leaves out keeps its value.
    fn usage(&mut self, usage: Option<&Value>) {
        let Some(usage) = usage else { return };
        let read = |key: &str| usage.get(key).and_then(Value::as_u64);
        if let Some(n) = read("input_tokens") {
            self.counts.input = n;
        }
        if let Some(n) = read("output_tokens") {
            self.counts.output = n;
        }
        if let Some(n) = read("cache_creation_input_tokens") {
            self.counts.cache_creation = n;
        }
        if let Some(n) = read("cache_read_input_tokens") {
            self.counts.cache_read = n;
        }
    }

    /// The counts, and the model the upstream says answered.
    pub fn finish(mut self) -> (TokenCounts, Option<String>) {
        if !self.stream {
            if let Ok(body) = serde_json::from_slice::<Value>(&self.body) {
                self.model = body.get("model").and_then(Value::as_str).map(str::to_string);
                self.usage(body.get("usage"));
            }
        }
        (self.counts, self.model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STREAM: &str = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"model\":\"deepseek/deepseek-v4.1-flash-1017\",\"usage\":{\"input_tokens\":120,\"cache_read_input_tokens\":40,\"output_tokens\":1}}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"hi\"}}\n\n",
        "event: message_delta\n",
        "data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":57}}\n\n",
    );

    #[test]
    fn a_stream_is_read_whatever_the_chunking() {
        for size in [1, 7, 64, STREAM.len()] {
            let mut tap = UsageTap::new(true);
            for chunk in STREAM.as_bytes().chunks(size) {
                tap.push(chunk);
            }
            let (counts, model) = tap.finish();
            assert_eq!((counts.input, counts.output, counts.cache_read), (120, 57, 40), "chunks of {size}");
            assert_eq!(model.as_deref(), Some("deepseek/deepseek-v4.1-flash-1017"));
        }
    }

    #[test]
    fn a_plain_response_is_read_at_the_end() {
        let mut tap = UsageTap::new(false);
        tap.push(br#"{"model":"m","usage":{"input_tokens":9,"output_tokens":3}"#);
        tap.push(b"}");
        let (counts, model) = tap.finish();
        assert_eq!((counts.input, counts.output, model.as_deref()), (9, 3, Some("m")));
    }

    #[test]
    fn text_that_is_not_an_event_is_ignored() {
        let mut tap = UsageTap::new(true);
        tap.push(b": keep-alive\n\ndata: not json\n\ndata: [DONE]\n");
        assert_eq!(tap.finish().0, TokenCounts::default());
    }
}
