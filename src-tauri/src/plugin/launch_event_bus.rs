use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fmt;
use tauri::{AppHandle, Emitter, Runtime};

pub const LAUNCH_EVENT_NAME: &str = "launch-event";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchEvent {
    pub event: String,
    #[serde(default)]
    pub pid: Option<u32>,
    #[serde(default)]
    pub module: Option<String>,
    #[serde(default)]
    pub stage: Option<String>,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(flatten)]
    pub details: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchEventParseError {
    InvalidJson(String),
    MissingEvent,
}

impl fmt::Display for LaunchEventParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson(error) => write!(formatter, "invalid launch event JSON: {error}"),
            Self::MissingEvent => write!(formatter, "launch event is missing event"),
        }
    }
}

impl std::error::Error for LaunchEventParseError {}

pub fn parse_json_line(line: &str) -> Result<Option<LaunchEvent>, LaunchEventParseError> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(None);
    }
    let raw = serde_json::from_str::<Value>(line)
        .map_err(|error| LaunchEventParseError::InvalidJson(error.to_string()))?;
    if raw.get("event").and_then(Value::as_str).is_none() {
        return Err(LaunchEventParseError::MissingEvent);
    }
    let event = serde_json::from_value::<LaunchEvent>(raw)
        .map_err(|error| LaunchEventParseError::InvalidJson(error.to_string()))?;
    if event.event.trim().is_empty() {
        return Err(LaunchEventParseError::MissingEvent);
    }
    Ok(Some(event))
}

pub struct LaunchEventBus;

impl LaunchEventBus {
    pub fn publish<R: Runtime>(
        app: &AppHandle<R>,
        event: &LaunchEvent,
    ) -> Result<(), tauri::Error> {
        app.emit(LAUNCH_EVENT_NAME, event)
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_json_line, LaunchEventParseError};

    #[test]
    fn parses_known_and_future_fields_without_losing_data() {
        let event = parse_json_line(r#"{"event":"target_created","pid":42,"future":"kept"}"#)
            .unwrap()
            .unwrap();
        assert_eq!(event.event, "target_created");
        assert_eq!(event.pid, Some(42));
        assert_eq!(event.details["future"], "kept");
    }

    #[test]
    fn ignores_blank_lines_and_rejects_missing_event() {
        assert_eq!(parse_json_line("  \n").unwrap(), None);
        assert!(matches!(
            parse_json_line(r#"{"pid":42}"#),
            Err(LaunchEventParseError::MissingEvent)
        ));
    }
}
