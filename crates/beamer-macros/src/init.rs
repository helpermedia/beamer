//! The `#[beamer::export]` attribute macro implementation.
//!
//! Reads Config.toml from the plugin crate's root directory, then
//! generates the `CONFIG` static and the format-specific entry points.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use beamer_utils::web_ui::{self, WebUi};

use crate::config_file::{category_variant, ConfigFile};

/// Map a category string from Config.toml to the corresponding token stream.
fn category_tokens(category: &str) -> Result<TokenStream, String> {
    let variant = format_ident!("{}", category_variant(category)?);
    Ok(quote! { ::beamer::prelude::Category::#variant })
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

/// Generate the WEBVIEW_ASSETS static and the with_webview_assets builder call.
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

    let builder_call = quote! { .with_webview_assets(&WEBVIEW_ASSETS) };

    (static_def, builder_call)
}

/// Generate the Config static from a parsed ConfigFile.
fn generate_config(config: &ConfigFile, manifest_dir: &str) -> Result<TokenStream, String> {
    let name = &config.name;
    let category = category_tokens(&config.category)?;
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

    // The web UI, found by the rule the bundler uses too. The expansion is
    // cached: it names the environment variables it depends on with
    // `option_env!`, which rustc records, so that cargo expands it again
    // when the dev server URL or the bundler's stamp of the files changes.
    // Changes to the contents of embedded files are tracked by
    // `include_bytes!()`.
    let dev_url = std::env::var(web_ui::DEV_URL_VAR).ok();
    let found = web_ui::find(std::path::Path::new(manifest_dir), dev_url)?;
    let dev_url_var = web_ui::DEV_URL_VAR;
    let stamp_var = web_ui::STAMP_VAR;
    let tracked = quote! {
        const _: Option<&str> = option_env!(#dev_url_var);
        const _: Option<&str> = option_env!(#stamp_var);
    };
    let (assets_static, web_ui_source) = match &found {
        None => (None, None),
        Some(WebUi::DevServer(url)) => (None, Some(quote! { .with_webview_url(#url) })),
        Some(WebUi::Embedded { dir, files }) => {
            let (static_def, builder) = generate_assets_tokens(files, dir);
            (Some(static_def), Some(builder))
        }
    };

    let editor_size = match (&found, config.editor_size) {
        (Some(_), Some((w, h))) => Some(quote! { .with_editor_size(#w, #h) }),
        (Some(_), None) => {
            return Err(
                "editor_size is required for a plugin with a web UI (e.g. editor_size = [400, 300])"
                    .to_string(),
            );
        }
        (None, _) => None,
    };

    let vst3_id = config.vst3_id.as_ref().map(|id| {
        quote! { .with_vst3_id(#id) }
    });
    let vst3_controller_id = config.vst3_controller_id.as_ref().map(|id| {
        quote! { .with_vst3_controller_id(#id) }
    });

    let sysex_bytes_per_block = config.sysex_bytes_per_block.map(|bytes| {
        quote! { .with_sysex_bytes_per_block(#bytes) }
    });

    let midi_input = config.midi_input.map(|enabled| {
        quote! { .with_midi_input(#enabled) }
    });
    let midi_output = config.midi_output.map(|enabled| {
        quote! { .with_midi_output(#enabled) }
    });

    let sample_accurate_automation = (config.sample_accurate_automation == Some(true))
        .then(|| quote! { .with_sample_accurate_automation() });

    let editor_background_color = config
        .editor_background_color
        .as_deref()
        .map(|hex| {
            let hex = hex.strip_prefix('#').unwrap_or(hex);
            if hex.len() != 6 {
                return Err(format!(
                    "editor_background_color must be a 6-digit hex string (e.g. \"#1a1a2e\"), got {:?}",
                    hex
                ));
            }
            let r = u8::from_str_radix(&hex[0..2], 16)
                .map_err(|e| format!("editor_background_color red: {e}"))?;
            let g = u8::from_str_radix(&hex[2..4], 16)
                .map_err(|e| format!("editor_background_color green: {e}"))?;
            let b = u8::from_str_radix(&hex[4..6], 16)
                .map_err(|e| format!("editor_background_color blue: {e}"))?;
            Ok(quote! { .with_editor_background_color([#r, #g, #b, 255]) })
        })
        .transpose()?;

    // The subsystem is the base of the bundle ids that xtask writes. A
    // plugin without a level still gets it: a diagnostics build and the
    // allocation check log without one.
    let logging = match config.log_level.as_ref() {
        Some(level) => {
            let variant = format_ident!("{}", crate::config_file::log_level_variant(level)?);
            quote! {
                .with_logging(
                    ::beamer::core::LogLevel::#variant,
                    concat!("com.beamer.", env!("CARGO_PKG_NAME")),
                )
            }
        }
        None => quote! {
            .with_log_subsystem(concat!("com.beamer.", env!("CARGO_PKG_NAME")))
        },
    };

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
        #tracked
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
        #web_ui_source
        #editor_size
        #vst3_id
        #vst3_controller_id
        #midi_input
        #midi_output
        #sysex_bytes_per_block
        #sample_accurate_automation
        #subcategories
        #editor_background_color
        #logging
        ;
    })
}

/// Main entry point for the `#[beamer::export]` attribute macro.
///
/// Generates the CONFIG static and the format entry points for the given
/// model struct.
pub fn export_impl(model: syn::Ident) -> Result<TokenStream, String> {
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

    // File dependency tracking: include_str! tells cargo to re-run when the file changes.
    let config_path_str = config_path.to_string_lossy().to_string();

    Ok(quote! {
        const _: &str = include_str!(#config_path_str);
        #config_tokens
        ::beamer::export_plugin!(CONFIG, #model);
    })
}

#[cfg(test)]
mod tests {
    use beamer_core::config::Category;

    use super::*;
    use crate::config_file::CATEGORIES;

    /// Name of a variant as `CATEGORIES` spells it.
    ///
    /// The match is exhaustive, so a new `Category` variant fails to compile
    /// here until it is added to this test.
    fn variant_name(category: Category) -> &'static str {
        match category {
            Category::Effect => "Effect",
            Category::Instrument => "Instrument",
            Category::MidiEffect => "MidiEffect",
            Category::Generator => "Generator",
        }
    }

    #[test]
    fn test_categories_match_core() {
        // Every variant has exactly one name in Config.toml. The table has
        // no entry for a variant that does not exist.
        let variants = [
            Category::Effect,
            Category::Instrument,
            Category::MidiEffect,
            Category::Generator,
        ];
        assert_eq!(CATEGORIES.len(), variants.len());
        for category in variants {
            let name = variant_name(category);
            assert_eq!(
                CATEGORIES.iter().filter(|(_, variant)| *variant == name).count(),
                1,
                "variant {}",
                name
            );
        }
    }

    #[test]
    fn test_category_tokens() {
        let tokens = category_tokens("midi_effect").unwrap().to_string();
        assert!(tokens.ends_with("Category :: MidiEffect"), "{}", tokens);

        // An unknown name is an error that lists the valid ones.
        let error = category_tokens("synth").unwrap_err();
        assert!(error.contains("\"generator\"") && error.contains("\"synth\""), "{}", error);
    }
}
