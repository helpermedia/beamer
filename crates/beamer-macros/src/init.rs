//! The `#[beamer::export]` attribute macro implementation.
//!
//! Reads Config.toml and (optionally) Presets.toml from the plugin crate's
//! root directory, then generates the `CONFIG` static, preset implementation,
//! and format-specific entry points.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use beamer_utils::fnv1a_32;

use crate::config_file::{ConfigFile, PresetsFile};

/// Map a category string from Config.toml to the corresponding token stream.
fn category_tokens(category: &str) -> TokenStream {
    match category {
        "effect" => quote! { ::beamer::prelude::Category::Effect },
        "instrument" => quote! { ::beamer::prelude::Category::Instrument },
        "midi_effect" => quote! { ::beamer::prelude::Category::MidiEffect },
        "generator" => quote! { ::beamer::prelude::Category::Generator },
        _ => unreachable!("category validated before calling"),
    }
}

/// Map a subcategory string from Config.toml to the corresponding token stream.
fn subcategory_tokens(sub: &str) -> Result<TokenStream, String> {
    let tokens = match sub {
        "analyzer" => quote! { ::beamer::prelude::Subcategory::Analyzer },
        "bass" => quote! { ::beamer::prelude::Subcategory::Bass },
        "channel_strip" => quote! { ::beamer::prelude::Subcategory::ChannelStrip },
        "delay" => quote! { ::beamer::prelude::Subcategory::Delay },
        "distortion" => quote! { ::beamer::prelude::Subcategory::Distortion },
        "drums" => quote! { ::beamer::prelude::Subcategory::Drums },
        "dynamics" => quote! { ::beamer::prelude::Subcategory::Dynamics },
        "eq" => quote! { ::beamer::prelude::Subcategory::Eq },
        "filter" => quote! { ::beamer::prelude::Subcategory::Filter },
        "generator" => quote! { ::beamer::prelude::Subcategory::Generator },
        "guitar" => quote! { ::beamer::prelude::Subcategory::Guitar },
        "mastering" => quote! { ::beamer::prelude::Subcategory::Mastering },
        "microphone" => quote! { ::beamer::prelude::Subcategory::Microphone },
        "modulation" => quote! { ::beamer::prelude::Subcategory::Modulation },
        "network" => quote! { ::beamer::prelude::Subcategory::Network },
        "pitch_shift" => quote! { ::beamer::prelude::Subcategory::PitchShift },
        "restoration" => quote! { ::beamer::prelude::Subcategory::Restoration },
        "reverb" => quote! { ::beamer::prelude::Subcategory::Reverb },
        "spatial" => quote! { ::beamer::prelude::Subcategory::Spatial },
        "surround" => quote! { ::beamer::prelude::Subcategory::Surround },
        "tools" => quote! { ::beamer::prelude::Subcategory::Tools },
        "vocals" => quote! { ::beamer::prelude::Subcategory::Vocals },
        "drum" => quote! { ::beamer::prelude::Subcategory::Drum },
        "external" => quote! { ::beamer::prelude::Subcategory::External },
        "piano" => quote! { ::beamer::prelude::Subcategory::Piano },
        "sampler" => quote! { ::beamer::prelude::Subcategory::Sampler },
        "synth" => quote! { ::beamer::prelude::Subcategory::Synth },
        "mono" => quote! { ::beamer::prelude::Subcategory::Mono },
        "stereo" => quote! { ::beamer::prelude::Subcategory::Stereo },
        "ambisonics" => quote! { ::beamer::prelude::Subcategory::Ambisonics },
        "up_down_mix" => quote! { ::beamer::prelude::Subcategory::UpDownMix },
        "only_realtime" => quote! { ::beamer::prelude::Subcategory::OnlyRealTime },
        "only_offline" => quote! { ::beamer::prelude::Subcategory::OnlyOfflineProcess },
        "no_offline" => quote! { ::beamer::prelude::Subcategory::NoOfflineProcess },
        other => return Err(format!("unknown subcategory {:?}", other)),
    };
    Ok(tokens)
}

