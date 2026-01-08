//! Code generation for the derive macro.
//!
//! This module generates the Rust code for the `Parameters` and `ParameterStore` trait
//! implementations from the validated IR.

use proc_macro2::TokenStream;
use quote::quote;

use crate::ir::{
    FieldIR, ParameterDefault, ParameterFieldIR, ParameterKind, ParametersIR, SmoothingStyle,
};

/// Generate all code for the derive macro.
pub fn generate(ir: &ParametersIR) -> TokenStream {
    let const_ids = generate_const_ids(ir);
    let unit_consts = generate_group_consts(ir);
    let collision_check = generate_collision_check(ir);
    let units_impl = generate_groups_impl(ir);
    let vst3_parameters_impl = generate_parameter_store_impl(ir);
    let parameters_impl = generate_parameters_impl(ir);
    let set_group_ids_impl = generate_set_group_ids(ir);
    let default_impl = generate_default_impl(ir);

    quote! {
        #const_ids
        #unit_consts
        #collision_check
        #units_impl
        #vst3_parameters_impl
        #parameters_impl
        #set_group_ids_impl
        #default_impl
    }
}

/// Generate const ID declarations for each parameter.
fn generate_const_ids(ir: &ParametersIR) -> TokenStream {
    let struct_name = &ir.struct_name;

    let const_defs: Vec<TokenStream> = ir
        .parameter_fields()
        .map(|parameter| {
            let const_name = parameter.const_name();
            let hash = parameter.hash_id;
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

/// Generate group ID constants for each nested field and flat group.
fn generate_group_consts(ir: &ParametersIR) -> TokenStream {
    let struct_name = &ir.struct_name;

    // Generate constants for flat groups first (they get IDs 1, 2, 3, ...)
    let flat_groups = ir.flat_group_names();
    let flat_group_consts: Vec<TokenStream> = flat_groups
        .iter()
        .enumerate()
        .map(|(idx, group_name)| {
            // Convert group name to uppercase const name (e.g., "Filter" -> "PARAM_GROUP_FILTER")
            let const_name = syn::Ident::new(
                &format!("PARAM_GROUP_{}", group_name.to_uppercase().replace(' ', "_")),
                proc_macro2::Span::call_site(),
            );
            let group_id = (idx + 1) as i32; // Start at 1 (0 is root)
            quote! {
                /// Group ID for the flat parameter group.
                pub const #const_name: ::beamer::core::parameter_groups::GroupId = #group_id;
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
                &format!("GROUP_{}", nested.field_name.to_string().to_uppercase()),
                nested.span,
            );
            let group_id = flat_group_count + (idx as i32) + 1;
            quote! {
                /// Group ID for the nested parameter group.
                pub const #const_name: ::beamer::core::parameter_groups::GroupId = #group_id;
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

/// Generate the `ParameterGroups` trait implementation.
///
/// For structs with nested groups or flat groups, this generates unit discovery
/// that includes both flat groups and deeply nested ones.
fn generate_groups_impl(ir: &ParametersIR) -> TokenStream {
    let struct_name = &ir.struct_name;
    let (impl_generics, ty_generics, where_clause) = ir.generics.split_for_impl();

    let flat_groups = ir.flat_group_names();
    let has_flat_groups = !flat_groups.is_empty();
    let has_nested = ir.has_nested();

    if !has_flat_groups && !has_nested {
        // No groups at all = use default ParameterGroups impl (root only)
        return quote! {
            impl #impl_generics ::beamer::core::parameter_groups::ParameterGroups for #struct_name #ty_generics #where_clause {}
        };
    }

    // Generate flat group GroupInfo entries
    let flat_group_count = flat_groups.len();
    let flat_group_infos: Vec<TokenStream> = flat_groups
        .iter()
        .enumerate()
        .map(|(idx, group_name)| {
            let group_id = (idx + 1) as i32;
            quote! {
                #idx => Some(::beamer::core::parameter_groups::GroupInfo {
                    id: #group_id,
                    name: #group_name,
                    parent_id: 0, // All flat groups are children of root
                }),
            }
        })
        .collect();

    if has_nested {
        // Flat groups + nested groups: combine static flat groups with dynamic nested collection
        quote! {
            impl #impl_generics ::beamer::core::parameter_groups::ParameterGroups for #struct_name #ty_generics #where_clause {
                fn group_count(&self) -> usize {
                    use ::beamer::core::parameter_types::Parameters;
                    // Count = 1 (root) + flat groups + nested units recursively
                    let flat_count = #flat_group_count;
                    let mut units = Vec::new();
                    self.collect_groups(&mut units, (flat_count + 1) as i32, 0);
                    1 + flat_count + units.len()
                }

                fn group_info(&self, index: usize) -> Option<::beamer::core::parameter_groups::GroupInfo> {
                    use ::beamer::core::parameter_types::Parameters;
                    if index == 0 {
                        return Some(::beamer::core::parameter_groups::GroupInfo::root());
                    }

                    // Check flat groups first (indices 1..=flat_group_count)
                    let flat_idx = index - 1;
                    if flat_idx < #flat_group_count {
                        return match flat_idx {
                            #(#flat_group_infos)*
                            _ => None,
                        };
                    }

                    // Then check nested groups
                    let mut units = Vec::new();
                    self.collect_groups(&mut units, (#flat_group_count + 1) as i32, 0);
                    let nested_idx = index - 1 - #flat_group_count;
                    units.get(nested_idx).cloned()
                }
            }
        }
    } else {
        // Only flat groups, no nesting
        quote! {
            impl #impl_generics ::beamer::core::parameter_groups::ParameterGroups for #struct_name #ty_generics #where_clause {
                fn group_count(&self) -> usize {
                    1 + #flat_group_count // root + flat groups
                }

                fn group_info(&self, index: usize) -> Option<::beamer::core::parameter_groups::GroupInfo> {
                    if index == 0 {
                        return Some(::beamer::core::parameter_groups::GroupInfo::root());
                    }

                    let flat_idx = index - 1;
                    match flat_idx {
                        #(#flat_group_infos)*
                        _ => None,
                    }
                }
            }
        }
    }
}

/// Generate the `set_group_ids()` method for initializing parameter group IDs.
///
/// This handles both flat groups (group="...") and nested groups (#[nested(...)]).
/// For flat groups, it sets group_id directly on the parameters based on their group.
/// For nested groups, it uses the recursive `assign_group_ids` method.
fn generate_set_group_ids(ir: &ParametersIR) -> TokenStream {
    let struct_name = &ir.struct_name;
    let (impl_generics, ty_generics, where_clause) = ir.generics.split_for_impl();

    let flat_groups = ir.flat_group_names();
    let has_flat_groups = !flat_groups.is_empty();
    let has_nested = ir.has_nested();

    if !has_flat_groups && !has_nested {
        // No groups at all = no-op set_group_ids
        return quote! {
            impl #impl_generics #struct_name #ty_generics #where_clause {
                /// Initialize group IDs for parameters.
                ///
                /// No groups in this struct, so this is a no-op.
                pub fn set_group_ids(&mut self) {}
            }
        };
    }

    // Build a map of group name -> group ID
    let group_to_group_id: std::collections::HashMap<&str, i32> = flat_groups
        .iter()
        .enumerate()
        .map(|(idx, name)| (*name, (idx + 1) as i32))
        .collect();

    // Generate statements to set group_id on parameters with flat groups
    let flat_group_assignments: Vec<TokenStream> = ir
        .parameter_fields()
        .filter_map(|parameter| {
            parameter.attributes.group.as_ref().map(|group_name| {
                let field = &parameter.field_name;
                let group_id = group_to_group_id.get(group_name.as_str()).copied().unwrap_or(0);
                quote! {
                    self.#field.set_group_id(#group_id);
                }
            })
        })
        .collect();

    let flat_group_count = flat_groups.len() as i32;
    let nested_init = if has_nested {
        quote! {
            use ::beamer::core::parameter_types::Parameters;
            // Nested groups start after flat groups
            self.assign_group_ids(#flat_group_count + 1, 0);
        }
    } else {
        quote! {}
    };

    quote! {
        impl #impl_generics #struct_name #ty_generics #where_clause {
            /// Initialize group IDs for all parameters.
            ///
            /// This method assigns group IDs to parameters with `group` attributes
            /// and recursively assigns group IDs to nested parameter groups.
            /// Group IDs are assigned sequentially starting from 1 (0 is reserved for root).
            ///
            /// Call this once after construction to set up the group hierarchy.
            ///
            /// # Example
            ///
            /// ```ignore
            /// let mut parameters = SynthParameters::default();
            /// parameters.set_group_ids();
            /// ```
            pub fn set_group_ids(&mut self) {
                // Set group IDs for flat groups
                #(#flat_group_assignments)*
                // Initialize nested groups
                #nested_init
            }
        }
    }
}

/// Generate compile-time collision detection.
fn generate_collision_check(ir: &ParametersIR) -> TokenStream {
    let parameter_fields: Vec<_> = ir.parameter_fields().collect();

    if parameter_fields.len() < 2 {
        // No collision possible with 0 or 1 parameters
        return quote! {};
    }

    let id_pairs: Vec<TokenStream> = parameter_fields
        .iter()
        .map(|parameter| {
            let id_str = &parameter.string_id;
            let hash = parameter.hash_id;
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

/// Generate the `Parameters` trait implementation.
fn generate_parameters_impl(ir: &ParametersIR) -> TokenStream {
    let struct_name = &ir.struct_name;
    let (impl_generics, ty_generics, where_clause) = ir.generics.split_for_impl();

    let count_impl = generate_count(ir);
    let iter_impl = generate_iter(ir);
    let by_id_impl = generate_by_id(ir);
    let save_state_impl = generate_save_state(ir);
    let load_state_impl = generate_load_state(ir);
    let set_all_group_ids_impl = generate_set_all_group_ids(ir);
    let nested_discovery_impl = generate_nested_discovery(ir);
    let set_sample_rate_impl = generate_set_sample_rate(ir);
    let reset_smoothing_impl = generate_reset_smoothing(ir);

    quote! {
        impl #impl_generics ::beamer::core::parameter_types::Parameters for #struct_name #ty_generics #where_clause {
            fn count(&self) -> usize {
                #count_impl
            }

            fn iter(&self) -> Box<dyn Iterator<Item = &dyn ::beamer::core::parameter_types::ParameterRef> + '_> {
                #iter_impl
            }

            fn by_id(&self, id: ::beamer::core::types::ParameterId) -> Option<&dyn ::beamer::core::parameter_types::ParameterRef> {
                #by_id_impl
            }

            fn by_id_mut(&mut self, id: ::beamer::core::types::ParameterId) -> Option<&dyn ::beamer::core::parameter_types::ParameterRef> {
                self.by_id(id)
            }

            #set_all_group_ids_impl

            #nested_discovery_impl

            #save_state_impl

            #load_state_impl

            #set_sample_rate_impl

            #reset_smoothing_impl
        }
    }
}

/// Generate the `set_all_group_ids()` method for the Parameters trait.
fn generate_set_all_group_ids(ir: &ParametersIR) -> TokenStream {
    if ir.parameter_count() == 0 {
        // No direct parameters = use default no-op
        return quote! {};
    }

    // Generate statements to set group_id on each direct parameter field
    let assignments: Vec<TokenStream> = ir
        .parameter_fields()
        .map(|parameter| {
            let field = &parameter.field_name;
            quote! {
                self.#field.set_group_id(group_id);
            }
        })
        .collect();

    quote! {
        fn set_all_group_ids(&mut self, group_id: ::beamer::core::parameter_groups::GroupId) {
            #(#assignments)*
        }
    }
}

/// Generate the nested group discovery methods for the Parameters trait.
fn generate_nested_discovery(ir: &ParametersIR) -> TokenStream {
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
                #idx => Some((#name, &self.#field as &dyn ::beamer::core::parameter_types::Parameters)),
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
                #idx => Some((#name, &mut self.#field as &mut dyn ::beamer::core::parameter_types::Parameters)),
            }
        })
        .collect();

    quote! {
        fn nested_count(&self) -> usize {
            #nested_count
        }

        fn nested_group(&self, index: usize) -> Option<(&'static str, &dyn ::beamer::core::parameter_types::Parameters)> {
            match index {
                #(#group_match_arms)*
                _ => None,
            }
        }

        fn nested_group_mut(&mut self, index: usize) -> Option<(&'static str, &mut dyn ::beamer::core::parameter_types::Parameters)> {
            match index {
                #(#group_mut_match_arms)*
                _ => None,
            }
        }
    }
}

/// Generate the count() method body.
fn generate_count(ir: &ParametersIR) -> TokenStream {
    let parameter_count = ir.parameter_count();

    if ir.has_nested() {
        let nested_counts: Vec<TokenStream> = ir
            .nested_fields()
            .map(|nested| {
                let field = &nested.field_name;
                // Use fully qualified syntax to disambiguate between Parameters::count and Parameters::count
                quote! { ::beamer::core::parameter_types::Parameters::count(&self.#field) }
            })
            .collect();

        quote! {
            #parameter_count #(+ #nested_counts)*
        }
    } else {
        quote! { #parameter_count }
    }
}

/// Generate the iter() method body.
fn generate_iter(ir: &ParametersIR) -> TokenStream {
    let parameter_iters: Vec<TokenStream> = ir
        .parameter_fields()
        .map(|parameter| {
            let field = &parameter.field_name;
            quote! { &self.#field as &dyn ::beamer::core::parameter_types::ParameterRef }
        })
        .collect();

    let nested_chains: Vec<TokenStream> = ir
        .nested_fields()
        .map(|nested| {
            let field = &nested.field_name;
            quote! { .chain(self.#field.iter()) }
        })
        .collect();

    if parameter_iters.is_empty() && nested_chains.is_empty() {
        quote! { Box::new(::std::iter::empty()) }
    } else if parameter_iters.is_empty() {
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
                [#(#parameter_iters),*].into_iter()
                    #(#nested_chains)*
            )
        }
    }
}

/// Generate the by_id() method body.
fn generate_by_id(ir: &ParametersIR) -> TokenStream {
    let struct_name = &ir.struct_name;

    let match_arms: Vec<TokenStream> = ir
        .parameter_fields()
        .map(|parameter| {
            let field = &parameter.field_name;
            let const_name = parameter.const_name();
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
                if let Some(parameter) = self.#field.by_id(id) {
                    return Some(parameter);
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
fn generate_save_state(ir: &ParametersIR) -> TokenStream {
    // Generate saves for direct parameters using string IDs with prefix
    let parameter_saves: Vec<TokenStream> = ir
        .parameter_fields()
        .map(|parameter| {
            let field = &parameter.field_name;
            let id_str = &parameter.string_id;
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

    let parameter_count = ir.parameter_count();
    // Estimate capacity: ~20 bytes per parameter (path_len + avg 10 char path + 8 byte f64)
    let estimated_capacity = parameter_count * 20;

    quote! {
        fn save_state_prefixed(&self, data: &mut Vec<u8>, prefix: &str) {
            #(#parameter_saves)*
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
fn generate_load_state(ir: &ParametersIR) -> TokenStream {
    // Generate match arms for direct parameter string IDs (no path prefix)
    let direct_match_arms: Vec<TokenStream> = ir
        .parameter_fields()
        .map(|parameter| {
            let field = &parameter.field_name;
            let id_str = &parameter.string_id;
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

/// Generate the `ParameterStore` trait implementation.
fn generate_parameter_store_impl(ir: &ParametersIR) -> TokenStream {
    let struct_name = &ir.struct_name;
    let (impl_generics, ty_generics, where_clause) = ir.generics.split_for_impl();

    let count_impl = generate_count(ir);

    // Generate info() - iterate and return by index
    let info_impl = generate_info(ir);

    // Generate get_normalized - match on ID
    let get_match_arms: Vec<TokenStream> = ir
        .parameter_fields()
        .map(|parameter| {
            let field = &parameter.field_name;
            let const_name = parameter.const_name();
            quote! {
                #struct_name::#const_name => self.#field.get_normalized(),
            }
        })
        .collect();

    // Generate set_normalized - match on ID
    let set_match_arms: Vec<TokenStream> = ir
        .parameter_fields()
        .map(|parameter| {
            let field = &parameter.field_name;
            let const_name = parameter.const_name();
            quote! {
                #struct_name::#const_name => self.#field.set_normalized(value),
            }
        })
        .collect();

    quote! {
        impl #impl_generics ::beamer::core::parameter_store::ParameterStore for #struct_name #ty_generics #where_clause {
            fn count(&self) -> usize {
                #count_impl
            }

            #info_impl

            fn get_normalized(&self, id: ::beamer::core::types::ParameterId) -> ::beamer::core::types::ParameterValue {
                match id {
                    #(#get_match_arms)*
                    _ => {
                        // Check nested or use default
                        use ::beamer::core::parameter_types::Parameters;
                        self.by_id(id).map(|p| p.get_normalized()).unwrap_or(0.0)
                    }
                }
            }

            fn set_normalized(&self, id: ::beamer::core::types::ParameterId, value: ::beamer::core::types::ParameterValue) {
                match id {
                    #(#set_match_arms)*
                    _ => {
                        // Check nested
                        use ::beamer::core::parameter_types::Parameters;
                        if let Some(parameter) = self.by_id(id) {
                            parameter.set_normalized(value);
                        }
                    }
                }
            }

            fn normalized_to_string(&self, id: ::beamer::core::types::ParameterId, normalized: ::beamer::core::types::ParameterValue) -> String {
                use ::beamer::core::parameter_types::Parameters;
                self.by_id(id).map(|p| p.display_normalized(normalized)).unwrap_or_default()
            }

            fn string_to_normalized(&self, id: ::beamer::core::types::ParameterId, string: &str) -> Option<::beamer::core::types::ParameterValue> {
                use ::beamer::core::parameter_types::Parameters;
                self.by_id(id).and_then(|p| p.parse(string))
            }

            fn normalized_to_plain(&self, id: ::beamer::core::types::ParameterId, normalized: ::beamer::core::types::ParameterValue) -> ::beamer::core::types::ParameterValue {
                use ::beamer::core::parameter_types::Parameters;
                self.by_id(id).map(|p| p.normalized_to_plain(normalized)).unwrap_or(0.0)
            }

            fn plain_to_normalized(&self, id: ::beamer::core::types::ParameterId, plain: ::beamer::core::types::ParameterValue) -> ::beamer::core::types::ParameterValue {
                use ::beamer::core::parameter_types::Parameters;
                self.by_id(id).map(|p| p.plain_to_normalized(plain)).unwrap_or(0.0)
            }
        }
    }
}

/// Generate the info() method for the Parameters trait.
fn generate_info(ir: &ParametersIR) -> TokenStream {
    // Generate match arms for direct parameters
    let parameter_match_arms: Vec<TokenStream> = ir
        .parameter_fields()
        .enumerate()
        .map(|(idx, parameter)| {
            let field = &parameter.field_name;
            quote! {
                #idx => Some(self.#field.info()),
            }
        })
        .collect();

    let parameter_count = ir.parameter_count();

    // Handle nested parameters
    if ir.has_nested() {
        let nested_infos: Vec<TokenStream> = ir
            .nested_fields()
            .map(|nested| {
                let field = &nested.field_name;
                // Use fully qualified syntax to disambiguate
                quote! {
                    let nested_count = ::beamer::core::parameter_types::Parameters::count(&self.#field);
                    if adjusted_index < nested_count {
                        return ::beamer::core::parameter_store::ParameterStore::info(&self.#field, adjusted_index);
                    }
                    adjusted_index -= nested_count;
                }
            })
            .collect();

        quote! {
            fn info(&self, index: usize) -> Option<&::beamer::core::parameter_info::ParameterInfo> {
                // First check direct parameters
                match index {
                    #(#parameter_match_arms)*
                    _ => {
                        // Adjust index for nested parameters
                        let mut adjusted_index = index - #parameter_count;
                        #(#nested_infos)*
                        None
                    }
                }
            }
        }
    } else {
        quote! {
            fn info(&self, index: usize) -> Option<&::beamer::core::parameter_info::ParameterInfo> {
                match index {
                    #(#parameter_match_arms)*
                    _ => None,
                }
            }
        }
    }
}

/// Generate the `set_sample_rate()` method for the Parameters trait.
fn generate_set_sample_rate(ir: &ParametersIR) -> TokenStream {
    // Generate calls for direct parameter fields
    let parameter_calls: Vec<TokenStream> = ir
        .parameter_fields()
        .map(|parameter| {
            let field = &parameter.field_name;
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

    if parameter_calls.is_empty() && nested_calls.is_empty() {
        // No parameters = use default no-op
        quote! {}
    } else {
        quote! {
            fn set_sample_rate(&mut self, sample_rate: f64) {
                #(#parameter_calls)*
                #(#nested_calls)*
            }
        }
    }
}

/// Generate the `reset_smoothing()` method for the Parameters trait.
fn generate_reset_smoothing(ir: &ParametersIR) -> TokenStream {
    // Generate calls for direct parameter fields
    let parameter_calls: Vec<TokenStream> = ir
        .parameter_fields()
        .map(|parameter| {
            let field = &parameter.field_name;
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

    if parameter_calls.is_empty() && nested_calls.is_empty() {
        // No parameters = use default no-op
        quote! {}
    } else {
        quote! {
            fn reset_smoothing(&mut self) {
                #(#parameter_calls)*
                #(#nested_calls)*
            }
        }
    }
}

// =============================================================================
// Default Implementation Generation
// =============================================================================

/// Generate `Default` impl if all parameter fields have declarative attributes.
///
/// This is the core of the declarative parameter system. When all parameters
/// have the required attributes (name, default, range, etc.), the macro
/// generates a complete `Default` implementation.
fn generate_default_impl(ir: &ParametersIR) -> TokenStream {
    // Only generate if all parameters have declarative attributes
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
            FieldIR::Parameter(p) => generate_parameter_initializer(p, struct_name),
            FieldIR::Nested(n) => {
                let field = &n.field_name;
                quote! { #field: Default::default() }
            }
        })
        .collect();

    // Add set_group_ids() call if there are groups (flat or nested)
    let group_id_init = if ir.has_nested() || ir.has_flat_groups() {
        quote! {
            parameters.set_group_ids();
        }
    } else {
        quote! {}
    };

    quote! {
        impl #impl_generics Default for #struct_name #ty_generics #where_clause {
            fn default() -> Self {
                let mut parameters = Self {
                    #(#field_inits),*
                };
                #group_id_init
                parameters
            }
        }
    }
}

/// Generate the initializer for a single parameter field.
fn generate_parameter_initializer(parameter: &ParameterFieldIR, struct_name: &syn::Ident) -> TokenStream {
    let field = &parameter.field_name;

    // Generate constructor call
    let constructor = generate_constructor(parameter);

    // Generate builder chain (with_id, with_short_name, with_smoother)
    let builder_chain = generate_builder_chain(parameter, struct_name);

    quote! {
        #field: #constructor #builder_chain
    }
}

/// Generate the constructor call for a parameter.
fn generate_constructor(parameter: &ParameterFieldIR) -> TokenStream {
    match parameter.parameter_type {
        crate::ir::ParameterType::Float => generate_float_constructor(parameter),
        crate::ir::ParameterType::Int => generate_int_constructor(parameter),
        crate::ir::ParameterType::Bool => generate_bool_constructor(parameter),
        crate::ir::ParameterType::Enum => generate_enum_constructor(parameter),
    }
}

/// Generate constructor for FloatParameter.
fn generate_float_constructor(parameter: &ParameterFieldIR) -> TokenStream {
    let name = parameter.attributes.name.as_ref().expect("FloatParameter requires name");
    let default = match &parameter.attributes.default {
        Some(ParameterDefault::Float(v)) => *v,
        Some(ParameterDefault::Int(v)) => *v as f64,
        _ => 0.0,
    };

    // Get kind, defaulting to Linear
    let kind = parameter.attributes.kind.unwrap_or(ParameterKind::Linear);

    // Handle special kinds with fixed ranges
    match kind {
        ParameterKind::Percent => {
            return quote! {
                ::beamer::core::parameter_types::FloatParameter::percent(#name, #default)
            };
        }
        ParameterKind::Pan => {
            return quote! {
                ::beamer::core::parameter_types::FloatParameter::pan(#name, #default)
            };
        }
        _ => {}
    }

    // Get range (required for non-fixed-range kinds)
    let (start, end) = parameter
        .attributes
        .range
        .as_ref()
        .map(|r| (r.start, r.end))
        .or_else(|| kind.fixed_range())
        .expect("FloatParameter requires range");

    match kind {
        ParameterKind::Db => quote! {
            ::beamer::core::parameter_types::FloatParameter::db(#name, #default, #start..=#end)
        },
        ParameterKind::DbLog => quote! {
            ::beamer::core::parameter_types::FloatParameter::db_log(#name, #default, #start..=#end)
        },
        ParameterKind::DbLogOffset => quote! {
            ::beamer::core::parameter_types::FloatParameter::db_log_offset(#name, #default, #start..=#end)
        },
        ParameterKind::Hz => quote! {
            ::beamer::core::parameter_types::FloatParameter::hz(#name, #default, #start..=#end)
        },
        ParameterKind::Ms => quote! {
            ::beamer::core::parameter_types::FloatParameter::ms(#name, #default, #start..=#end)
        },
        ParameterKind::Seconds => quote! {
            ::beamer::core::parameter_types::FloatParameter::seconds(#name, #default, #start..=#end)
        },
        ParameterKind::Ratio => quote! {
            ::beamer::core::parameter_types::FloatParameter::ratio(#name, #default, #start..=#end)
        },
        ParameterKind::Linear => quote! {
            ::beamer::core::parameter_types::FloatParameter::new(#name, #default, #start..=#end)
        },
        ParameterKind::Semitones => {
            // Semitones is an int kind, shouldn't reach here
            quote! {
                ::beamer::core::parameter_types::FloatParameter::new(#name, #default, #start..=#end)
            }
        }
        // Percent and Pan are handled by early returns above; this is unreachable
        ParameterKind::Percent | ParameterKind::Pan => unreachable!("handled by early return"),
    }
}

/// Generate constructor for IntParameter.
fn generate_int_constructor(parameter: &ParameterFieldIR) -> TokenStream {
    let name = parameter.attributes.name.as_ref().expect("IntParameter requires name");
    let default = match &parameter.attributes.default {
        Some(ParameterDefault::Int(v)) => *v,
        Some(ParameterDefault::Float(v)) => *v as i64,
        _ => 0,
    };

    let range = parameter.attributes.range.as_ref().expect("IntParameter requires range");
    let start = range.start as i64;
    let end = range.end as i64;

    // Check for semitones kind
    if parameter.attributes.kind == Some(ParameterKind::Semitones) {
        quote! {
            ::beamer::core::parameter_types::IntParameter::semitones(#name, #default, #start..=#end)
        }
    } else {
        quote! {
            ::beamer::core::parameter_types::IntParameter::new(#name, #default, #start..=#end)
        }
    }
}

/// Generate constructor for BoolParameter.
fn generate_bool_constructor(parameter: &ParameterFieldIR) -> TokenStream {
    // Special case: bypass parameter
    if parameter.attributes.bypass {
        return quote! {
            ::beamer::core::parameter_types::BoolParameter::bypass()
        };
    }

    let name = parameter.attributes.name.as_ref().expect("BoolParameter requires name");
    let default = match &parameter.attributes.default {
        Some(ParameterDefault::Bool(v)) => *v,
        _ => false,
    };

    quote! {
        ::beamer::core::parameter_types::BoolParameter::new(#name, #default)
    }
}

/// Generate constructor for EnumParameter.
fn generate_enum_constructor(parameter: &ParameterFieldIR) -> TokenStream {
    let name = parameter.attributes.name.as_ref().expect("EnumParameter requires name");

    quote! {
        ::beamer::core::parameter_types::EnumParameter::new(#name)
    }
}

/// Generate the builder method chain (.with_id(), .with_short_name(), .with_smoother()).
fn generate_builder_chain(parameter: &ParameterFieldIR, struct_name: &syn::Ident) -> TokenStream {
    let const_name = parameter.const_name();

    // Always add .with_id()
    let with_id = quote! {
        .with_id(#struct_name::#const_name)
    };

    // Optional: .with_short_name()
    let with_short_name = parameter.attributes.short_name.as_ref().map(|short| {
        quote! { .with_short_name(#short) }
    });

    // Optional: .with_smoother() (only for FloatParameter)
    let with_smoother = if parameter.parameter_type == crate::ir::ParameterType::Float {
        parameter.attributes.smoothing.as_ref().map(|s| {
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
