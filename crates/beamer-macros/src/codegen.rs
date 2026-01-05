//! Code generation for the derive macro.
//!
//! This module generates the Rust code for the `Params` and `Parameters` trait
//! implementations from the validated IR.

use proc_macro2::TokenStream;
use quote::quote;

use crate::ir::{
    FieldIR, ParamDefault, ParamFieldIR, ParamKind, ParamsIR, SmoothingStyle,
};

/// Generate all code for the derive macro.
pub fn generate(ir: &ParamsIR) -> TokenStream {
    let const_ids = generate_const_ids(ir);
    let unit_consts = generate_unit_consts(ir);
    let collision_check = generate_collision_check(ir);
    let units_impl = generate_units_impl(ir);
    let params_impl = generate_params_impl(ir);
    let parameters_impl = generate_parameters_impl(ir);
    let set_unit_ids_impl = generate_set_unit_ids(ir);
    let default_impl = generate_default_impl(ir);

    quote! {
        #const_ids
        #unit_consts
        #collision_check
        #units_impl
        #params_impl
        #parameters_impl
        #set_unit_ids_impl
        #default_impl
    }
}

/// Generate const ID declarations for each parameter.
fn generate_const_ids(ir: &ParamsIR) -> TokenStream {
    let struct_name = &ir.struct_name;

    let const_defs: Vec<TokenStream> = ir
        .param_fields()
        .map(|param| {
            let const_name = param.const_name();
            let hash = param.hash_id;
            quote! {
                const #const_name: u32 = #hash;
            }
        })
        .collect();

    if const_defs.is_empty() {
        quote! {}
    } else {
        quote! {
            impl #struct_name {
                #(#const_defs)*
            }
        }
    }
}

/// Generate unit ID constants for each nested field and flat group.
fn generate_unit_consts(ir: &ParamsIR) -> TokenStream {
    let struct_name = &ir.struct_name;

    // Generate constants for flat groups first (they get IDs 1, 2, 3, ...)
    let flat_groups = ir.flat_group_names();
    let flat_group_consts: Vec<TokenStream> = flat_groups
        .iter()
        .enumerate()
        .map(|(idx, group_name)| {
            // Convert group name to uppercase const name (e.g., "Filter" -> "UNIT_GROUP_FILTER")
            let const_name = syn::Ident::new(
                &format!("UNIT_GROUP_{}", group_name.to_uppercase().replace(' ', "_")),
                proc_macro2::Span::call_site(),
            );
            let unit_id = (idx + 1) as i32; // Start at 1 (0 is root)
            quote! {
                /// Unit ID for the flat parameter group.
                pub const #const_name: ::beamer::core::params::UnitId = #unit_id;
            }
        })
        .collect();

    // Nested groups get IDs after flat groups
    let flat_group_count = flat_groups.len() as i32;
    let nested_consts: Vec<TokenStream> = ir
        .nested_fields()
        .enumerate()
        .map(|(idx, nested)| {
            let const_name = syn::Ident::new(
                &format!("UNIT_{}", nested.field_name.to_string().to_uppercase()),
                nested.span,
            );
            let unit_id = flat_group_count + (idx as i32) + 1;
            quote! {
                /// Unit ID for the nested parameter group.
                pub const #const_name: ::beamer::core::params::UnitId = #unit_id;
            }
        })
        .collect();

    if flat_group_consts.is_empty() && nested_consts.is_empty() {
        quote! {}
    } else {
        quote! {
            impl #struct_name {
                #(#flat_group_consts)*
                #(#nested_consts)*
            }
        }
    }
}

