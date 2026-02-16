// AUv2 C code generation template
// This file is included by auv2.rs via include!()

fn generate_auv2_wrapper_source(plugin_name: &str, component_type: &str) -> String {
    let pascal_name = to_pascal_case(plugin_name);
    let factory_name = format!("Beamer{}Factory", pascal_name);
    let cocoa_view_factory_class = format!("Beamer{}CocoaViewFactory", pascal_name);
    let cocoa_editor_view_class = format!("Beamer{}EditorView", pascal_name);

    // Only expose MusicDeviceMIDIEvent for types that accept MIDI
    let midi_event_case = match component_type {
        "aumu" | "aumf" | "aumi" => {
            "        case kMusicDeviceMIDIEventSelect:\n            return (AudioComponentMethod)BeamerAuv2MIDIEvent;"
        }
        _ => "        // MIDI not exposed for this component type",
    };

    include_str!("auv2_wrapper.c")
        .replace("{{PLUGIN_NAME}}", plugin_name)
        .replace("{{FACTORY_NAME}}", &factory_name)
        .replace("{{COCOA_VIEW_FACTORY_CLASS}}", &cocoa_view_factory_class)
        .replace("{{COCOA_EDITOR_VIEW_CLASS}}", &cocoa_editor_view_class)
        .replace("{{MIDI_EVENT_CASE}}", midi_event_case)
}
