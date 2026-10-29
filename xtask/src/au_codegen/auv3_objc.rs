// AUv3 ObjC code generation templates
// This file is included by build.rs via include!()

/// Generate the AUv3 wrapper ObjC implementation with plugin-specific class names.
///
/// The AUv2 wrapper has its own template and is generated in `auv2_c.rs`.
fn generate_au_wrapper_source(plugin_name: &str, names: &AuNames) -> String {
    include_str!("auv3_wrapper.m")
        .replace("{{PLUGIN_NAME}}", plugin_name)
        .replace("{{WRAPPER_CLASS}}", &names.wrapper_class)
}

/// Generate the AU extension ObjC implementation with plugin-specific class names.
///
/// When `has_editor` is true, uses the AUViewController-based template so
/// that Logic Pro (and other hosts that check NSExtensionPrincipalClass)
/// shows the editor. Plugins without an editor use the plain NSObject
/// template.
fn generate_au_extension_source(plugin_name: &str, names: &AuNames, has_editor: bool) -> String {
    let template = if has_editor {
        include_str!("auv3_extension_editor.m")
    } else {
        include_str!("auv3_extension.m")
    };

    template
        .replace("{{PLUGIN_NAME}}", plugin_name)
        .replace("{{WRAPPER_CLASS}}", &names.wrapper_class)
        .replace("{{EXTENSION_CLASS}}", &names.extension_class)
}