/// Recursively scan a directory for web assets, returning relative paths.
///
/// Skips dotfiles, dot-directories, node_modules and source maps.
fn scan_webview_dir(
    dir: &std::path::Path,
    manifest_dir: &str,
) -> Result<Vec<String>, String> {
    let mut files = Vec::new();
    scan_dir_recursive(dir, dir, &mut files)
        .map_err(|e| format!("failed to scan {}: {}", dir.display(), e))?;
    files.sort();
    let _ = manifest_dir; // used by caller for include_bytes path
    Ok(files)
}

fn scan_dir_recursive(
    base: &std::path::Path,
    dir: &std::path::Path,
    files: &mut Vec<String>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Skip dotfiles and dot-directories
        if name_str.starts_with('.') {
            continue;
        }

        // Skip node_modules
        if name_str == "node_modules" {
            continue;
        }

        let path = entry.path();
        if path.is_dir() {
            scan_dir_recursive(base, &path, files)?;
        } else if path.is_file() {
            // Skip source maps
            if name_str.ends_with(".map") {
                continue;
            }

            let relative = path
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            files.push(relative);
        }
    }
    Ok(())
}

/// Generate the WEBVIEW_ASSETS static and the with_gui_assets builder call.
///
/// `subdir` is the path relative to CARGO_MANIFEST_DIR (e.g. "webview" or "webview/dist").
fn generate_assets_tokens(asset_paths: &[String], subdir: &str) -> (TokenStream, TokenStream) {
    let entries: Vec<TokenStream> = asset_paths
        .iter()
        .map(|path| {
            let include_path = format!("{subdir}/{path}");
            quote! {
                ::beamer::prelude::EmbeddedAsset {
                    path: #path,
                    data: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/", #include_path)),
                }
            }
        })
        .collect();

    let static_def = quote! {
        static WEBVIEW_ASSETS: ::beamer::prelude::EmbeddedAssets =
            ::beamer::prelude::EmbeddedAssets::new(&[
                #(#entries),*
            ]);
    };

    let builder_call = quote! { .with_gui_assets(&WEBVIEW_ASSETS) };

    (static_def, builder_call)
}

