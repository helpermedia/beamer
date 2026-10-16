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
    let unit_consts = generate_group_consts(ir);
    let units_impl = generate_groups_impl(ir);
    let vst3_parameters_impl = generate_parameter_store_impl(ir);
    let parameters_impl = generate_parameters_impl(ir);
    let set_group_ids_impl = generate_set_group_ids(ir);
    let default_impl = generate_default_impl(ir);

    quote! {
        #unit_consts
        #units_impl
        #vst3_parameters_impl
        #parameters_impl
        #set_group_ids_impl
        #default_impl
    }
}

/// Generate group ID constants for each nested field and flat group.
fn generate_group_consts(ir: &ParametersIR) -> TokenStream {
    let struct_name = &ir.struct_name;
    let (impl_generics, ty_generics, where_clause) = ir.generics.split_for_impl();

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
                &format!(
                    "GROUP_{}",
                    syn::ext::IdentExt::unraw(&nested.field_name).to_string().to_uppercase()
                ),
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
            impl #impl_generics #struct_name #ty_generics #where_clause {
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

/// Generate the `Parameters` trait implementation.
fn generate_parameters_impl(ir: &ParametersIR) -> TokenStream {
    let struct_name = &ir.struct_name;
    let (impl_generics, ty_generics, where_clause) = ir.generics.split_for_impl();

    let count_impl = generate_count(ir);
    let iter_impl = generate_iter(ir);
    let by_id_impl = generate_by_id(ir);
    let assign_ids_impl = generate_assign_ids(ir);
    let save_state_impl = generate_save_state(ir);
    let load_state_impl = generate_load_state(ir);
    let set_all_group_ids_impl = generate_set_all_group_ids(ir);
    let nested_discovery_impl = generate_nested_discovery(ir);
    let set_sample_rate_impl = generate_set_sample_rate(ir);
    let reset_smoothing_impl = generate_reset_smoothing(ir);
    let webview_events_impl = generate_webview_events(ir);

    quote! {
        impl #impl_generics ::beamer::core::parameter_types::Parameters for #struct_name #ty_generics #where_clause {
            #webview_events_impl

            fn count(&self) -> usize {
                #count_impl
            }

            fn iter(&self) -> Box<dyn Iterator<Item = &dyn ::beamer::core::parameter_types::ParameterRef> + '_> {
                #iter_impl
            }

            fn by_id(&self, id: ::beamer::core::types::ParameterId) -> Option<&dyn ::beamer::core::parameter_types::ParameterRef> {
                #by_id_impl
            }

            #assign_ids_impl

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
///
/// The ids are compared at run time: a parameter of a nested struct has an
/// id that depends on where the struct sits.
fn generate_by_id(ir: &ParametersIR) -> TokenStream {
    let parameter_lookups: Vec<TokenStream> = ir
        .parameter_fields()
        .map(|parameter| {
            let field = &parameter.field_name;
            quote! {
                if ::beamer::core::parameter_types::ParameterRef::id(&self.#field) == id {
                    return Some(&self.#field);
                }
            }
        })
        .collect();

    let nested_lookups: Vec<TokenStream> = ir
        .nested_fields()
        .map(|nested| {
            let field = &nested.field_name;
            quote! {
                if let Some(parameter) =
                    ::beamer::core::parameter_types::Parameters::by_id(&self.#field, id)
                {
                    return Some(parameter);
                }
            }
        })
        .collect();

    quote! {
        #(#parameter_lookups)*
        #(#nested_lookups)*
        None
    }
}

/// Generate `assign_ids()`: every parameter gets its path as string id and
/// the hash of the path as the id that hosts see.
///
/// The path segments are the same as in `save_entries()`: the `id` of a
/// parameter and the field name of a nested struct.
fn generate_assign_ids(ir: &ParametersIR) -> TokenStream {
    let parameter_assignments: Vec<TokenStream> = ir
        .parameter_fields()
        .map(|parameter| {
            let field = &parameter.field_name;
            let id_str = &parameter.string_id;
            quote! {
                self.#field.set_path(
                    ::beamer::core::parameter_types::parameter_path(prefix, #id_str),
                );
            }
        })
        .collect();

    let nested_assignments: Vec<TokenStream> = ir
        .nested_fields()
        .map(|nested| {
            let field = &nested.field_name;
            let segment = syn::ext::IdentExt::unraw(&nested.field_name).to_string();
            quote! {
                ::beamer::core::parameter_types::Parameters::assign_ids(
                    &mut self.#field,
                    &::beamer::core::state_format::join_path(prefix, #segment),
                );
            }
        })
        .collect();

    if parameter_assignments.is_empty() && nested_assignments.is_empty() {
        // Nothing to name: use the default no-op.
        return quote! {};
    }

    quote! {
        fn assign_ids(&mut self, prefix: &str) {
            #(#parameter_assignments)*
            #(#nested_assignments)*
        }
    }
}

/// Generate `save_entries()`: what this struct adds to a saved state.
///
/// Parameters come first, in declaration order, then the `State` fields,
/// then the nested structs. The bytes are laid out by
/// `beamer_core::state_format`.
fn generate_save_state(ir: &ParametersIR) -> TokenStream {
    let parameter_saves: Vec<TokenStream> = ir
        .parameter_fields()
        .map(|parameter| {
            let field = &parameter.field_name;
            let id_str = &parameter.string_id;
            quote! {
                writer.parameter(
                    &::beamer::core::state_format::join_path(prefix, #id_str),
                    &self.#field,
                );
            }
        })
        .collect();

    let state_saves: Vec<TokenStream> = ir
        .state_fields()
        .map(|state| {
            let field = &state.field_name;
            let id_str = &state.string_id;
            quote! {
                writer.state(
                    &::beamer::core::state_format::join_path(prefix, #id_str),
                    &self.#field,
                );
            }
        })
        .collect();

    // A nested struct is saved under the name of its field, which stays
    // the same when the group gets another display name.
    let nested_saves: Vec<TokenStream> = ir
        .nested_fields()
        .map(|nested| {
            let field = &nested.field_name;
            let segment = syn::ext::IdentExt::unraw(&nested.field_name).to_string();
            quote! {
                self.#field.save_entries(
                    writer,
                    &::beamer::core::state_format::join_path(prefix, #segment),
                );
            }
        })
        .collect();

    quote! {
        fn save_entries(
            &self,
            writer: &mut ::beamer::core::state_format::StateWriter,
            prefix: &str,
        ) {
            #(#parameter_saves)*
            #(#state_saves)*
            #(#nested_saves)*
        }
    }
}

/// Generate `load_entries()`: every field takes its value from its entry.
fn generate_load_state(ir: &ParametersIR) -> TokenStream {
    let parameter_loads: Vec<TokenStream> = ir
        .parameter_fields()
        .map(|parameter| {
            let field = &parameter.field_name;
            let id_str = &parameter.string_id;
            quote! {
                entries.load_parameter(
                    &::beamer::core::state_format::join_path(prefix, #id_str),
                    &self.#field,
                );
            }
        })
        .collect();

    let state_loads: Vec<TokenStream> = ir
        .state_fields()
        .map(|state| {
            let field = &state.field_name;
            let id_str = &state.string_id;
            quote! {
                entries.load_state(
                    &::beamer::core::state_format::join_path(prefix, #id_str),
                    &self.#field,
                );
            }
        })
        .collect();

    let nested_loads: Vec<TokenStream> = ir
        .nested_fields()
        .map(|nested| {
            let field = &nested.field_name;
            let segment = syn::ext::IdentExt::unraw(&nested.field_name).to_string();
            quote! {
                self.#field.load_entries(
                    entries,
                    &::beamer::core::state_format::join_path(prefix, #segment),
                );
            }
        })
        .collect();

    quote! {
        fn load_entries(
            &self,
            entries: &::beamer::core::state_format::StateEntries<'_>,
            prefix: &str,
        ) {
            #(#parameter_loads)*
            #(#state_loads)*
            #(#nested_loads)*
        }
    }
}

/// Generate the `ParameterStore` trait implementation.
fn generate_parameter_store_impl(ir: &ParametersIR) -> TokenStream {
    let struct_name = &ir.struct_name;
    let (impl_generics, ty_generics, where_clause) = ir.generics.split_for_impl();

    let count_impl = generate_count(ir);
    let state_generation_impl = generate_state_generation(ir);
    let for_each_state_impl = generate_for_each_state(ir);

    // Generate info() - iterate and return by index
    let info_impl = generate_info(ir);

    quote! {
        impl #impl_generics ::beamer::core::parameter_store::ParameterStore for #struct_name #ty_generics #where_clause {
            fn count(&self) -> usize {
                #count_impl
            }

            #info_impl

            fn get_normalized(&self, id: ::beamer::core::types::ParameterId) -> ::beamer::core::types::ParameterValue {
                use ::beamer::core::parameter_types::Parameters;
                self.by_id(id).map(|p| p.get_normalized()).unwrap_or(0.0)
            }

            fn set_normalized(&self, id: ::beamer::core::types::ParameterId, value: ::beamer::core::types::ParameterValue) {
                use ::beamer::core::parameter_types::Parameters;
                if let Some(parameter) = self.by_id(id) {
                    parameter.set_normalized(value);
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

            fn formatter_kind(&self, id: ::beamer::core::types::ParameterId) -> &'static str {
                use ::beamer::core::parameter_types::Parameters;
                self.by_id(id).map(|p| p.formatter_kind()).unwrap_or("float")
            }

            #state_generation_impl

            #for_each_state_impl
        }
    }
}

/// Generate `state_generation()`: the sum of the generations of the `State`
/// fields of this struct and of its nested structs.
fn generate_state_generation(ir: &ParametersIR) -> TokenStream {
    let state_generations = ir.state_fields().map(|state| {
        let field = &state.field_name;
        quote! {
            .wrapping_add(::beamer::core::state_format::SavedState::generation(&self.#field))
        }
    });
    let nested_generations = ir.nested_fields().map(|nested| {
        let field = &nested.field_name;
        quote! {
            .wrapping_add(::beamer::core::parameter_store::ParameterStore::state_generation(&self.#field))
        }
    });

    quote! {
        fn state_generation(&self) -> u64 {
            0u64 #(#state_generations)* #(#nested_generations)*
        }
    }
}

/// Generate `webview_events()`: the queue of this struct, or else the first
/// one a nested struct has.
fn generate_webview_events(ir: &ParametersIR) -> TokenStream {
    if let Some(field) = ir.events_fields().next() {
        return quote! {
            fn webview_events(&self) -> Option<&::beamer::core::WebViewEvents> {
                Some(&self.#field)
            }
        };
    }

    let nested: Vec<TokenStream> = ir
        .nested_fields()
        .map(|nested| {
            let field = &nested.field_name;
            quote! {
                if let Some(events) = ::beamer::core::parameter_types::Parameters::webview_events(&self.#field) {
                    return Some(events);
                }
            }
        })
        .collect();

    // No queue here and no nested struct to look in: the default.
    if nested.is_empty() {
        return quote! {};
    }

    quote! {
        fn webview_events(&self) -> Option<&::beamer::core::WebViewEvents> {
            #(#nested)*
            None
        }
    }
}

/// Generate `for_each_state()`: the `State` fields of this struct with
/// their ids, then those of the nested structs under the names of their
/// fields, the same paths as in a saved state.
fn generate_for_each_state(ir: &ParametersIR) -> TokenStream {
    let states: Vec<TokenStream> = ir
        .state_fields()
        .map(|state| {
            let field = &state.field_name;
            let id_str = &state.string_id;
            quote! {
                visit(#id_str, &self.#field);
            }
        })
        .collect();
    let nested: Vec<TokenStream> = ir
        .nested_fields()
        .map(|nested| {
            let field = &nested.field_name;
            let segment = syn::ext::IdentExt::unraw(&nested.field_name).to_string();
            quote! {
                ::beamer::core::parameter_store::ParameterStore::for_each_state(
                    &self.#field,
                    &mut |path: &str, state: &dyn ::beamer::core::state_format::SavedState| {
                        visit(&::beamer::core::state_format::join_path(#segment, path), state)
                    },
                );
            }
        })
        .collect();

    // Nothing to visit: use the default, which visits nothing.
    if states.is_empty() && nested.is_empty() {
        return quote! {};
    }

    quote! {
        fn for_each_state(
            &self,
            visit: &mut dyn FnMut(&str, &dyn ::beamer::core::state_format::SavedState),
        ) {
            #(#states)*
            #(#nested)*
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
    // Generate calls for the float parameter fields. Only they can have a
    // smoother.
    let parameter_calls: Vec<TokenStream> = ir
        .parameter_fields()
        .filter(|parameter| parameter.parameter_type == crate::ir::ParameterType::Float)
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
        // Nothing to smooth: use the default no-op.
        quote! {}
    } else {
        quote! {
            fn set_sample_rate(&self, sample_rate: f64) {
                #(#parameter_calls)*
                #(#nested_calls)*
            }
        }
    }
}

/// Generate the `reset_smoothing()` method for the Parameters trait.
fn generate_reset_smoothing(ir: &ParametersIR) -> TokenStream {
    // Generate calls for the float parameter fields. Only they can have a
    // smoother.
    let parameter_calls: Vec<TokenStream> = ir
        .parameter_fields()
        .filter(|parameter| parameter.parameter_type == crate::ir::ParameterType::Float)
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
        // Nothing to smooth: use the default no-op.
        quote! {}
    } else {
        quote! {
            fn reset_smoothing(&self) {
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
            FieldIR::Parameter(p) => generate_parameter_initializer(p),
            FieldIR::Nested(n) => {
                let field = &n.field_name;
                quote! { #field: Default::default() }
            }
            FieldIR::State(s) => {
                let field = &s.field_name;
                quote! { #field: Default::default() }
            }
            // A field without an attribute, such as a `Handover`.
            FieldIR::Other(field) | FieldIR::Events(field) => quote! { #field: Default::default() },
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
                ::beamer::core::parameter_types::Parameters::assign_ids(&mut parameters, "");
                #group_id_init
                parameters
            }
        }
    }
}

/// Generate the initializer for a single parameter field.
fn generate_parameter_initializer(parameter: &ParameterFieldIR) -> TokenStream {
    let field = &parameter.field_name;

    // Generate constructor call
    let constructor = generate_constructor(parameter);

    // Generate builder chain (with_short_name, with_smoother). The ids
    // are set by assign_ids().
    let builder_chain = generate_builder_chain(parameter);

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

/// Generate the builder method chain (.with_short_name(), .with_smoother(), .with_step_size()).
fn generate_builder_chain(parameter: &ParameterFieldIR) -> TokenStream {
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

    // Optional: .with_step_size() (only for FloatParameter)
    let with_step_size = if parameter.parameter_type == crate::ir::ParameterType::Float {
        parameter.attributes.step.map(|step| {
            quote! { .with_step_size(#step) }
        })
    } else {
        None
    };

    quote! {
        #with_short_name
        #with_smoother
        #with_step_size
    }
}
