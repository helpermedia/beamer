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
    /// The page set a parameter to a normalized value. `seq` numbers the
    /// sets of the page; the answer carries it back, so that the page can
    /// tell the answer to an older set from the answer to its newest.
    #[serde(rename = "parameter:set")]
    SetParameter {
        id: ParameterId,
        value: f64,
        #[serde(default)]
        seq: Option<u64>,
    },

    /// The page began a gesture on a parameter: a drag or a scroll.
    #[serde(rename = "parameter:begin")]
    BeginGesture { id: ParameterId },

    /// The page ended a gesture on a parameter.
    #[serde(rename = "parameter:end")]
    EndGesture { id: ParameterId },

    /// The page set a `State` field, known by its path. A value the page
    /// left out is `null`. `seq` as for a parameter.
    #[serde(rename = "state:set")]
    SetState {
        #[serde(rename = "id")]
        path: String,
        #[serde(default)]
        value: Value,
        #[serde(default)]
        seq: Option<u64>,
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

    /// The page started a note: a note number, a velocity from 0 to 1 and
    /// a channel from 0 to 15. The velocity is `None` when the page left it
    /// out, the channel 0.
    #[serde(rename = "note:on")]
    NoteOn {
        note: u8,
        #[serde(default)]
        velocity: Option<f32>,
        #[serde(default)]
        channel: u8,
    },

    /// The page ended a note, with a release velocity or `None`.
    #[serde(rename = "note:off")]
    NoteOff {
        note: u8,
        #[serde(default)]
        velocity: Option<f32>,
        #[serde(default)]
        channel: u8,
    },

    /// The page ended every note it started.
    #[serde(rename = "note:all-off")]
    AllNotesOff,
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
            parse(r#"{"type":"parameter:set","id":7,"value":0.25,"seq":4}"#),
            Some(PageMessage::SetParameter { id: 7, value: 0.25, seq: Some(4) })
        );
        assert_eq!(
            parse(r#"{"type":"parameter:begin","id":7}"#),
            Some(PageMessage::BeginGesture { id: 7 })
        );
        assert_eq!(
            parse(r#"{"type":"parameter:end","id":7}"#),
            Some(PageMessage::EndGesture { id: 7 })
        );
        assert_eq!(
            parse(r#"{"type":"state:set","id":"layer/zoom","value":[1,2],"seq":5}"#),
            Some(PageMessage::SetState { path: String::from("layer/zoom"), value: json!([1, 2]), seq: Some(5) })
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
        assert_eq!(
            parse(r#"{"type":"note:on","note":36,"velocity":0.5,"channel":9}"#),
            Some(PageMessage::NoteOn { note: 36, velocity: Some(0.5), channel: 9 })
        );
        assert_eq!(
            parse(r#"{"type":"note:off","note":36,"velocity":0.25,"channel":9}"#),
            Some(PageMessage::NoteOff { note: 36, velocity: Some(0.25), channel: 9 })
        );
        assert_eq!(parse(r#"{"type":"note:all-off"}"#), Some(PageMessage::AllNotesOff));
    }

    #[test]
    fn test_parts_the_page_left_out_are_null_or_empty() {
        assert_eq!(
            parse(r#"{"type":"state:set","id":"zoom"}"#),
            Some(PageMessage::SetState { path: String::from("zoom"), value: Value::Null, seq: None })
        );
        assert_eq!(
            parse(r#"{"type":"parameter:set","id":7,"value":0.25}"#),
            Some(PageMessage::SetParameter { id: 7, value: 0.25, seq: None })
        );
        assert_eq!(
            parse(r#"{"type":"invoke","method":"getInfo","callId":1}"#),
            Some(PageMessage::Invoke { method: String::from("getInfo"), args: Vec::new(), call_id: 1 })
        );
        assert_eq!(
            parse(r#"{"type":"event","name":"ping"}"#),
            Some(PageMessage::Event { name: String::from("ping"), data: Value::Null })
        );
        assert_eq!(
            parse(r#"{"type":"note:on","note":60}"#),
            Some(PageMessage::NoteOn { note: 60, velocity: None, channel: 0 })
        );
        assert_eq!(
            parse(r#"{"type":"note:off","note":60}"#),
            Some(PageMessage::NoteOff { note: 60, velocity: None, channel: 0 })
        );
    }

    #[test]
    fn test_other_messages_are_refused() {
        // Not JSON, no type, a type the runtime does not send, a missing
        // field and a parameter id out of range.
        assert_eq!(parse("parameter:set"), None);
        assert_eq!(parse(r#"{"id":7,"value":0.5}"#), None);
        assert_eq!(parse(r#"{"type":"parameter:reset","id":7}"#), None);
        assert_eq!(parse(r#"{"type":"parameter:set","id":7}"#), None);
        assert_eq!(parse(r#"{"type":"invoke","method":"getInfo"}"#), None);
        assert_eq!(parse(r#"{"type":"parameter:begin","id":-1}"#), None);
        // A note number that is not a byte, or none.
        assert_eq!(parse(r#"{"type":"note:on","note":60.5}"#), None);
        assert_eq!(parse(r#"{"type":"note:on","note":256}"#), None);
        assert_eq!(parse(r#"{"type":"note:off"}"#), None);
    }
}
