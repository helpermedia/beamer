//! Code generation for the model and group attributes.
//!
//! This module generates the Rust code for the `Parameters`, `Saved`,
//! `WebViewFields` and (for a model) `WebViewDispatch` trait
//! implementations from the validated IR. The `ParameterStore` of the
//! struct follows from `Parameters`.

use proc_macro2::TokenStream;
use quote::quote;

use crate::ir::{
    FieldIR, ParameterDefault, ParameterFieldIR, ParameterKind, ModelIR, SmoothingStyle,
};

/// Generate all code for the model and group attributes.
pub fn generate(ir: &ModelIR) -> TokenStream {
    let unit_consts = generate_group_consts(ir);
    let units_impl = generate_groups_impl(ir);
    let parameters_impl = generate_parameters_impl(ir);
    let saved_impl = generate_saved_impl(ir);
    let webview_fields_impl = generate_webview_fields_impl(ir);
    let webview_dispatch_impl = generate_webview_dispatch_impl(ir);
    let set_group_ids_impl = generate_set_group_ids(ir);
    let default_impl = generate_default_impl(ir);

    quote! {
        #unit_consts
        #units_impl
        #parameters_impl
        #saved_impl
        #webview_fields_impl
        #webview_dispatch_impl
        #set_group_ids_impl
        #default_impl
    }
}