/// Generate the `Units` trait implementation.
///
/// For structs with nested groups or flat groups, this generates unit discovery
/// that includes both flat groups and deeply nested ones.
fn generate_units_impl(ir: &ParamsIR) -> TokenStream {
    let struct_name = &ir.struct_name;
    let (impl_generics, ty_generics, where_clause) = ir.generics.split_for_impl();

    let flat_groups = ir.flat_group_names();
    let has_flat_groups = !flat_groups.is_empty();
    let has_nested = ir.has_nested();

    if !has_flat_groups && !has_nested {
        // No groups at all = use default Units impl (root only)
        return quote! {
            impl #impl_generics ::beamer::core::params::Units for #struct_name #ty_generics #where_clause {}
        };
    }

    // Generate flat group UnitInfo entries
    let flat_group_count = flat_groups.len();
    let flat_unit_infos: Vec<TokenStream> = flat_groups
        .iter()
        .enumerate()
        .map(|(idx, group_name)| {
            let unit_id = (idx + 1) as i32;
            quote! {
                #idx => Some(::beamer::core::params::UnitInfo {
                    id: #unit_id,
                    name: #group_name,
                    parent_id: 0, // All flat groups are children of root
                }),
            }
        })
        .collect();

    if has_nested {
        // Flat groups + nested groups: combine static flat groups with dynamic nested collection
        quote! {
            impl #impl_generics ::beamer::core::params::Units for #struct_name #ty_generics #where_clause {
                fn unit_count(&self) -> usize {
                    use ::beamer::core::param_types::Params;
                    // Count = 1 (root) + flat groups + nested units recursively
                    let flat_count = #flat_group_count;
                    let mut units = Vec::new();
                    self.collect_units(&mut units, (flat_count + 1) as i32, 0);
                    1 + flat_count + units.len()
                }

                fn unit_info(&self, index: usize) -> Option<::beamer::core::params::UnitInfo> {
                    use ::beamer::core::param_types::Params;
                    if index == 0 {
                        return Some(::beamer::core::params::UnitInfo::root());
                    }

                    // Check flat groups first (indices 1..=flat_group_count)
                    let flat_idx = index - 1;
                    if flat_idx < #flat_group_count {
                        return match flat_idx {
                            #(#flat_unit_infos)*
                            _ => None,
                        };
                    }

                    // Then check nested groups
                    let mut units = Vec::new();
                    self.collect_units(&mut units, (#flat_group_count + 1) as i32, 0);
                    let nested_idx = index - 1 - #flat_group_count;
                    units.get(nested_idx).cloned()
                }
            }
        }
    } else {
        // Only flat groups, no nesting
        quote! {
            impl #impl_generics ::beamer::core::params::Units for #struct_name #ty_generics #where_clause {
                fn unit_count(&self) -> usize {
                    1 + #flat_group_count // root + flat groups
                }

                fn unit_info(&self, index: usize) -> Option<::beamer::core::params::UnitInfo> {
                    if index == 0 {
                        return Some(::beamer::core::params::UnitInfo::root());
                    }

                    let flat_idx = index - 1;
                    match flat_idx {
                        #(#flat_unit_infos)*
                        _ => None,
                    }
                }
            }
        }
    }
}

/// Generate the `set_unit_ids()` method for initializing param unit IDs.
///
/// This handles both flat groups (group="...") and nested groups (#[nested(...)]).
/// For flat groups, it sets unit_id directly on the params based on their group.
/// For nested groups, it uses the recursive `assign_unit_ids` method.
fn generate_set_unit_ids(ir: &ParamsIR) -> TokenStream {
    let struct_name = &ir.struct_name;
    let (impl_generics, ty_generics, where_clause) = ir.generics.split_for_impl();

    let flat_groups = ir.flat_group_names();
    let has_flat_groups = !flat_groups.is_empty();
    let has_nested = ir.has_nested();

    if !has_flat_groups && !has_nested {
        // No groups at all = no-op set_unit_ids
        return quote! {
            impl #impl_generics #struct_name #ty_generics #where_clause {
                /// Initialize unit IDs for parameters.
                ///
                /// No groups in this struct, so this is a no-op.
                pub fn set_unit_ids(&mut self) {}
            }
        };
    }

    // Build a map of group name -> unit ID
    let group_to_unit_id: std::collections::HashMap<&str, i32> = flat_groups
        .iter()
        .enumerate()
        .map(|(idx, name)| (*name, (idx + 1) as i32))
        .collect();

    // Generate statements to set unit_id on params with flat groups
    let flat_group_assignments: Vec<TokenStream> = ir
        .param_fields()
        .filter_map(|param| {
            param.attrs.group.as_ref().map(|group_name| {
                let field = &param.field_name;
                let unit_id = group_to_unit_id.get(group_name.as_str()).copied().unwrap_or(0);
                quote! {
                    self.#field.set_unit_id(#unit_id);
                }
            })
        })
        .collect();

    let flat_group_count = flat_groups.len() as i32;
    let nested_init = if has_nested {
        quote! {
            use ::beamer::core::param_types::Params;
            // Nested groups start after flat groups
            self.assign_unit_ids(#flat_group_count + 1, 0);
        }
    } else {
        quote! {}
    };

    quote! {
        impl #impl_generics #struct_name #ty_generics #where_clause {
            /// Initialize unit IDs for all parameters.
            ///
            /// This method assigns unit IDs to parameters with `group` attributes
            /// and recursively assigns unit IDs to nested parameter groups.
            /// Unit IDs are assigned sequentially starting from 1 (0 is reserved for root).
            ///
            /// Call this once after construction to set up VST3 unit hierarchy.
            ///
            /// # Example
            ///
            /// ```ignore
            /// let mut params = SynthParams::default();
            /// params.set_unit_ids();
            /// ```
            pub fn set_unit_ids(&mut self) {
                // Set unit IDs for flat groups
                #(#flat_group_assignments)*
                // Initialize nested groups
                #nested_init
            }
        }
    }
}

