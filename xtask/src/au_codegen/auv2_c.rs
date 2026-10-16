// AUv2 C code generation template
// This file is included by auv2.rs via include!()

fn generate_auv2_wrapper_source(plugin_name: &str, codes: &AuCodes, names: &AuNames) -> String {
    // Only expose MusicDeviceMIDIEvent for types that accept MIDI
    let midi_event_case = match codes.component_type.as_str() {
        "aumu" | "aumi" => {
            "        case kMusicDeviceMIDIEventSelect:\n            return (AudioComponentMethod)BeamerAuv2MIDIEvent;"
        }
        _ => "        // MIDI not exposed for this component type",
    };

    include_str!("auv2_wrapper.c")
        .replace("{{PLUGIN_NAME}}", plugin_name)
        .replace("{{FACTORY_NAME}}", &names.auv2_factory)
        .replace("{{COCOA_VIEW_FACTORY_CLASS}}", &names.cocoa_view_factory_class)
        .replace("{{COCOA_GUI_VIEW_CLASS}}", &names.cocoa_gui_view_class)
        .replace("{{MIDI_EVENT_CASE}}", midi_event_case)
}
