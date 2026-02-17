// AUv3 ObjC code generation templates
// This file is included by build.rs via include!()

fn generate_au_wrapper_source(plugin_name: &str) -> String {
    let pascal_name = to_pascal_case(plugin_name);
    let wrapper_class = format!("Beamer{}AuWrapper", pascal_name);

    // Generate AUv3 wrapper (large ObjC implementation)
    let auv3_source = generate_auv3_wrapper_impl(plugin_name, &wrapper_class);

    // Generate AUv2 wrapper (large C implementation)
    let auv2_source = generate_auv2_wrapper_impl(plugin_name);

    format!("{}\n\n{}", auv3_source, auv2_source)
}

fn generate_auv3_wrapper_impl(plugin_name: &str, wrapper_class: &str) -> String {
    include_str!("auv3_wrapper.m")
        .replace("{{PLUGIN_NAME}}", plugin_name)
        .replace("{{WRAPPER_CLASS}}", wrapper_class)
}

fn generate_auv2_wrapper_impl(_plugin_name: &str) -> String {
    // AUv2 wrapper is very large - it's included in the main wrapper source
    // The full AUv2 implementation is generated separately
    // For now, return empty string as AUv2 is generated separately
    String::new()
}

/// Generate the AU extension ObjC implementation with plugin-specific class names.
///
/// When `has_editor` is true, uses the AUViewController-based template so that
/// Logic Pro (and other hosts that check NSExtensionPrincipalClass) shows the
/// custom WebView editor. Non-editor plugins use the plain NSObject template.
fn generate_au_extension_source(plugin_name: &str, has_editor: bool) -> String {
    let pascal_name = to_pascal_case(plugin_name);
    let wrapper_class = format!("Beamer{}AuWrapper", pascal_name);
    let extension_class = format!("Beamer{}AuExtension", pascal_name);
    let factory_func = format!("Beamer{}AuExtensionFactory", pascal_name);

    let template = if has_editor {
        include_str!("auv3_extension_editor.m")
    } else {
        include_str!("auv3_extension.m")
    };

    template
        .replace("{{PLUGIN_NAME}}", plugin_name)
        .replace("{{WRAPPER_CLASS}}", &wrapper_class)
        .replace("{{EXTENSION_CLASS}}", &extension_class)
        .replace("{{FACTORY_FUNC}}", &factory_func)
}