/// Generate compile-time collision detection.
fn generate_collision_check(ir: &ParamsIR) -> TokenStream {
    let param_fields: Vec<_> = ir.param_fields().collect();

    if param_fields.len() < 2 {
        // No collision possible with 0 or 1 parameters
        return quote! {};
    }

    let id_pairs: Vec<TokenStream> = param_fields
        .iter()
        .map(|param| {
            let id_str = &param.string_id;
            let hash = param.hash_id;
            quote! { (#id_str, #hash) }
        })
        .collect();

    let struct_name = &ir.struct_name;

    quote! {
        const _: () = {
            const IDS: &[(&str, u32)] = &[#(#id_pairs),*];

            // Compile-time collision detection
            let mut i = 0;
            while i < IDS.len() {
                let mut j = i + 1;
                while j < IDS.len() {
                    if IDS[i].1 == IDS[j].1 {
                        panic!(concat!(
                            "Parameter ID hash collision in ",
                            stringify!(#struct_name),
                            ": two IDs hash to the same value"
                        ));
                    }
                    j += 1;
                }
                i += 1;
            }
        };
    }
}

/// Generate the `Params` trait implementation.
fn generate_params_impl(ir: &ParamsIR) -> TokenStream {
    let struct_name = &ir.struct_name;
    let (impl_generics, ty_generics, where_clause) = ir.generics.split_for_impl();

    let count_impl = generate_count(ir);
    let iter_impl = generate_iter(ir);
    let by_id_impl = generate_by_id(ir);
    let save_state_impl = generate_save_state(ir);
    let load_state_impl = generate_load_state(ir);
    let set_all_unit_ids_impl = generate_set_all_unit_ids(ir);
    let nested_discovery_impl = generate_nested_discovery(ir);
    let set_sample_rate_impl = generate_set_sample_rate(ir);
    let reset_smoothing_impl = generate_reset_smoothing(ir);

    quote! {
        impl #impl_generics ::beamer::core::param_types::Params for #struct_name #ty_generics #where_clause {
            fn count(&self) -> usize {
                #count_impl
            }

            fn iter(&self) -> Box<dyn Iterator<Item = &dyn ::beamer::core::param_types::ParamRef> + '_> {
                #iter_impl
            }

            fn by_id(&self, id: ::beamer::core::types::ParamId) -> Option<&dyn ::beamer::core::param_types::ParamRef> {
                #by_id_impl
            }

            fn by_id_mut(&mut self, id: ::beamer::core::types::ParamId) -> Option<&dyn ::beamer::core::param_types::ParamRef> {
                self.by_id(id)
            }

            #set_all_unit_ids_impl

            #nested_discovery_impl

            #save_state_impl

            #load_state_impl

            #set_sample_rate_impl

            #reset_smoothing_impl
        }
    }
}

/// Generate the `set_all_unit_ids()` method for the Params trait.
fn generate_set_all_unit_ids(ir: &ParamsIR) -> TokenStream {
    if ir.param_count() == 0 {
        // No direct params = use default no-op
        return quote! {};
    }

    // Generate statements to set unit_id on each direct param field
    let assignments: Vec<TokenStream> = ir
        .param_fields()
        .map(|param| {
            let field = &param.field_name;
            quote! {
                self.#field.set_unit_id(unit_id);
            }
        })
        .collect();

    quote! {
        fn set_all_unit_ids(&mut self, unit_id: ::beamer::core::params::UnitId) {
            #(#assignments)*
        }
    }
}

/// Generate the nested group discovery methods for the Params trait.
fn generate_nested_discovery(ir: &ParamsIR) -> TokenStream {
    if !ir.has_nested() {
        // No nested fields = use default implementations (return 0/None)
        return quote! {};
    }

    let nested_count = ir.nested_fields().count();

    // Generate match arms for nested_group()
    let group_match_arms: Vec<TokenStream> = ir
        .nested_fields()
        .enumerate()
        .map(|(idx, nested)| {
            let field = &nested.field_name;
            let name = &nested.group_name;
            quote! {
                #idx => Some((#name, &self.#field as &dyn ::beamer::core::param_types::Params)),
            }
        })
        .collect();

    // Generate match arms for nested_group_mut()
    let group_mut_match_arms: Vec<TokenStream> = ir
        .nested_fields()
        .enumerate()
        .map(|(idx, nested)| {
            let field = &nested.field_name;
            let name = &nested.group_name;
            quote! {
                #idx => Some((#name, &mut self.#field as &mut dyn ::beamer::core::param_types::Params)),
            }
        })
        .collect();

    quote! {
        fn nested_count(&self) -> usize {
            #nested_count
        }

        fn nested_group(&self, index: usize) -> Option<(&'static str, &dyn ::beamer::core::param_types::Params)> {
            match index {
                #(#group_match_arms)*
                _ => None,
            }
        }

        fn nested_group_mut(&mut self, index: usize) -> Option<(&'static str, &mut dyn ::beamer::core::param_types::Params)> {
            match index {
                #(#group_mut_match_arms)*
                _ => None,
            }
        }
    }
}

/// Generate the count() method body.
fn generate_count(ir: &ParamsIR) -> TokenStream {
    let param_count = ir.param_count();

    if ir.has_nested() {
        let nested_counts: Vec<TokenStream> = ir
            .nested_fields()
            .map(|nested| {
                let field = &nested.field_name;
                // Use fully qualified syntax to disambiguate between Params::count and Parameters::count
                quote! { ::beamer::core::param_types::Params::count(&self.#field) }
            })
            .collect();

        quote! {
            #param_count #(+ #nested_counts)*
        }
    } else {
        quote! { #param_count }
    }
}

/// Generate the iter() method body.
fn generate_iter(ir: &ParamsIR) -> TokenStream {
    let param_iters: Vec<TokenStream> = ir
        .param_fields()
        .map(|param| {
            let field = &param.field_name;
            quote! { &self.#field as &dyn ::beamer::core::param_types::ParamRef }
        })
        .collect();

    let nested_chains: Vec<TokenStream> = ir
        .nested_fields()
        .map(|nested| {
            let field = &nested.field_name;
            quote! { .chain(self.#field.iter()) }
        })
        .collect();

    if param_iters.is_empty() && nested_chains.is_empty() {
        quote! { Box::new(::std::iter::empty()) }
    } else if param_iters.is_empty() {
        // Only nested fields
        let first_nested = &ir.nested_fields().next().unwrap().field_name;
        let rest_nested: Vec<TokenStream> = ir
            .nested_fields()
            .skip(1)
            .map(|n| {
                let field = &n.field_name;
                quote! { .chain(self.#field.iter()) }
            })
            .collect();
        quote! {
            Box::new(self.#first_nested.iter() #(#rest_nested)*)
        }
    } else {
        quote! {
            Box::new(
                [#(#param_iters),*].into_iter()
                    #(#nested_chains)*
            )
        }
    }
}

/// Generate the by_id() method body.
fn generate_by_id(ir: &ParamsIR) -> TokenStream {
    let struct_name = &ir.struct_name;

    let match_arms: Vec<TokenStream> = ir
        .param_fields()
        .map(|param| {
            let field = &param.field_name;
            let const_name = param.const_name();
            quote! {
                #struct_name::#const_name => Some(&self.#field),
            }
        })
        .collect();

    let nested_lookups: Vec<TokenStream> = ir
        .nested_fields()
        .map(|nested| {
            let field = &nested.field_name;
            quote! {
                if let Some(param) = self.#field.by_id(id) {
                    return Some(param);
                }
            }
        })
        .collect();

    if match_arms.is_empty() && nested_lookups.is_empty() {
        quote! { None }
    } else {
        quote! {
            match id {
                #(#match_arms)*
                _ => {
                    #(#nested_lookups)*
                    None
                }
            }
        }
    }
}

/// Generate the save_state_prefixed() method body.
///
/// This generates path-based serialization that supports nested groups.
/// Paths like "filter/cutoff" disambiguate parameters with the same ID
/// in different nested groups.
fn generate_save_state(ir: &ParamsIR) -> TokenStream {
    // Generate saves for direct params using string IDs with prefix
    let param_saves: Vec<TokenStream> = ir
        .param_fields()
        .map(|param| {
            let field = &param.field_name;
            let id_str = &param.string_id;
            quote! {
                // Build path: prefix + "/" + id (or just id if prefix is empty)
                let path = if prefix.is_empty() {
                    #id_str.to_string()
                } else {
                    format!("{}/{}", prefix, #id_str)
                };
                let path_bytes = path.as_bytes();
                data.push(path_bytes.len() as u8);
                data.extend_from_slice(path_bytes);
                data.extend_from_slice(&self.#field.get_normalized().to_le_bytes());
            }
        })
        .collect();

    // Generate recursive calls for nested groups with updated prefix
    let nested_saves: Vec<TokenStream> = ir
        .nested_fields()
        .map(|nested| {
            let field = &nested.field_name;
            let group_name = &nested.group_name;
            quote! {
                // Build nested prefix: prefix + "/" + group_name (or just group_name)
                let nested_prefix = if prefix.is_empty() {
                    #group_name.to_string()
                } else {
                    format!("{}/{}", prefix, #group_name)
                };
                self.#field.save_state_prefixed(data, &nested_prefix);
            }
        })
        .collect();

    let param_count = ir.param_count();
    // Estimate capacity: ~20 bytes per param (path_len + avg 10 char path + 8 byte f64)
    let estimated_capacity = param_count * 20;

    quote! {
        fn save_state_prefixed(&self, data: &mut Vec<u8>, prefix: &str) {
            #(#param_saves)*
            #(#nested_saves)*
        }

        fn save_state(&self) -> Vec<u8> {
            let mut data = Vec::with_capacity(#estimated_capacity);
            self.save_state_prefixed(&mut data, "");
            data
        }
    }
}

/// Generate the load_state() method body.
///
/// This generates path-based deserialization that supports nested groups.
/// Paths like "filter/cutoff" are split to route to the correct nested group.
fn generate_load_state(ir: &ParamsIR) -> TokenStream {
    // Generate match arms for direct param string IDs (no path prefix)
    let direct_match_arms: Vec<TokenStream> = ir
        .param_fields()
        .map(|param| {
            let field = &param.field_name;
            let id_str = &param.string_id;
            quote! {
                #id_str => {
                    self.#field.set_normalized(value.clamp(0.0, 1.0));
                    true
                }
            }
        })
        .collect();

    // Generate nested group routing - match group name and delegate rest of path
    let nested_routes: Vec<TokenStream> = ir
        .nested_fields()
        .map(|nested| {
            let field = &nested.field_name;
            let group_name = &nested.group_name;
            quote! {
                #group_name => {
                    // Delegate to nested group with remaining path
                    self.#field.load_state_path(rest, value);
                    true
                }
            }
        })
        .collect();

    let nested_routing = if nested_routes.is_empty() {
        quote! { false }
    } else {
        quote! {
            match group {
                #(#nested_routes)*
                _ => false
            }
        }
    };

    let direct_matching = if direct_match_arms.is_empty() {
        quote! { false }
    } else {
        quote! {
            match path {
                #(#direct_match_arms)*
                _ => false
            }
        }
    };

    quote! {
        /// Load a single parameter by its path.
        ///
        /// Called recursively for nested groups. The path is relative to this struct.
        fn load_state_path(&mut self, path: &str, value: f64) -> bool {
            // Check if path contains a group prefix
            if let Some(slash_pos) = path.find('/') {
                let group = &path[..slash_pos];
                let rest = &path[slash_pos + 1..];
                #nested_routing
            } else {
                // Direct parameter match
                #direct_matching
            }
        }

        fn load_state(&mut self, data: &[u8]) -> Result<(), String> {
            if data.is_empty() {
                return Ok(());
            }

            let mut cursor = 0;
            while cursor < data.len() {
                // Read path length
                let path_len = data[cursor] as usize;
                cursor += 1;

                if cursor + path_len + 8 > data.len() {
                    break; // Incomplete data
                }

                // Read path string
                let path = match ::std::str::from_utf8(&data[cursor..cursor + path_len]) {
                    Ok(s) => s,
                    Err(_) => {
                        cursor += path_len + 8;
                        continue; // Skip invalid UTF-8
                    }
                };
                cursor += path_len;

                // Read value
                let value_bytes: [u8; 8] = data[cursor..cursor + 8]
                    .try_into()
                    .map_err(|_| "Invalid state data")?;
                let value = f64::from_le_bytes(value_bytes);
                cursor += 8;

                // Route to correct parameter by path
                self.load_state_path(path, value);
            }

            Ok(())
        }
    }
}

/// Generate the `Parameters` trait implementation.
fn generate_parameters_impl(ir: &ParamsIR) -> TokenStream {
    let struct_name = &ir.struct_name;
    let (impl_generics, ty_generics, where_clause) = ir.generics.split_for_impl();

    let count_impl = generate_count(ir);

    // Generate info() - iterate and return by index
    let info_impl = generate_info(ir);

    // Generate get_normalized - match on ID
    let get_match_arms: Vec<TokenStream> = ir
        .param_fields()
        .map(|param| {
            let field = &param.field_name;
            let const_name = param.const_name();
            quote! {
                #struct_name::#const_name => self.#field.get_normalized(),
            }
        })
        .collect();

    // Generate set_normalized - match on ID
    let set_match_arms: Vec<TokenStream> = ir
        .param_fields()
        .map(|param| {
            let field = &param.field_name;
            let const_name = param.const_name();
            quote! {
                #struct_name::#const_name => self.#field.set_normalized(value),
            }
        })
        .collect();

    quote! {
        impl #impl_generics ::beamer::core::params::Parameters for #struct_name #ty_generics #where_clause {
            fn count(&self) -> usize {
                #count_impl
            }

            #info_impl

            fn get_normalized(&self, id: ::beamer::core::types::ParamId) -> ::beamer::core::types::ParamValue {
                match id {
                    #(#get_match_arms)*
                    _ => {
                        // Check nested or use default
                        use ::beamer::core::param_types::Params;
                        self.by_id(id).map(|p| p.get_normalized()).unwrap_or(0.0)
                    }
                }
            }

            fn set_normalized(&self, id: ::beamer::core::types::ParamId, value: ::beamer::core::types::ParamValue) {
                match id {
                    #(#set_match_arms)*
                    _ => {
                        // Check nested
                        use ::beamer::core::param_types::Params;
                        if let Some(param) = self.by_id(id) {
                            param.set_normalized(value);
                        }
                    }
                }
            }

            fn normalized_to_string(&self, id: ::beamer::core::types::ParamId, normalized: ::beamer::core::types::ParamValue) -> String {
                use ::beamer::core::param_types::Params;
                self.by_id(id).map(|p| p.display_normalized(normalized)).unwrap_or_default()
            }

            fn string_to_normalized(&self, id: ::beamer::core::types::ParamId, string: &str) -> Option<::beamer::core::types::ParamValue> {
                use ::beamer::core::param_types::Params;
                self.by_id(id).and_then(|p| p.parse(string))
            }

            fn normalized_to_plain(&self, id: ::beamer::core::types::ParamId, normalized: ::beamer::core::types::ParamValue) -> ::beamer::core::types::ParamValue {
                use ::beamer::core::param_types::Params;
                self.by_id(id).map(|p| p.normalized_to_plain(normalized)).unwrap_or(0.0)
            }

            fn plain_to_normalized(&self, id: ::beamer::core::types::ParamId, plain: ::beamer::core::types::ParamValue) -> ::beamer::core::types::ParamValue {
                use ::beamer::core::param_types::Params;
                self.by_id(id).map(|p| p.plain_to_normalized(plain)).unwrap_or(0.0)
            }
        }
    }
}

/// Generate the info() method for the Parameters trait.
fn generate_info(ir: &ParamsIR) -> TokenStream {
    // Generate match arms for direct parameters
    let param_match_arms: Vec<TokenStream> = ir
        .param_fields()
        .enumerate()
        .map(|(idx, param)| {
            let field = &param.field_name;
            quote! {
                #idx => Some(self.#field.info()),
            }
        })
        .collect();

    let param_count = ir.param_count();

    // Handle nested params
    if ir.has_nested() {
        let nested_infos: Vec<TokenStream> = ir
            .nested_fields()
            .map(|nested| {
                let field = &nested.field_name;
                // Use fully qualified syntax to disambiguate
                quote! {
                    let nested_count = ::beamer::core::param_types::Params::count(&self.#field);
                    if adjusted_index < nested_count {
                        return ::beamer::core::params::Parameters::info(&self.#field, adjusted_index);
                    }
                    adjusted_index -= nested_count;
                }
            })
            .collect();

        quote! {
            fn info(&self, index: usize) -> Option<&::beamer::core::params::ParamInfo> {
                // First check direct params
                match index {
                    #(#param_match_arms)*
                    _ => {
                        // Adjust index for nested params
                        let mut adjusted_index = index - #param_count;
                        #(#nested_infos)*
                        None
                    }
                }
            }
        }
    } else {
        quote! {
            fn info(&self, index: usize) -> Option<&::beamer::core::params::ParamInfo> {
                match index {
                    #(#param_match_arms)*
                    _ => None,
                }
            }
        }
    }
}

/// Generate the `set_sample_rate()` method for the Params trait.
fn generate_set_sample_rate(ir: &ParamsIR) -> TokenStream {
    // Generate calls for direct param fields
    let param_calls: Vec<TokenStream> = ir
        .param_fields()
        .map(|param| {
            let field = &param.field_name;
            quote! { self.#field.set_sample_rate(sample_rate); }
        })
        .collect();

    // Generate calls for nested fields
    let nested_calls: Vec<TokenStream> = ir
        .nested_fields()
        .map(|nested| {
            let field = &nested.field_name;
            quote! { self.#field.set_sample_rate(sample_rate); }
        })
        .collect();

    if param_calls.is_empty() && nested_calls.is_empty() {
        // No params = use default no-op
        quote! {}
    } else {
        quote! {
            fn set_sample_rate(&mut self, sample_rate: f64) {
                #(#param_calls)*
                #(#nested_calls)*
            }
        }
    }
}

/// Generate the `reset_smoothing()` method for the Params trait.
fn generate_reset_smoothing(ir: &ParamsIR) -> TokenStream {
    // Generate calls for direct param fields
    let param_calls: Vec<TokenStream> = ir
        .param_fields()
        .map(|param| {
            let field = &param.field_name;
            quote! { self.#field.reset_smoothing(); }
        })
        .collect();

    // Generate calls for nested fields
    let nested_calls: Vec<TokenStream> = ir
        .nested_fields()
        .map(|nested| {
            let field = &nested.field_name;
            quote! { self.#field.reset_smoothing(); }
        })
        .collect();

    if param_calls.is_empty() && nested_calls.is_empty() {
        // No params = use default no-op
        quote! {}
    } else {
        quote! {
            fn reset_smoothing(&mut self) {
                #(#param_calls)*
                #(#nested_calls)*
            }
        }
    }
}

// =============================================================================
// Default Implementation Generation
// =============================================================================

/// Generate `Default` impl if all param fields have declarative attributes.
///
/// This is the core of the declarative parameter system. When all parameters
/// have the required attributes (name, default, range, etc.), the macro
/// generates a complete `Default` implementation.
fn generate_default_impl(ir: &ParamsIR) -> TokenStream {
    // Only generate if all params have declarative attributes
    if !ir.can_generate_default() {
        return quote! {};
    }

    let struct_name = &ir.struct_name;
    let (impl_generics, ty_generics, where_clause) = ir.generics.split_for_impl();

    // Generate field initializers for all fields
    let field_inits: Vec<TokenStream> = ir
        .fields
        .iter()
        .map(|field| match field {
            FieldIR::Param(p) => generate_param_initializer(p, struct_name),
            FieldIR::Nested(n) => {
                let field = &n.field_name;
                quote! { #field: Default::default() }
            }
        })
        .collect();

    // Add set_unit_ids() call if there are groups (flat or nested)
    let unit_id_init = if ir.has_nested() || ir.has_flat_groups() {
        quote! {
            params.set_unit_ids();
        }
    } else {
        quote! {}
    };

    quote! {
        impl #impl_generics Default for #struct_name #ty_generics #where_clause {
            fn default() -> Self {
                let mut params = Self {
                    #(#field_inits),*
                };
                #unit_id_init
                params
            }
        }
    }
}

/// Generate the initializer for a single parameter field.
fn generate_param_initializer(param: &ParamFieldIR, struct_name: &syn::Ident) -> TokenStream {
    let field = &param.field_name;

    // Generate constructor call
    let constructor = generate_constructor(param);

    // Generate builder chain (with_id, with_short_name, with_smoother)
    let builder_chain = generate_builder_chain(param, struct_name);

    quote! {
        #field: #constructor #builder_chain
    }
}

/// Generate the constructor call for a parameter.
fn generate_constructor(param: &ParamFieldIR) -> TokenStream {
    match param.param_type {
        crate::ir::ParamType::Float => generate_float_constructor(param),
        crate::ir::ParamType::Int => generate_int_constructor(param),
        crate::ir::ParamType::Bool => generate_bool_constructor(param),
        crate::ir::ParamType::Enum => generate_enum_constructor(param),
    }
}

/// Generate constructor for FloatParam.
fn generate_float_constructor(param: &ParamFieldIR) -> TokenStream {
    let name = param.attrs.name.as_ref().expect("FloatParam requires name");
    let default = match &param.attrs.default {
        Some(ParamDefault::Float(v)) => *v,
        Some(ParamDefault::Int(v)) => *v as f64,
        _ => 0.0,
    };

    // Get kind, defaulting to Linear
    let kind = param.attrs.kind.unwrap_or(ParamKind::Linear);

    // Handle special kinds with fixed ranges
    match kind {
        ParamKind::Percent => {
            return quote! {
                ::beamer::core::param_types::FloatParam::percent(#name, #default)
            };
        }
        ParamKind::Pan => {
            return quote! {
                ::beamer::core::param_types::FloatParam::pan(#name, #default)
            };
        }
        _ => {}
    }

    // Get range (required for non-fixed-range kinds)
    let (start, end) = param
        .attrs
        .range
        .as_ref()
        .map(|r| (r.start, r.end))
        .or_else(|| kind.fixed_range())
        .expect("FloatParam requires range");

    match kind {
        ParamKind::Db => quote! {
            ::beamer::core::param_types::FloatParam::db(#name, #default, #start..=#end)
        },
        ParamKind::DbLog => quote! {
            ::beamer::core::param_types::FloatParam::db_log(#name, #default, #start..=#end)
        },
        ParamKind::DbLogOffset => quote! {
            ::beamer::core::param_types::FloatParam::db_log_offset(#name, #default, #start..=#end)
        },
        ParamKind::Hz => quote! {
            ::beamer::core::param_types::FloatParam::hz(#name, #default, #start..=#end)
        },
        ParamKind::Ms => quote! {
            ::beamer::core::param_types::FloatParam::ms(#name, #default, #start..=#end)
        },
        ParamKind::Seconds => quote! {
            ::beamer::core::param_types::FloatParam::seconds(#name, #default, #start..=#end)
        },
        ParamKind::Ratio => quote! {
            ::beamer::core::param_types::FloatParam::ratio(#name, #default, #start..=#end)
        },
        ParamKind::Linear => quote! {
            ::beamer::core::param_types::FloatParam::new(#name, #default, #start..=#end)
        },
        ParamKind::Semitones => {
            // Semitones is an int kind, shouldn't reach here
            quote! {
                ::beamer::core::param_types::FloatParam::new(#name, #default, #start..=#end)
            }
        }
        // Percent and Pan are handled by early returns above; this is unreachable
        ParamKind::Percent | ParamKind::Pan => unreachable!("handled by early return"),
    }
}

/// Generate constructor for IntParam.
fn generate_int_constructor(param: &ParamFieldIR) -> TokenStream {
    let name = param.attrs.name.as_ref().expect("IntParam requires name");
    let default = match &param.attrs.default {
        Some(ParamDefault::Int(v)) => *v,
        Some(ParamDefault::Float(v)) => *v as i64,
        _ => 0,
    };

    let range = param.attrs.range.as_ref().expect("IntParam requires range");
    let start = range.start as i64;
    let end = range.end as i64;

    // Check for semitones kind
    if param.attrs.kind == Some(ParamKind::Semitones) {
        quote! {
            ::beamer::core::param_types::IntParam::semitones(#name, #default, #start..=#end)
        }
    } else {
        quote! {
            ::beamer::core::param_types::IntParam::new(#name, #default, #start..=#end)
        }
    }
}

/// Generate constructor for BoolParam.
fn generate_bool_constructor(param: &ParamFieldIR) -> TokenStream {
    // Special case: bypass parameter
    if param.attrs.bypass {
        return quote! {
            ::beamer::core::param_types::BoolParam::bypass()
        };
    }

    let name = param.attrs.name.as_ref().expect("BoolParam requires name");
    let default = match &param.attrs.default {
        Some(ParamDefault::Bool(v)) => *v,
        _ => false,
    };

    quote! {
        ::beamer::core::param_types::BoolParam::new(#name, #default)
    }
}

/// Generate constructor for EnumParam.
fn generate_enum_constructor(param: &ParamFieldIR) -> TokenStream {
    let name = param.attrs.name.as_ref().expect("EnumParam requires name");

    quote! {
        ::beamer::core::param_types::EnumParam::new(#name)
    }
}

/// Generate the builder method chain (.with_id(), .with_short_name(), .with_smoother()).
fn generate_builder_chain(param: &ParamFieldIR, struct_name: &syn::Ident) -> TokenStream {
    let const_name = param.const_name();

    // Always add .with_id()
    let with_id = quote! {
        .with_id(#struct_name::#const_name)
    };

    // Optional: .with_short_name()
    let with_short_name = param.attrs.short_name.as_ref().map(|short| {
        quote! { .with_short_name(#short) }
    });

    // Optional: .with_smoother() (only for FloatParam)
    let with_smoother = if param.param_type == crate::ir::ParamType::Float {
        param.attrs.smoothing.as_ref().map(|s| {
            let time_ms = s.time_ms;
            let style = match s.style {
                SmoothingStyle::Exponential => {
                    quote! { ::beamer::core::smoothing::SmoothingStyle::Exponential(#time_ms) }
                }
                SmoothingStyle::Linear => {
                    quote! { ::beamer::core::smoothing::SmoothingStyle::Linear(#time_ms) }
                }
            };
            quote! { .with_smoother(#style) }
        })
    } else {
        None
    };

    quote! {
        #with_id
        #with_short_name
        #with_smoother
    }
}
