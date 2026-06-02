use std::{mem, pin::Pin, time::Duration};

use async_stream::try_stream;
use reqwest::{
    Response, StatusCode,
    header::{CONTENT_TYPE, HeaderValue},
};
use tokio::io::AsyncBufReadExt;
use tokio_stream::{Stream, StreamExt};
use tokio_util::io::StreamReader;

#[derive(Debug, thiserror::Error)]
pub enum EventSourceError {
    #[error("Unexpected response status: {0}")]
    BadStatus(StatusCode),
    #[error("Missing or invalid Content-Type header: {0:?}")]
    BadContentType(Option<HeaderValue>),
    #[error("Error while reading event stream: {0}")]
    EventError(#[from] EventError),
}

#[derive(Debug, thiserror::Error)]
pub enum EventError {
    #[error("I/O error while reading event stream: {0}")]
    IoError(#[from] std::io::Error),
}

/// Represents a stream of Server-Sent Events.
pub type ServerSentEvents = Pin<Box<dyn Stream<Item = Result<Event, EventSourceError>> + Send>>;

pub static MIME_EVENT_STREAM: &[u8] = b"text/event-stream";

/// Returns `true` if the given [`HeaderValue`] is an event stream MIME type.
fn is_event_stream(value: &HeaderValue) -> bool {
    value
        .as_bytes()
        .split(|&b| b == b';')
        .next()
        .unwrap_or(b"")
        .trim_ascii()
        .eq_ignore_ascii_case(MIME_EVENT_STREAM)
}

/// Internal buffer used to accumulate lines of an SSE (Server-Sent Events) stream.
///
/// A single [`EventBuffer`] can be used to process the whole stream. [`set_event_type`] and [`push_data`]
/// methods update the state. [`produce_event`] produces a proper [`Event`] and prepares the internal
/// state to process further data.
#[derive(Default)]
struct EventBuffer {
    event_type: String,
    data: String,
    last_event_id: Option<String>,
    retry: Option<Duration>,
}

impl EventBuffer {
    /// Produces a [`Event`], if current state allows it.
    ///
    /// Resets the internal state to process further data.
    fn produce_event(&mut self) -> Option<Event> {
        if self.data.is_empty() {
            // Per spec, if the data buffer is empty the event type buffer must also be cleared.
            self.event_type.clear();
            return None;
        }

        let event_type = mem::take(&mut self.event_type);
        let event_type = if event_type.is_empty() {
            "message".to_string()
        } else {
            event_type
        };
        let data = mem::take(&mut self.data);

        Some(Event {
            event_type,
            data,
            last_event_id: self.last_event_id.clone(),
            retry: self.retry,
        })
    }

    /// Set the [`Event`]'s type. Overrides previous value.
    fn set_event_type(&mut self, event_type: &str) {
        self.event_type.clear();
        self.event_type.push_str(event_type);
    }

    /// Extends internal data with given data.
    fn push_data(&mut self, data: &str) {
        if !self.data.is_empty() {
            self.data.push('\n');
        }
        self.data.push_str(data);
    }

    fn set_id(&mut self, id: &str) {
        self.last_event_id = Some(id.to_string());
    }

    fn set_retry(&mut self, retry: Duration) {
        self.retry = Some(retry);
    }
}

/// Parse line to split field name and value, applying proper trimming.
fn parse_line(line: &str) -> (&str, &str) {
    let (field, value) = line.split_once(':').unwrap_or((line, ""));
    let value = value.strip_prefix(' ').unwrap_or(value);
    (field, value)
}

/// Server-Sent Event representation.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Event {
    /// A string identifying the type of event described.
    pub event_type: String,
    /// The data field for the message.
    pub data: String,
    /// Last event ID value.
    pub last_event_id: Option<String>,
    /// Reconnection time.
    pub retry: Option<Duration>,
}

/// A trait for consuming a [`Response`] as a [`Stream`] of Server-Sent [`Event`]s (SSE).
pub trait EventSource {
    /// Converts the [`Response`] into a stream of Server-Sent Events.
    /// Returns it as a fallible [`Stream`] of [`Event`]s.
    ///
    /// # Errors
    ///
    /// Returns an [`EventSourceError`] if:
    /// - The response status is not `200 OK`
    /// - The `Content-Type` header is missing or not `text/event-stream`
    ///
    /// The stream yields an [`EventError`] when an error occurs on event reading.
    fn events(self) -> impl Future<Output = Result<ServerSentEvents, EventSourceError>> + Send;
}

impl EventSource for Response {
    async fn events(self) -> Result<ServerSentEvents, EventSourceError> {
        let status = self.status();
        if status != StatusCode::OK {
            return Err(EventSourceError::BadStatus(status));
        }
        match self.headers().get(CONTENT_TYPE) {
            Some(content_type) => {
                if !is_event_stream(content_type) {
                    return Err(EventSourceError::BadContentType(Some(
                        content_type.to_owned(),
                    )));
                }
            }
            None => return Err(EventSourceError::BadContentType(None)),
        }

        let mut stream = StreamReader::new(
            self.bytes_stream()
                .map(|result| result.map_err(std::io::Error::other)),
        );

        let mut line_buffer = String::new();
        let mut event_buffer = EventBuffer::default();

        let stream: ServerSentEvents = Box::pin(try_stream! {
            loop {
                line_buffer.clear();
                let count = stream.read_line(&mut line_buffer).await.map_err(EventError::IoError)?;
                if count == 0 {
                    break;
                }

                // Strip the line terminator (\r\n, \n, or \r) according to the SSE spec.
                let line = line_buffer
                    .strip_suffix("\r\n")
                    .or_else(|| line_buffer.strip_suffix('\n'))
                    .unwrap_or(&line_buffer);

                // dispatch
                if line.is_empty() {
                    if let Some(event) = event_buffer.produce_event() {
                        yield event;
                    }
                    continue;
                }

                let (field, value) = parse_line(line);

                match field {
                    "event" => {
                        event_buffer.set_event_type(value);
                    }
                    "data" => {
                        event_buffer.push_data(value);
                    }
                    "id" => {
                        event_buffer.set_id(value);
                    }
                    "retry" => {
                        if let Ok(millis) = value.parse() {
                            event_buffer.set_retry(Duration::from_millis(millis));
                        }
                    }
                    _ => {}
                }
            }
        });

        Ok(stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_line_properly() {
        let (field, value) = parse_line("event: message");
        assert_eq!(field, "event");
        assert_eq!(value, "message");

        let (field, value) = parse_line("non-standard field");
        assert_eq!(field, "non-standard field");
        assert_eq!(value, "");

        let (field, value) = parse_line("data:data with : inside");
        assert_eq!(field, "data");
        assert_eq!(value, "data with : inside");
    }

    #[test]
    fn is_event_stream_accept_valid_values() {
        assert!(is_event_stream(&HeaderValue::from_static(
            "text/event-stream"
        )));
        assert!(is_event_stream(&HeaderValue::from_static(
            "text/event-stream; charset=utf-8"
        )));
        assert!(is_event_stream(&HeaderValue::from_static(
            "   TEXT/event-stream    ; charset=utf-8"
        )));
    }

    #[test]
    fn is_event_stream_reject_invalid_values() {
        assert!(!is_event_stream(&HeaderValue::from_static("plain/text")));
        assert!(!is_event_stream(&HeaderValue::from_static(
            "text/event-but-not-realy"
        )));
    }
}
