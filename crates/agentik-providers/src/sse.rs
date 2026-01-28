//! Server-Sent Events (SSE) parser with line buffering.
//!
//! SSE lines can span multiple TCP packets, so we need to buffer bytes
//! until we have complete lines before parsing.

use std::fmt;

/// A parsed SSE event.
#[derive(Debug, Clone, PartialEq)]
pub struct SseEvent {
    /// The event type (from "event:" line)
    pub event: Option<String>,
    /// The event data (from "data:" lines)
    pub data: String,
    /// The event ID (from "id:" line)
    pub id: Option<String>,
    /// Retry value (from "retry:" line)
    pub retry: Option<u64>,
}

impl SseEvent {
    /// Check if this is a [DONE] marker.
    pub fn is_done(&self) -> bool {
        self.data == "[DONE]"
    }
}

/// SSE parser that handles line buffering across TCP packets.
#[derive(Default)]
pub struct SseParser {
    /// Buffer for incomplete lines
    buffer: String,
    /// Current event being built
    current_event: Option<String>,
    current_data: Vec<String>,
    current_id: Option<String>,
    current_retry: Option<u64>,
}

impl SseParser {
    /// Create a new SSE parser.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed bytes into the parser and return any complete events.
    ///
    /// Call this for each chunk of bytes received from the stream.
    /// Returns a vector of complete SSE events.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
        self.buffer.push_str(&String::from_utf8_lossy(bytes));
        self.parse_buffer()
    }

    /// Feed a string into the parser.
    pub fn feed_str(&mut self, text: &str) -> Vec<SseEvent> {
        self.buffer.push_str(text);
        self.parse_buffer()
    }

    /// Parse the buffer for complete events.
    fn parse_buffer(&mut self) -> Vec<SseEvent> {
        let mut events = Vec::new();

        // Process complete lines from the buffer
        loop {
            // Find the next line ending
            let newline_pos = match self.buffer.find('\n') {
                Some(pos) => pos,
                None => break, // No complete line yet
            };

            // Extract the line (without the newline)
            let line = self.buffer[..newline_pos].to_string();
            // Remove the line from the buffer (including newline)
            self.buffer = self.buffer[newline_pos + 1..].to_string();

            // Strip carriage return if present (for \r\n line endings)
            let line = line.trim_end_matches('\r');

            // Empty line signals end of event
            if line.is_empty() {
                if let Some(event) = self.finalize_event() {
                    events.push(event);
                }
                continue;
            }

            // Parse the field
            if let Some((field, value)) = Self::parse_field(line) {
                match field {
                    "event" => self.current_event = Some(value.to_string()),
                    "data" => self.current_data.push(value.to_string()),
                    "id" => self.current_id = Some(value.to_string()),
                    "retry" => {
                        if let Ok(ms) = value.parse() {
                            self.current_retry = Some(ms);
                        }
                    }
                    _ => {} // Ignore unknown fields
                }
            }
            // Lines starting with : are comments, ignore them
        }

        events
    }

    /// Parse a single SSE field line.
    fn parse_field(line: &str) -> Option<(&str, &str)> {
        // Lines starting with : are comments
        if line.starts_with(':') {
            return None;
        }

        // Find the colon separator
        if let Some(colon_pos) = line.find(':') {
            let field = &line[..colon_pos];
            let mut value = &line[colon_pos + 1..];

            // Remove leading space from value if present
            if value.starts_with(' ') {
                value = &value[1..];
            }

            Some((field, value))
        } else {
            // Field with no value
            Some((line, ""))
        }
    }

    /// Finalize the current event and reset state.
    fn finalize_event(&mut self) -> Option<SseEvent> {
        if self.current_data.is_empty() {
            // No data, reset and return None
            self.current_event = None;
            self.current_id = None;
            self.current_retry = None;
            return None;
        }

        let event = SseEvent {
            event: self.current_event.take(),
            data: self.current_data.join("\n"),
            id: self.current_id.take(),
            retry: self.current_retry.take(),
        };

        self.current_data.clear();
        Some(event)
    }

    /// Check if there's any buffered data.
    pub fn has_buffered_data(&self) -> bool {
        !self.buffer.is_empty() || !self.current_data.is_empty()
    }

    /// Clear the parser state.
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.current_event = None;
        self.current_data.clear();
        self.current_id = None;
        self.current_retry = None;
    }
}

impl fmt::Debug for SseParser {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SseParser")
            .field("buffer_len", &self.buffer.len())
            .field("current_data_lines", &self.current_data.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_event() {
        let mut parser = SseParser::new();
        let events = parser.feed(b"data: hello world\n\n");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "hello world");
        assert_eq!(events[0].event, None);
    }

    #[test]
    fn test_event_with_type() {
        let mut parser = SseParser::new();
        let events = parser.feed(b"event: message\ndata: hello\n\n");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, Some("message".to_string()));
        assert_eq!(events[0].data, "hello");
    }

    #[test]
    fn test_multiline_data() {
        let mut parser = SseParser::new();
        let events = parser.feed(b"data: line one\ndata: line two\n\n");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "line one\nline two");
    }

    #[test]
    fn test_split_across_chunks() {
        let mut parser = SseParser::new();

        // First chunk - incomplete
        let events = parser.feed(b"data: hel");
        assert!(events.is_empty());

        // Second chunk - completes the line
        let events = parser.feed(b"lo world\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "hello world");
    }

    #[test]
    fn test_multiple_events() {
        let mut parser = SseParser::new();
        let events = parser.feed(b"data: first\n\ndata: second\n\n");

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].data, "first");
        assert_eq!(events[1].data, "second");
    }

    #[test]
    fn test_done_marker() {
        let mut parser = SseParser::new();
        let events = parser.feed(b"data: [DONE]\n\n");

        assert_eq!(events.len(), 1);
        assert!(events[0].is_done());
    }

    #[test]
    fn test_comment_ignored() {
        let mut parser = SseParser::new();
        let events = parser.feed(b": this is a comment\ndata: hello\n\n");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "hello");
    }

    #[test]
    fn test_json_data() {
        let mut parser = SseParser::new();
        let events = parser.feed(b"data: {\"type\": \"message\", \"text\": \"hello\"}\n\n");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, r#"{"type": "message", "text": "hello"}"#);
    }

    #[test]
    fn test_crlf_line_endings() {
        let mut parser = SseParser::new();
        let events = parser.feed(b"data: hello\r\n\r\n");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "hello");
    }

    #[test]
    fn test_empty_event_ignored() {
        let mut parser = SseParser::new();
        // Two empty lines should not produce an event
        let events = parser.feed(b"\n\n");
        assert!(events.is_empty());
    }

    #[test]
    fn test_partial_json_reassembly() {
        let mut parser = SseParser::new();

        // Simulate a JSON object split across chunks
        let events = parser.feed(b"data: {\"type\":");
        assert!(events.is_empty());

        let events = parser.feed(b" \"delta\", ");
        assert!(events.is_empty());

        let events = parser.feed(b"\"text\": \"hello\"}\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, r#"{"type": "delta", "text": "hello"}"#);
    }

    #[test]
    fn test_id_and_retry() {
        let mut parser = SseParser::new();
        let events = parser.feed(b"id: 123\nretry: 5000\ndata: hello\n\n");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, Some("123".to_string()));
        assert_eq!(events[0].retry, Some(5000));
        assert_eq!(events[0].data, "hello");
    }
}