/// Generate the Config static from a parsed ConfigFile.
fn generate_config(config: &ConfigFile, manifest_dir: &str) -> Result<TokenStream, String> {
    let name = &config.name;
    let category = category_tokens(&config.category);
    let manufacturer_code = &config.manufacturer_code;
    let plugin_code = &config.plugin_code;

    // Optional builder calls
    let vendor = config.vendor.as_ref().map(|v| {
        quote! { .with_vendor(#v) }
    });
    let url = config.url.as_ref().map(|u| {
        quote! { .with_url(#u) }
    });
    let email = config.email.as_ref().map(|e| {
        quote! { .with_email(#e) }
    });

    // Webview asset detection and embedding.
    //
    // Detection rules:
    // 1. BEAMER_DEV_URL set -> Load from URL (any project type)
    // 2. package.json + dist/ exists -> Embed all files from webview/dist/
    // 3. package.json without dist/ -> No assets (build not run yet)
    // 4. webview/ exists (no package.json) -> Embed all files from webview/
    // 5. No webview/ directory -> No GUI
    //
    // NOTE: Directory scanning runs at macro expansion time, so its results
    // are baked into the cached compilation. After running a web build for
    // the first time, you may need to touch a .rs file to trigger
    // recompilation. Changes to existing file contents are tracked
    // automatically by include_bytes!().
    let dev_url = std::env::var("BEAMER_DEV_URL").ok();
    let webview_dir = std::path::Path::new(manifest_dir).join("webview");
    let has_package_json = webview_dir.join("package.json").exists();
    let dist_dir = webview_dir.join("dist");

    let (assets_static, gui_source, has_webview) = if let Some(url) = &dev_url {
        // Dev server mode: load from URL
        (None, Some(quote! { .with_gui_url(#url) }), true)
    } else if has_package_json && dist_dir.exists() {
        // Framework project with built output: embed from dist/
        let assets = scan_webview_dir(&dist_dir, manifest_dir)?;
        if assets.is_empty() {
            (None, None, false)
        } else {
            let (static_def, builder) = generate_assets_tokens(&assets, "webview/dist");
            (Some(static_def), Some(builder), true)
        }
    } else if !has_package_json && webview_dir.exists() {
        // Plain HTML project: embed from webview/
        let assets = scan_webview_dir(&webview_dir, manifest_dir)?;
        if assets.is_empty() {
            (None, None, false)
        } else {
            let (static_def, builder) = generate_assets_tokens(&assets, "webview");
            (Some(static_def), Some(builder), true)
        }
    } else {
        (None, None, false)
    };

    // has_gui is true if explicitly set and no webview (with_gui_assets/with_gui_url already sets it)
    let has_gui = if !has_webview && config.has_gui.unwrap_or(false) {
        Some(quote! { .with_gui() })
    } else {
        None
    };

    let gui_size = config.gui_size.as_ref().map(|size| {
        let w = size.0;
        let h = size.1;
        quote! { .with_gui_size(#w, #h) }
    });

    let vst3_id = config.vst3_id.as_ref().map(|id| {
        quote! { .with_vst3_id(#id) }
    });
    let vst3_controller_id = config.vst3_controller_id.as_ref().map(|id| {
        quote! { .with_vst3_controller_id(#id) }
    });

    let sysex_slots = config.sysex_slots.map(|slots| {
        quote! { .with_sysex_slots(#slots) }
    });

    let sysex_buffer_size = config.sysex_buffer_size.map(|size| {
        quote! { .with_sysex_buffer_size(#size) }
    });

    let gui_background_color = config
        .gui_background_color
        .as_deref()
        .map(|hex| {
            let hex = hex.strip_prefix('#').unwrap_or(hex);
            if hex.len() != 6 {
                return Err(format!(
                    "gui_background_color must be a 6-digit hex string (e.g. \"#1a1a2e\"), got {:?}",
                    hex
                ));
            }
            let r = u8::from_str_radix(&hex[0..2], 16)
                .map_err(|e| format!("gui_background_color red: {e}"))?;
            let g = u8::from_str_radix(&hex[2..4], 16)
                .map_err(|e| format!("gui_background_color green: {e}"))?;
            let b = u8::from_str_radix(&hex[4..6], 16)
                .map_err(|e| format!("gui_background_color blue: {e}"))?;
            Ok(quote! { .with_gui_background_color([#r, #g, #b, 255]) })
        })
        .transpose()?;

    let subcategories = if let Some(subs) = &config.subcategories {
        let sub_tokens: Vec<TokenStream> = subs
            .iter()
            .map(|s| subcategory_tokens(s))
            .collect::<Result<Vec<_>, _>>()?;
        Some(quote! { .with_subcategories(&[#(#sub_tokens),*]) })
    } else {
        None
    };

    Ok(quote! {
        #assets_static

        pub static CONFIG: ::beamer::prelude::Config = ::beamer::prelude::Config::new(
            #name,
            #category,
            #manufacturer_code,
            #plugin_code,
        )
        #vendor
        #url
        #email
        .with_version(env!("CARGO_PKG_VERSION"))
        #has_gui
        #gui_source
        #gui_size
        #vst3_id
        #vst3_controller_id
        #sysex_slots
        #sysex_buffer_size
        #subcategories
        #gui_background_color
        ;
    })
}

/// Generate the FactoryPresets implementation from a parsed PresetsFile.
fn generate_presets(presets: &PresetsFile, descriptor: &syn::Ident) -> Result<TokenStream, String> {
    let count = presets.preset.len();

    // Generate info match arms
    let info_arms: Vec<TokenStream> = presets
        .preset
        .iter()
        .enumerate()
        .map(|(idx, preset)| {
            let name = &preset.name;
            quote! {
                #idx => Some(::beamer::core::preset::PresetInfo { name: #name }),
            }
        })
        .collect();

    // Generate static value arrays and match arms
    let mut values_statics = Vec::new();
    let mut values_arms = Vec::new();

    for (idx, preset) in presets.preset.iter().enumerate() {
        let static_name = format_ident!("__BEAMER_PRESET_{}_VALUES", idx);

        let values: Vec<TokenStream> = preset
            .values
            .iter()
            .filter_map(|(key, val)| {
                let plain_value = match val {
                    toml::Value::Float(f) => *f,
                    toml::Value::Integer(i) => *i as f64,
                    _ => return None, // skip non-numeric values
                };
                let hash = fnv1a_32(key);
                Some(quote! {
                    ::beamer::core::preset::PresetValue {
                        id: #hash,
                        plain_value: #plain_value,
                    }
                })
            })
            .collect();

        let values_count = values.len();

        values_statics.push(quote! {
            static #static_name: [::beamer::core::preset::PresetValue; #values_count] = [
                #(#values),*
            ];
        });

        values_arms.push(quote! {
            #idx => &#static_name,
        });
    }

    Ok(quote! {
        #(#values_statics)*

        pub struct __BeamerPresets;

        impl ::beamer::core::preset::FactoryPresets for __BeamerPresets {
            type Parameters = <#descriptor as ::beamer::prelude::HasParameters>::Parameters;

            fn count() -> usize {
                #count
            }

            fn info(index: usize) -> Option<::beamer::core::preset::PresetInfo> {
                match index {
                    #(#info_arms)*
                    _ => None,
                }
            }

            fn values(index: usize) -> &'static [::beamer::core::preset::PresetValue] {
                match index {
                    #(#values_arms)*
                    _ => &[],
                }
            }
        }
    })
}

/// Main entry point for the `#[beamer::export]` attribute macro.
///
/// Generates the CONFIG static, optional presets and format entry points
/// for the given descriptor struct.
pub fn export_impl(descriptor: syn::Ident) -> Result<TokenStream, String> {
    // Read Config.toml from the crate's root directory
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .map_err(|_| "CARGO_MANIFEST_DIR not set".to_string())?;

    let config_path = std::path::Path::new(&manifest_dir).join("Config.toml");
    let config_str = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("failed to read {}: {}", config_path.display(), e))?;

    let config: ConfigFile =
        toml::from_str(&config_str).map_err(|e| format!("invalid Config.toml: {}", e))?;

    config.validate()?;

    // Generate Config static
    let config_tokens = generate_config(&config, &manifest_dir)?;

    // Check for Presets.toml
    let presets_path = std::path::Path::new(&manifest_dir).join("Presets.toml");
    let has_presets = presets_path.exists();

    let presets_tokens = if has_presets {
        let presets_str = std::fs::read_to_string(&presets_path)
            .map_err(|e| format!("failed to read {}: {}", presets_path.display(), e))?;

        let presets: PresetsFile =
            toml::from_str(&presets_str).map_err(|e| format!("invalid Presets.toml: {}", e))?;

        Some(generate_presets(&presets, &descriptor)?)
    } else {
        None
    };

    // Generate export call
    let export_tokens = if has_presets {
        quote! {
            ::beamer::export_plugin!(CONFIG, #descriptor, __BeamerPresets);
        }
    } else {
        quote! {
            ::beamer::export_plugin!(CONFIG, #descriptor);
        }
    };

    // File dependency tracking: include_str! tells cargo to re-run when files change
    let config_path_str = config_path.to_string_lossy().to_string();
    let file_tracking = if has_presets {
        let presets_path_str = presets_path.to_string_lossy().to_string();
        quote! {
            const _: &str = include_str!(#config_path_str);
            const _: &str = include_str!(#presets_path_str);
        }
    } else {
        quote! {
            const _: &str = include_str!(#config_path_str);
        }
    };

    Ok(quote! {
        #file_tracking
        #config_tokens
        #presets_tokens
        #export_tokens
    })
}
