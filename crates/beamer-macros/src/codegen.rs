//! Code generation for the derive macro.
//!
//! This module generates the Rust code for the `Params` and `Parameters` trait
//! implementations from the validated IR.

use proc_macro2::TokenStream;
use quote::quote;

use crate::ir::ParamsIR;

/// Generate all code for the derive macro.
pub fn generate(ir: &ParamsIR) -> TokenStream {
    let const_ids = generate_const_ids(ir);
    let unit_consts = generate_unit_consts(ir);
    let collision_check = generate_collision_check(ir);
    let units_impl = generate_units_impl(ir);
    let params_impl = generate_params_impl(ir);
    let parameters_impl = generate_parameters_impl(ir);
    let set_unit_ids_impl = generate_set_unit_ids(ir);

    quote! {
        #const_ids
        #unit_consts
        #collision_check
        #units_impl
        #params_impl
        #parameters_impl
        #set_unit_ids_impl
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

/// Generate unit ID constants for each nested field.
fn generate_unit_consts(ir: &ParamsIR) -> TokenStream {
    let struct_name = &ir.struct_name;

    let unit_consts: Vec<TokenStream> = ir
        .nested_fields()
        .map(|nested| {
            let const_name = syn::Ident::new(
                &format!("UNIT_{}", nested.field_name.to_string().to_uppercase()),
                nested.span,
            );
            let unit_id = nested.unit_id;
            quote! {
                /// Unit ID for the nested parameter group.
                pub const #const_name: ::beamer::core::params::UnitId = #unit_id;
            }
        })
        .collect();

    if unit_consts.is_empty() {
        quote! {}
    } else {
        quote! {
            impl #struct_name {
                #(#unit_consts)*
            }
        }
    }
}

/// Generate the `Units` trait implementation.
///
/// For structs with nested groups, this generates dynamic unit discovery
/// that recursively collects all units including deeply nested ones.
fn generate_units_impl(ir: &ParamsIR) -> TokenStream {
    let struct_name = &ir.struct_name;
    let (impl_generics, ty_generics, where_clause) = ir.generics.split_for_impl();

    if !ir.has_nested() {
        // No nested fields = use default Units impl (root only)
        return quote! {
            impl #impl_generics ::beamer::core::params::Units for #struct_name #ty_generics #where_clause {}
        };
    }

    // For structs with nested groups, use dynamic unit collection
    // This properly handles deeply nested groups with correct parent IDs
    quote! {
        impl #impl_generics ::beamer::core::params::Units for #struct_name #ty_generics #where_clause {
            fn unit_count(&self) -> usize {
                use ::beamer::core::param_types::Params;
                // Count = 1 (root) + all nested units recursively
                let mut units = Vec::new();
                self.collect_units(&mut units, 1, 0);
                1 + units.len()
            }

            fn unit_info(&self, index: usize) -> Option<::beamer::core::params::UnitInfo> {
                use ::beamer::core::param_types::Params;
                if index == 0 {
                    return Some(::beamer::core::params::UnitInfo::root());
                }

                // Collect all units dynamically
                let mut units = Vec::new();
                self.collect_units(&mut units, 1, 0);

                // Return the requested unit (index-1 because index 0 is root)
                units.get(index - 1).cloned()
            }
        }
    }
}

/// Generate the `set_unit_ids()` method for initializing nested param unit IDs.
///
/// This uses the recursive `assign_unit_ids` method from the `Params` trait
/// to properly assign unit IDs to deeply nested groups with correct parent
/// relationships.
fn generate_set_unit_ids(ir: &ParamsIR) -> TokenStream {
    let struct_name = &ir.struct_name;
    let (impl_generics, ty_generics, where_clause) = ir.generics.split_for_impl();

    if !ir.has_nested() {
        // No nested fields = no-op set_unit_ids
        return quote! {
            impl #impl_generics #struct_name #ty_generics #where_clause {
                /// Initialize unit IDs for nested parameters.
                ///
                /// No nested parameters in this struct, so this is a no-op.
                pub fn set_unit_ids(&mut self) {}
            }
        };
    }

    // Use the recursive assign_unit_ids method from the Params trait
    // This properly handles deeply nested groups with correct parent IDs
    quote! {
        impl #impl_generics #struct_name #ty_generics #where_clause {
            /// Initialize unit IDs for all nested parameters.
            ///
            /// This method recursively assigns unit IDs to all nested parameter
            /// groups, including deeply nested ones. Unit IDs are assigned
            /// sequentially starting from 1 (0 is reserved for root).
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
                use ::beamer::core::param_types::Params;
                // Start from unit ID 1 (0 is root), with parent ID 0 (root)
                self.assign_unit_ids(1, 0);
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
