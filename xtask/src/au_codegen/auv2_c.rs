// AUv2 C code generation template
// This file is included by auv2.rs via include!()

fn generate_auv2_wrapper_source(plugin_name: &str) -> String {
    let pascal_name = to_pascal_case(plugin_name);
    let factory_name = format!("Beamer{}Factory", pascal_name);

    include_str!("auv2_wrapper.c")
        .replace("{{PLUGIN_NAME}}", plugin_name)
        .replace("{{FACTORY_NAME}}", &factory_name)
}
