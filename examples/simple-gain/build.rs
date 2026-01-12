//! Build script for simple-gain plugin.
//!
//! This script ensures the ObjC static libraries from beamer-au are linked
//! into the final cdylib, including all ObjC class definitions.

fn main() {
    #[cfg(target_os = "macos")]
    {
        // Print all DEP_ environment variables for debugging
        for (key, value) in std::env::vars() {
            if key.starts_with("DEP_") {
                println!("cargo:warning=ENV: {}={}", key, value);
            }
        }

        // Find beamer-au's output directory by looking for the DEP_BEAMER_AU_NATIVE_OUT_DIR env var
        // This is set by beamer-au's build.rs due to the `links = "beamer_au_native"` key
        if let Ok(beamer_au_out) = std::env::var("DEP_BEAMER_AU_NATIVE_OUT_DIR") {
            println!("cargo:warning=Found beamer_au output dir: {}", beamer_au_out);
            println!("cargo:rustc-link-search=native={}", beamer_au_out);

            // Use -force_load to ensure ALL symbols from the static libraries are included,
            // including ObjC class definitions that aren't directly referenced.
            println!("cargo:rustc-link-arg=-Wl,-force_load,{}/libbeamer_au_objc.a", beamer_au_out);
            println!("cargo:rustc-link-arg=-Wl,-force_load,{}/libbeamer_au_extension.a", beamer_au_out);

            // Export the ObjC class symbols so they're visible to the ObjC runtime.
            // Without this, the classes are included but marked as local symbols,
            // and the runtime can't find them when loading the .appex bundle.
            println!("cargo:rustc-link-arg=-Wl,-exported_symbol,_OBJC_CLASS_$_BeamerAuWrapper");
            println!("cargo:rustc-link-arg=-Wl,-exported_symbol,_OBJC_CLASS_$_BeamerAuExtension");
            // Also export metaclasses (needed for +[Class alloc] etc.)
            println!("cargo:rustc-link-arg=-Wl,-exported_symbol,_OBJC_METACLASS_$_BeamerAuWrapper");
            println!("cargo:rustc-link-arg=-Wl,-exported_symbol,_OBJC_METACLASS_$_BeamerAuExtension");
            // Export the force_link function (needed by appex_main.m)
            println!("cargo:rustc-link-arg=-Wl,-exported_symbol,_beamer_au_appex_force_link");
            // Export the factory function (needed for AudioComponent in-process loading)
            println!("cargo:rustc-link-arg=-Wl,-exported_symbol,_BeamerAuExtensionFactory");
        } else {
            println!("cargo:warning=DEP_BEAMER_AU_NATIVE_OUT_DIR not found!");
        }
    }
}