/// The id of the group at `path` (see `beamer_core::parameter_groups::group_id`):
/// the name of a flat group, the field of a nested one.
fn group_id_expr(path: &str) -> TokenStream {
    quote! { ::beamer::core::parameter_groups::group_id(#path) }
}

/// Generate group ID constants for each nested field and flat group.
fn generate_group_consts(ir: &ModelIR) -> TokenStream {
    let struct_name = &ir.struct_name;
    let (impl_generics, ty_generics, where_clause) = ir.generics.split_for_impl();

    // Constants for flat groups, with the id of their name.
    let flat_groups = ir.flat_group_names();
    let flat_group_consts: Vec<TokenStream> = flat_groups
        .iter()
        .map(|group_name| {
            // Convert the group name to an uppercase const name: "Filter" becomes
            // "PARAMETER_GROUP_FILTER".
            let const_name = syn::Ident::new(
                &format!("PARAMETER_GROUP_{}", group_name.to_uppercase().replace(' ', "_")),
                proc_macro2::Span::call_site(),
            );
            let group_id = group_id_expr(group_name);
            quote! {
                /// The id of the flat parameter group, when this struct is the
                /// root of the plugin (not nested in another).
                pub const #const_name: ::beamer::core::parameter_groups::GroupId = #group_id;
            }
        })
        .collect();

    // Constants for nested groups, with the id of their field.
    let nested_consts: Vec<TokenStream> = ir
        .nested_fields()
        .map(|nested| {
            let field = syn::ext::IdentExt::unraw(&nested.field_name).to_string();
            let const_name = syn::Ident::new(&format!("GROUP_{}", field.to_uppercase()), nested.span);
            let group_id = group_id_expr(&field);
            quote! {
                /// The id of the group of the nested field, when this struct is
                /// the root of the plugin (not nested in another).
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
fn generate_groups_impl(ir: &ModelIR) -> TokenStream {
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

    if has_nested {
        // Flat and nested groups in the order of the fields: a flat group
        // where its first parameter is, a nested group at its field with
        // the groups inside it right after it. VST3 units and AUv2 clumps
        // are numbered in this order.
        let mut seen_flat = std::collections::HashSet::new();
        let mut nested_index = 0usize;
        let mut pushes: Vec<TokenStream> = Vec::new();
        for field in &ir.fields {
            match field {
                crate::ir::FieldIR::Parameter(parameter) => {
                    if let Some(group_name) = &parameter.attributes.group {
                        if seen_flat.insert(group_name.as_str()) {
                            let group_id = group_id_expr(group_name);
                            pushes.push(quote! {
                                groups.push(::beamer::core::parameter_groups::GroupInfo::new(
                                    #group_id,
                                    #group_name,
                                    ::beamer::core::parameter_groups::ROOT_GROUP_ID,
                                ));
                            });
                        }
                    }
                }
                crate::ir::FieldIR::Nested(_) => {
                    let index = nested_index;
                    nested_index += 1;
                    pushes.push(quote! {
                        if let Some((field, name, nested)) = Parameters::nested_group(self, #index) {
                            let id = ::beamer::core::parameter_groups::group_id(field);
                            groups.push(::beamer::core::parameter_groups::GroupInfo::new(
                                id,
                                name,
                                ::beamer::core::parameter_groups::ROOT_GROUP_ID,
                            ));
                            nested.collect_groups(&mut groups, field, id);
                        }
                    });
                }
                _ => {}
            }
        }
        let collect = quote! {
            use ::beamer::core::parameter_types::Parameters;
            let mut groups: Vec<::beamer::core::parameter_groups::GroupInfo> = Vec::new();
            #(#pushes)*
        };

        quote! {
            impl #impl_generics ::beamer::core::parameter_groups::ParameterGroups for #struct_name #ty_generics #where_clause {
                fn group_count(&self) -> usize {
                    #collect
                    // The root, then the groups.
                    1 + groups.len()
                }

                fn group_info(&self, index: usize) -> Option<::beamer::core::parameter_groups::GroupInfo> {
                    if index == 0 {
                        return Some(::beamer::core::parameter_groups::GroupInfo::root());
                    }
                    #collect
                    groups.get(index - 1).cloned()
                }
            }
        }
    } else {
        // Only flat groups, no nesting: their entries in the order of first use.
        let flat_group_count = flat_groups.len();
        let flat_group_infos: Vec<TokenStream> = flat_groups
            .iter()
            .enumerate()
            .map(|(idx, group_name)| {
                let group_id = group_id_expr(group_name);
                // All flat groups are children of the root.
                quote! {
                    #idx => Some(::beamer::core::parameter_groups::GroupInfo {
                        id: #group_id,
                        name: #group_name,
                        parent_id: ::beamer::core::parameter_groups::ROOT_GROUP_ID,
                    }),
                }
            })
            .collect();

        quote! {
            impl #impl_generics ::beamer::core::parameter_groups::ParameterGroups for #struct_name #ty_generics #where_clause {
                fn group_count(&self) -> usize {
                    // The root, then the flat groups.
                    1 + #flat_group_count
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
fn generate_set_group_ids(ir: &ModelIR) -> TokenStream {
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

    // Generate statements to set group_id on parameters with flat groups
    let flat_group_assignments: Vec<TokenStream> = ir
        .parameter_fields()
        .filter_map(|parameter| {
            parameter.attributes.group.as_ref().map(|group_name| {
                let field = &parameter.field_name;
                let group_id = group_id_expr(group_name);
                quote! {
                    self.#field.set_group_id(#group_id);
                }
            })
        })
        .collect();

    let nested_init = if has_nested {
        quote! {
            use ::beamer::core::parameter_types::Parameters;
            self.assign_group_ids("");
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
            /// A group's id is the hash of its path (see
            /// `beamer::core::parameter_groups::group_id`); 0 is the root.
            ///
            /// Call this once after construction to set up the group hierarchy.
            ///
            /// # Example
            ///
            /// ```ignore
            /// let mut model = SynthModel::default();
            /// model.set_group_ids();
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
fn generate_parameters_impl(ir: &ModelIR) -> TokenStream {
    let struct_name = &ir.struct_name;
    let (impl_generics, ty_generics, where_clause) = ir.generics.split_for_impl();

    let count_impl = generate_count(ir);
    let iter_impl = generate_iter(ir);
    let by_id_impl = generate_by_id(ir);
    let assign_ids_impl = generate_assign_ids(ir);
    let set_all_group_ids_impl = generate_set_all_group_ids(ir);
    let set_middle_c_impl = generate_set_middle_c(ir);
    let nested_discovery_impl = generate_nested_discovery(ir);
    let set_sample_rate_impl = generate_set_sample_rate(ir);
    let reset_smoothing_impl = generate_reset_smoothing(ir);
    let reset_parameters_impl = generate_reset_parameters(ir);

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

            #assign_ids_impl

            #set_all_group_ids_impl

            #set_middle_c_impl

            #nested_discovery_impl

            #set_sample_rate_impl

            #reset_smoothing_impl

            #reset_parameters_impl
        }
    }
}

/// Generate the `set_all_group_ids()` method for the Parameters trait.
fn generate_set_all_group_ids(ir: &ModelIR) -> TokenStream {
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

/// Generate `set_middle_c()`: every `IntParameter` of this struct and of
/// its nested groups names notes with it.
fn generate_set_middle_c(ir: &ModelIR) -> TokenStream {
    let parameter_assignments: Vec<TokenStream> = ir
        .parameter_fields()
        .filter(|parameter| parameter.parameter_type == crate::ir::ParameterType::Int)
        .map(|parameter| {
            let field = &parameter.field_name;
            quote! {
                self.#field.set_middle_c(middle_c);
            }
        })
        .collect();

    let nested_assignments: Vec<TokenStream> = ir
        .nested_fields()
        .map(|nested| {
            let field = &nested.field_name;
            quote! {
                ::beamer::core::parameter_types::Parameters::set_middle_c(&mut self.#field, middle_c);
            }
        })
        .collect();

    if parameter_assignments.is_empty() && nested_assignments.is_empty() {
        // Nothing names notes: use the default no-op.
        return quote! {};
    }

    quote! {
        fn set_middle_c(&mut self, middle_c: ::beamer::core::MiddleC) {
            #(#parameter_assignments)*
            #(#nested_assignments)*
        }
    }
}

/// Generate the nested group discovery methods for the Parameters trait.
fn generate_nested_discovery(ir: &ModelIR) -> TokenStream {
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
            let segment = syn::ext::IdentExt::unraw(&nested.field_name).to_string();
            let name = &nested.group_name;
            quote! {
                #idx => Some((#segment, #name, &self.#field as &dyn ::beamer::core::parameter_types::Parameters)),
            }
        })
        .collect();

    // Generate match arms for nested_group_mut()
    let group_mut_match_arms: Vec<TokenStream> = ir
        .nested_fields()
        .enumerate()
        .map(|(idx, nested)| {
            let field = &nested.field_name;
            let segment = syn::ext::IdentExt::unraw(&nested.field_name).to_string();
            let name = &nested.group_name;
            quote! {
                #idx => Some((#segment, #name, &mut self.#field as &mut dyn ::beamer::core::parameter_types::Parameters)),
            }
        })
        .collect();

    quote! {
        fn nested_count(&self) -> usize {
            #nested_count
        }

        fn nested_group(&self, index: usize) -> Option<(&'static str, &'static str, &dyn ::beamer::core::parameter_types::Parameters)> {
            match index {
                #(#group_match_arms)*
                _ => None,
            }
        }

        fn nested_group_mut(&mut self, index: usize) -> Option<(&'static str, &'static str, &mut dyn ::beamer::core::parameter_types::Parameters)> {
            match index {
                #(#group_mut_match_arms)*
                _ => None,
            }
        }
    }
}

/// Generate the count() method body.
fn generate_count(ir: &ModelIR) -> TokenStream {
    let parameter_count = ir.parameter_count();

    if ir.has_nested() {
        let nested_counts: Vec<TokenStream> = ir
            .nested_fields()
            .map(|nested| {
                let field = &nested.field_name;
                // Fully qualified: a `ParameterStore` has a `count` as well.
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
///
/// The parameters come in the order of the fields: a nested group's
/// parameters where its field is, a flat group's parameters together where
/// its first parameter is, as hosts show a group's parameters together.
fn generate_iter(ir: &ModelIR) -> TokenStream {
    // Collected into a vector: chained iterators nest one type per field,
    // which the compiler refuses past its recursion limit for a struct with
    // many fields.
    let push = |field: &syn::Ident| {
        quote! {
            all.push(&self.#field as &dyn ::beamer::core::parameter_types::ParameterRef);
        }
    };
    let mut flat_groups_done = std::collections::HashSet::new();
    let mut parts: Vec<TokenStream> = Vec::new();
    for field in &ir.fields {
        match field {
            crate::ir::FieldIR::Parameter(parameter) => match &parameter.attributes.group {
                Some(group) => {
                    if flat_groups_done.insert(group.as_str()) {
                        for member in ir.parameter_fields() {
                            if member.attributes.group.as_deref() == Some(group.as_str()) {
                                parts.push(push(&member.field_name));
                            }
                        }
                    }
                }
                None => parts.push(push(&parameter.field_name)),
            },
            crate::ir::FieldIR::Nested(nested) => {
                let field = &nested.field_name;
                parts.push(quote! { all.extend(::beamer::core::parameter_types::Parameters::iter(&self.#field)); });
            }
            _ => {}
        }
    }

    quote! {
        let mut all: Vec<&dyn ::beamer::core::parameter_types::ParameterRef> =
            Vec::with_capacity(::beamer::core::parameter_types::Parameters::count(self));
        #(#parts)*
        Box::new(all.into_iter())
    }
}

/// Generate the by_id() method body.
///
/// The ids are compared at run time: a parameter of a nested group has an
/// id that depends on where the struct sits.
fn generate_by_id(ir: &ModelIR) -> TokenStream {
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
/// parameter and the field name of a nested group.
fn generate_assign_ids(ir: &ModelIR) -> TokenStream {
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

    // A model connects its declared senders and events to its queue once
    // all names are known, those of its nested groups too. A group is
    // connected by the model it sits in.
    let connect = if ir.generate_dispatch {
        quote! {
            let queue = ::beamer::webview::WebViewFields::webview_queue(self).cloned();
            if let ::std::option::Option::Some(queue) = queue {
                ::beamer::webview::WebViewFields::connect_webview(self, prefix, &queue);
            }
        }
    } else {
        quote! {}
    };

    if parameter_assignments.is_empty() && nested_assignments.is_empty() && !ir.generate_dispatch {
        // Nothing to name: use the default no-op.
        return quote! {};
    }

    quote! {
        fn assign_ids(&mut self, prefix: &str) {
            #(#parameter_assignments)*
            #(#nested_assignments)*
            #connect
        }
    }
}

/// Generate `save_entries()`: what this struct adds to a saved state.
///
/// Parameters come first, in declaration order, then the `State` fields,
/// then the nested groups. The bytes are laid out by
/// `beamer_core::state_format`.
fn generate_save_state(ir: &ModelIR) -> TokenStream {
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

    // A nested group is saved under the name of its field, which stays
    // the same when the group gets another display name.
    let nested_saves: Vec<TokenStream> = ir
        .nested_fields()
        .map(|nested| {
            let field = &nested.field_name;
            let segment = syn::ext::IdentExt::unraw(&nested.field_name).to_string();
            quote! {
                ::beamer::core::model::Saved::save_entries(
                    &self.#field,
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
fn generate_load_state(ir: &ModelIR) -> TokenStream {
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
                ::beamer::core::model::Saved::load_entries(
                    &self.#field,
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

/// Generate the `Saved` trait implementation: the `State` fields of the
/// struct, saved and loaded with the parameters.
fn generate_saved_impl(ir: &ModelIR) -> TokenStream {
    let struct_name = &ir.struct_name;
    let (impl_generics, ty_generics, where_clause) = ir.generics.split_for_impl();

    let state_generation_impl = generate_state_generation(ir);
    let for_each_state_impl = generate_for_each_state(ir);
    let save_state_impl = generate_save_state(ir);
    let load_state_impl = generate_load_state(ir);

    quote! {
        impl #impl_generics ::beamer::core::model::Saved for #struct_name #ty_generics #where_clause {
            #state_generation_impl

            #for_each_state_impl

            #save_state_impl

            #load_state_impl
        }
    }
}

/// Generate the `WebViewFields` trait implementation: where the queue of
/// events is, the connection of the declared senders and events to it and
/// the ids and types of the page's model types.
fn generate_webview_fields_impl(ir: &ModelIR) -> TokenStream {
    let struct_name = &ir.struct_name;
    let (impl_generics, ty_generics, where_clause) = ir.generics.split_for_impl();

    let webview_queue = generate_webview_queue(ir);
    let connect_webview = generate_connect_webview(ir);
    let describe_model_types = generate_describe_model_types(ir);

    quote! {
        impl #impl_generics ::beamer::webview::WebViewFields for #struct_name #ty_generics #where_clause {
            #webview_queue
            #connect_webview
            #describe_model_types
        }
    }
}

/// Generate `state_generation()`: the sum of the generations of the `State`
/// fields of this struct and of its nested groups.
fn generate_state_generation(ir: &ModelIR) -> TokenStream {
    let state_generations = ir.state_fields().map(|state| {
        let field = &state.field_name;
        quote! {
            .wrapping_add(::beamer::core::state_format::SavedState::generation(&self.#field))
        }
    });
    let nested_generations = ir.nested_fields().map(|nested| {
        let field = &nested.field_name;
        quote! {
            .wrapping_add(::beamer::core::model::Saved::state_generation(&self.#field))
        }
    });

    quote! {
        fn state_generation(&self) -> u64 {
            0u64 #(#state_generations)* #(#nested_generations)*
        }
    }
}

/// Generate `webview_queue()`: the queue of this struct, or else the first
/// one a nested group has.
fn generate_webview_queue(ir: &ModelIR) -> TokenStream {
    if let Some(field) = ir.event_queue_fields().next() {
        return quote! {
            fn webview_queue(&self) -> Option<&::beamer::webview::WebViewEventQueue> {
                Some(&self.#field)
            }
        };
    }

    let nested: Vec<TokenStream> = ir
        .nested_fields()
        .map(|nested| {
            let field = &nested.field_name;
            quote! {
                if let Some(queue) = ::beamer::webview::WebViewFields::webview_queue(&self.#field) {
                    return Some(queue);
                }
            }
        })
        .collect();

    // No queue here and no nested group to look in: the default.
    if nested.is_empty() {
        return quote! {};
    }

    quote! {
        fn webview_queue(&self) -> Option<&::beamer::webview::WebViewEventQueue> {
            #(#nested)*
            None
        }
    }
}

/// Generate `connect_webview()`: each declared sender and event gets its
/// name (the id behind `prefix`) and the queue; then the nested groups,
/// under the names of their fields.
fn generate_connect_webview(ir: &ModelIR) -> TokenStream {
    let fields: Vec<TokenStream> = ir
        .webview_fields()
        .map(|webview| {
            let field = &webview.field_name;
            let id = &webview.id;
            quote! {
                self.#field.connect(::beamer::core::state_format::join_path(prefix, #id), queue);
            }
        })
        .collect();
    let nested: Vec<TokenStream> = ir
        .nested_fields()
        .map(|nested| {
            let field = &nested.field_name;
            let segment = syn::ext::IdentExt::unraw(&nested.field_name).to_string();
            quote! {
                ::beamer::webview::WebViewFields::connect_webview(
                    &mut self.#field,
                    &::beamer::core::state_format::join_path(prefix, #segment),
                    queue,
                );
            }
        })
        .collect();

    // Nothing to connect: use the default.
    if fields.is_empty() && nested.is_empty() {
        return quote! {};
    }

    quote! {
        fn connect_webview(&mut self, prefix: &str, queue: &::beamer::webview::WebViewEventQueue) {
            #(#fields)*
            #(#nested)*
        }
    }
}

/// The TypeScript of what a field of type `field_type` carries, or `None`
/// when that type has no `TypeScript`: chosen when the generated code is
/// compiled, as the dispatch to a handler is.
fn type_script_if_any(field_type: &syn::Type) -> TokenStream {
    quote! {
        |types: &mut ::beamer::webview::type_script::TypeScriptTypes| {
            #[allow(unused_imports)]
            use ::beamer::webview::type_script::__private::{HasTypeScript as _, LacksTypeScript as _};
            (&::beamer::webview::type_script::__private::TypeProbe::<
                <#field_type as ::beamer::webview::model_types::Carries>::Value,
            >::new())
                .type_script_if_any(types)
        }
    }
}

/// The first type parameter of `generics` that `ty` names, as a string.
pub(crate) fn named_type_parameter(ty: &syn::Type, generics: &syn::Generics) -> Option<String> {
    fn find(tokens: proc_macro2::TokenStream, names: &[String]) -> Option<String> {
        tokens.into_iter().find_map(|token| match token {
            proc_macro2::TokenTree::Ident(ident) => {
                let name = ident.to_string();
                names.contains(&name).then_some(name)
            }
            proc_macro2::TokenTree::Group(group) => find(group.stream(), names),
            _ => None,
        })
    }
    let names: Vec<String> = generics.type_params().map(|parameter| parameter.ident.to_string()).collect();
    if names.is_empty() {
        return None;
    }
    find(quote::ToTokens::to_token_stream(ty), &names)
}

/// Generate `describe_model_types()`: the `State` fields, senders and
/// events of this struct with the types they carry, then those of the
/// nested groups under the names of their fields. The parameters come from
/// the model's store, in the order of the init dump.
fn generate_describe_model_types(ir: &ModelIR) -> TokenStream {
    let rust_type = |field_type: &syn::Type| {
        quote! {
            ::std::any::type_name::<<#field_type as ::beamer::webview::model_types::Carries>::Value>()
        }
    };
    // The model's type parameter that a field's type names, for the note
    // that its TypeScript needs a bound.
    let type_parameter = |field_type: &syn::Type| match named_type_parameter(field_type, &ir.generics) {
        Some(name) => quote!(::core::option::Option::Some(#name)),
        None => quote!(::core::option::Option::None),
    };
    let states: Vec<TokenStream> = ir
        .state_fields()
        .map(|state| {
            let id = &state.string_id;
            let rust_type = rust_type(&state.field_type);
            let type_parameter = type_parameter(&state.field_type);
            let type_script = type_script_if_any(&state.field_type);
            quote! {
                types.state(
                    ::beamer::core::state_format::join_path(prefix, #id),
                    #rust_type,
                    #type_parameter,
                    #type_script,
                );
            }
        })
        .collect();
    let webview: Vec<TokenStream> = ir
        .webview_fields()
        .map(|webview| {
            let id = &webview.id;
            let method = syn::Ident::new(webview.kind.model_types_method(), proc_macro2::Span::call_site());
            let rust_type = rust_type(&webview.field_type);
            let type_parameter = type_parameter(&webview.field_type);
            let type_script = type_script_if_any(&webview.field_type);
            quote! {
                types.#method(
                    ::beamer::core::state_format::join_path(prefix, #id),
                    #rust_type,
                    #type_parameter,
                    #type_script,
                );
            }
        })
        .collect();
    let nested: Vec<TokenStream> = ir
        .nested_fields()
        .map(|nested| {
            let field_type = &nested.field_type;
            let segment = syn::ext::IdentExt::unraw(&nested.field_name).to_string();
            quote! {
                <#field_type as ::beamer::webview::WebViewFields>::describe_model_types(
                    types,
                    &::beamer::core::state_format::join_path(prefix, #segment),
                );
            }
        })
        .collect();

    // Nothing to describe: use the default.
    if states.is_empty() && webview.is_empty() && nested.is_empty() {
        return quote! {};
    }

    quote! {
        fn describe_model_types(types: &mut ::beamer::webview::ModelTypes, prefix: &str)
        where
            Self: Sized,
        {
            #(#states)*
            #(#webview)*
            #(#nested)*
        }
    }
}

/// Generate `WebViewDispatch` for a model: the way of the format wrappers to
/// the model's `WebViewHandler`, or to the answers of a model without one.
///
/// The generated code knows the type of the model, so `&ModelRef(self)`
/// finds `Implemented` when the model implements `WebViewHandler` and
/// `Missing`, one reference further, when it does not. For a generic
/// model the impl has to hold for every type the struct takes.
fn generate_webview_dispatch_impl(ir: &ModelIR) -> TokenStream {
    if !ir.generate_dispatch {
        return quote! {};
    }

    let struct_name = &ir.struct_name;
    let (impl_generics, ty_generics, where_clause) = ir.generics.split_for_impl();

    quote! {
        impl #impl_generics ::beamer::webview::WebViewDispatch for #struct_name #ty_generics #where_clause {
            fn dispatch_invoke(
                &self,
                method: &str,
                args: &[::beamer::webview::serde_json::Value],
            ) -> ::std::result::Result<::beamer::webview::serde_json::Value, ::std::string::String> {
                #[allow(unused_imports)]
                use ::beamer::webview::__private::{Implemented as _, Missing as _};
                (&::beamer::webview::__private::ModelRef(self)).invoke(method, args)
            }

            fn dispatch_event(&self, name: &str, data: &::beamer::webview::serde_json::Value) {
                #[allow(unused_imports)]
                use ::beamer::webview::__private::{Implemented as _, Missing as _};
                (&::beamer::webview::__private::ModelRef(self)).event(name, data);
            }

            fn dispatch_describe_handler(types: &mut ::beamer::webview::ModelTypes) {
                #[allow(unused_imports)]
                use ::beamer::webview::__private::{Described as _, Undescribed as _};
                (&::beamer::webview::__private::HandlerOf::<Self>::new()).describe(types);
            }
        }
    }
}

/// Generate `for_each_state()`: the `State` fields of this struct with
/// their ids, then those of the nested groups under the names of their
/// fields, the same paths as in a saved state.
fn generate_for_each_state(ir: &ModelIR) -> TokenStream {
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
                ::beamer::core::model::Saved::for_each_state(
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

/// Generate the `set_sample_rate()` method for the Parameters trait.
fn generate_set_sample_rate(ir: &ModelIR) -> TokenStream {
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
fn generate_reset_smoothing(ir: &ModelIR) -> TokenStream {
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

/// Generate the `reset_parameters()` method for the Parameters trait.
///
/// It names each field, so unlike the default of the trait it does not
/// allocate and a factory preset can call it on the audio thread.
fn generate_reset_parameters(ir: &ModelIR) -> TokenStream {
    let parameter_calls: Vec<TokenStream> = ir
        .parameter_fields()
        .map(|parameter| {
            let field = &parameter.field_name;
            quote! {
                ::beamer::core::parameter_types::ParameterRef::set_normalized(
                    &self.#field,
                    ::beamer::core::parameter_types::ParameterRef::default_normalized(&self.#field),
                );
            }
        })
        .collect();

    let nested_calls: Vec<TokenStream> = ir
        .nested_fields()
        .map(|nested| {
            let field = &nested.field_name;
            quote! { ::beamer::core::parameter_types::Parameters::reset_parameters(&self.#field); }
        })
        .collect();

    quote! {
        fn reset_parameters(&self) {
            #(#parameter_calls)*
            #(#nested_calls)*
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
fn generate_default_impl(ir: &ModelIR) -> TokenStream {
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
            FieldIR::WebView(w) => {
                let field = &w.field_name;
                quote! { #field: Default::default() }
            }
            // A field without an attribute, such as a `Handover`.
            FieldIR::Other(field) | FieldIR::EventQueue(field) => quote! { #field: Default::default() },
        })
        .collect();

    // Add set_group_ids() call if there are groups (flat or nested)
    let group_id_init = if ir.has_nested() || ir.has_flat_groups() {
        quote! {
            this.set_group_ids();
        }
    } else {
        quote! {}
    };

    quote! {
        impl #impl_generics Default for #struct_name #ty_generics #where_clause {
            fn default() -> Self {
                let mut this = Self {
                    #(#field_inits),*
                };
                ::beamer::core::parameter_types::Parameters::assign_ids(&mut this, "");
                #group_id_init
                this
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

    // Get range (required unless the kind has a default range).
    let (start, end) = parameter
        .attributes
        .range
        .as_ref()
        .map(|r| (r.start, r.end))
        .or_else(|| kind.default_range())
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
        ParameterKind::Percent => quote! {
            ::beamer::core::parameter_types::FloatParameter::percent(#name, #default, #start..=#end)
        },
        ParameterKind::Pan => quote! {
            ::beamer::core::parameter_types::FloatParameter::pan(#name, #default, #start..=#end)
        },
        ParameterKind::Ratio => quote! {
            ::beamer::core::parameter_types::FloatParameter::ratio(#name, #default, #start..=#end)
        },
        ParameterKind::Linear => quote! {
            ::beamer::core::parameter_types::FloatParameter::new(#name, #default, #start..=#end)
        },
        ParameterKind::Semitones | ParameterKind::NoteNumber => {
            // An int kind, rejected for a FloatParameter by validation.
            quote! {
                ::beamer::core::parameter_types::FloatParameter::new(#name, #default, #start..=#end)
            }
        }
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

    match parameter.attributes.kind {
        Some(ParameterKind::Semitones) => quote! {
            ::beamer::core::parameter_types::IntParameter::semitones(#name, #default, #start..=#end)
        },
        Some(ParameterKind::NoteNumber) => quote! {
            ::beamer::core::parameter_types::IntParameter::note_number(#name, #default, #start..=#end)
        },
        _ => quote! {
            ::beamer::core::parameter_types::IntParameter::new(#name, #default, #start..=#end)
        },
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
