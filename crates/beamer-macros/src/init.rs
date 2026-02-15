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

    // Auto-detect webview/index.html for editor HTML.
    //
    // NOTE: This existence check runs at macro expansion time, so its result
    // is baked into the cached compilation. If a user adds webview/index.html
    // after an initial build without modifying any .rs file, the stale cache
    // won't pick it up until something else forces recompilation (e.g.
    // `cargo clean` or touching a source file). Proc macros cannot emit
    // `cargo:rerun-if-changed`, so there is no way to track a path that
    // doesn't exist yet. In practice, adding a webview UI also requires
    // source changes, so this rarely causes issues.
    let webview_html_path = std::path::Path::new(manifest_dir).join("webview/index.html");
    let has_webview = webview_html_path.exists();

    let editor_html = if has_webview {
        Some(quote! { .with_editor_html(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/webview/index.html"))) })
    } else {
        None
    };

    // has_editor is true if explicitly set and no webview (with_editor_html already sets it)
    let has_editor = if !has_webview && config.has_editor.unwrap_or(false) {
        Some(quote! { .with_editor() })
    } else {
        None
    };

    let editor_size = config.editor_size.as_ref().map(|size| {
        let w = size.0;
        let h = size.1;
        quote! { .with_editor_size(#w, #h) }
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
        #has_editor
        #editor_html
        #editor_size
        #vst3_id
        #vst3_controller_id
        #sysex_slots
        #sysex_buffer_size
        #subcategories
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
/// Generates the CONFIG static, optional presets, and format entry points
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
