// AUv3 ObjC code generation templates
// This file is included by build.rs via include!()

/// Generate the AUv3 wrapper ObjC implementation with plugin-specific class names.
///
/// The AUv2 wrapper has its own template and is generated in `auv2_c.rs`.
fn generate_au_wrapper_source(plugin_name: &str) -> String {
    let pascal_name = to_pascal_case(plugin_name);
    let wrapper_class = format!("Beamer{}AuWrapper", pascal_name);

    include_str!("auv3_wrapper.m")
        .replace("{{PLUGIN_NAME}}", plugin_name)
        .replace("{{WRAPPER_CLASS}}", &wrapper_class)
}

/// Generate the AU extension ObjC implementation with plugin-specific class names.
///
/// When `has_gui` is true, uses the AUViewController-based template so that
/// Logic Pro (and other hosts that check NSExtensionPrincipalClass) shows the
/// custom WebView GUI. Non-GUI plugins use the plain NSObject template.
fn generate_au_extension_source(plugin_name: &str, has_gui: bool) -> String {
    let pascal_name = to_pascal_case(plugin_name);
    let wrapper_class = format!("Beamer{}AuWrapper", pascal_name);
    let extension_class = format!("Beamer{}AuExtension", pascal_name);
    let factory_func = format!("Beamer{}AuExtensionFactory", pascal_name);

    let template = if has_gui {
        include_str!("auv3_extension_gui.m")
    } else {
        include_str!("auv3_extension.m")
    };

    template
        .replace("{{PLUGIN_NAME}}", plugin_name)
        .replace("{{WRAPPER_CLASS}}", &wrapper_class)
        .replace("{{EXTENSION_CLASS}}", &extension_class)
        .replace("{{FACTORY_FUNC}}", &factory_func)
}
