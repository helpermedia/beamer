//! The messages of a page to its plugin.
//!
//! The JavaScript runtime (`beamer_runtime.js`) posts each message as a
//! JSON object whose `type` names it. The editor sync reads it here once
//! for every format wrapper.

use beamer_core::ParameterId;
use serde::Deserialize;
use serde_json::Value;

/// A message of the page, as the runtime posts it.
#[derive(Debug, PartialEq, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum PageMessage {
    /// The page set a parameter to a normalized value.
    #[serde(rename = "param:set")]
    SetParameter { id: ParameterId, value: f64 },

    /// The page began a gesture on a parameter: a drag or a scroll.
    #[serde(rename = "param:begin")]
    BeginGesture { id: ParameterId },

    /// The page ended a gesture on a parameter.
    #[serde(rename = "param:end")]
    EndGesture { id: ParameterId },

    /// The page set a `State` field, known by its path. A value the page
    /// left out is `null`.
    #[serde(rename = "state:set")]
    SetState {
        #[serde(rename = "id")]
        path: String,
        #[serde(default)]
        value: Value,
    },

    /// A call of `__BEAMER__.invoke()`. The answer settles the promise
    /// with the same call id.
    #[serde(rename = "invoke")]
    Invoke {
        method: String,
        #[serde(default)]
        args: Vec<Value>,
        #[serde(rename = "callId")]
        call_id: u64,
    },

    /// A custom event of `__BEAMER__.emit()`. Data the page left out is
    /// `null`.
    #[serde(rename = "event")]
    Event {
        name: String,
        #[serde(default)]
        data: Value,
    },
}

impl PageMessage {
    /// Read a message of the page. `None`, logged, for one that is not a
    /// message of the runtime.
    pub(crate) fn parse(json: &[u8]) -> Option<Self> {
        match serde_json::from_slice(json) {
            Ok(message) => Some(message),
            Err(error) => {
                log::warn!(
                    "The web UI sent a message the plugin does not understand ({error}): {}",
                    String::from_utf8_lossy(json)
                );
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn parse(json: &str) -> Option<PageMessage> {
        PageMessage::parse(json.as_bytes())
    }

    #[test]
    fn test_messages_of_the_runtime_are_read() {
        assert_eq!(
            parse(r#"{"type":"param:set","id":7,"value":0.25}"#),
            Some(PageMessage::SetParameter { id: 7, value: 0.25 })
        );
        assert_eq!(
            parse(r#"{"type":"param:begin","id":7}"#),
            Some(PageMessage::BeginGesture { id: 7 })
        );
        assert_eq!(
            parse(r#"{"type":"param:end","id":7}"#),
            Some(PageMessage::EndGesture { id: 7 })
        );
        assert_eq!(
            parse(r#"{"type":"state:set","id":"layer/zoom","value":[1,2]}"#),
            Some(PageMessage::SetState { path: String::from("layer/zoom"), value: json!([1, 2]) })
        );
        assert_eq!(
            parse(r#"{"type":"invoke","method":"getInfo","args":[1,"a"],"callId":3}"#),
            Some(PageMessage::Invoke {
                method: String::from("getInfo"),
                args: vec![json!(1), json!("a")],
                call_id: 3,
            })
        );
        assert_eq!(
            parse(r#"{"type":"event","name":"zoom","data":{"level":2}}"#),
            Some(PageMessage::Event { name: String::from("zoom"), data: json!({"level": 2}) })
        );
    }

    #[test]
    fn test_parts_the_page_left_out_are_null_or_empty() {
        assert_eq!(
            parse(r#"{"type":"state:set","id":"zoom"}"#),
            Some(PageMessage::SetState { path: String::from("zoom"), value: Value::Null })
        );
        assert_eq!(
            parse(r#"{"type":"invoke","method":"getInfo","callId":1}"#),
            Some(PageMessage::Invoke { method: String::from("getInfo"), args: Vec::new(), call_id: 1 })
        );
        assert_eq!(
            parse(r#"{"type":"event","name":"ping"}"#),
            Some(PageMessage::Event { name: String::from("ping"), data: Value::Null })
        );
    }

    #[test]
    fn test_other_messages_are_refused() {
        // Not JSON, no type, a type the runtime does not send, a missing
        // field and a parameter id out of range.
        assert_eq!(parse("param:set"), None);
        assert_eq!(parse(r#"{"id":7,"value":0.5}"#), None);
        assert_eq!(parse(r#"{"type":"param:reset","id":7}"#), None);
        assert_eq!(parse(r#"{"type":"param:set","id":7}"#), None);
        assert_eq!(parse(r#"{"type":"invoke","method":"getInfo"}"#), None);
        assert_eq!(parse(r#"{"type":"param:begin","id":-1}"#), None);
    }
}
