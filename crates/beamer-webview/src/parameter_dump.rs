//! The parameters as a freshly loaded page gets them.

use beamer_core::{ParameterId, ParameterStore};

/// Build a JSON array of parameter info for WebView init dumps.
///
/// Returns a JSON string like `[{"id":0,"stringId":"gain",...}, ...]`.
/// The init script of the editor sync hands it to the JavaScript runtime
/// of a page that loaded.
pub fn params_to_init_json(store: &dyn ParameterStore) -> String {
    let entries: Vec<ParamInitEntry> = (0..store.count())
        .filter_map(|i| {
            let info = store.info(i)?;
            let normalized = store.get_normalized(info.id);
            Some(ParamInitEntry {
                id: info.id,
                string_id: &info.string_id,
                name: info.name,
                min: store.normalized_to_plain(info.id, 0.0),
                max: store.normalized_to_plain(info.id, 1.0),
                default_value: info.default_normalized,
                value: normalized,
                plain_value: store.normalized_to_plain(info.id, normalized),
                display_text: store.normalized_to_string(info.id, normalized),
                format: store.formatter_kind(info.id),
                units: info.units,
                steps: info.step_count,
                value_texts: value_texts(store, info.id, info.step_count),
            })
        })
        .collect();
    serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_string())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ParamInitEntry<'a> {
    id: u32,
    string_id: &'a str,
    name: &'static str,
    min: f64,
    max: f64,
    default_value: f64,
    value: f64,
    plain_value: f64,
    display_text: String,
    format: &'static str,
    units: &'static str,
    steps: i32,
    value_texts: Vec<String>,
}

/// The most values a parameter can have for the init dump to list their
/// texts: enough for a MIDI note or controller number.
const MAX_VALUE_TEXTS: i32 = 128;

/// The display text of each value of a discrete parameter, from the lowest
/// up, so that a web UI can label every choice of an enum. Empty for a
/// continuous parameter and for one with more than [`MAX_VALUE_TEXTS`]
/// values.
fn value_texts(store: &dyn ParameterStore, id: ParameterId, steps: i32) -> Vec<String> {
    if steps <= 0 || steps >= MAX_VALUE_TEXTS {
        return Vec::new();
    }
    (0..=steps)
        .map(|step| store.normalized_to_string(id, f64::from(step) / f64::from(steps)))
        .collect()
}
