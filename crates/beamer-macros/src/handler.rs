//! The `#[beamer::handler]` attribute: the model's answers to its web UI.
//!
//! On an impl block of the model, it generates `WebViewHandler` from the
//! methods marked `#[method(id = "...")]` (what `invoke()` calls) and
//! `#[page_event(id = "...")]` (what `emit()` sends), with their types for
//! the page's model types. The other methods of the block stay as they are.

use proc_macro2::TokenStream;
use quote::{quote, quote_spanned};
use syn::spanned::Spanned;

use crate::codegen::named_type_parameter;

/// What a marked method answers.
enum Kind {
    Method,
    PageEvent,
}

impl Kind {
    fn attribute(&self) -> &'static str {
        match self {
            Kind::Method => "method",
            Kind::PageEvent => "page_event",
        }
    }
}

/// A method of the block that the web UI reaches.
struct Marked {
    kind: Kind,
    id: String,
    ident: syn::Ident,
    // The types of the arguments after `&self`.
    arguments: Vec<syn::Type>,
    // The type the page's promise resolves to: the value of a `Result`, `()`
    // for a method that returns nothing.
    result: syn::Type,
    // The function's `#[cfg]` attributes, for its arm and its description.
    cfgs: Vec<syn::Attribute>,
}

/// Expand `#[beamer::handler]` on `item`.
pub fn expand(attr: TokenStream, item: TokenStream) -> TokenStream {
    match expand_impl(attr, item) {
        Ok(tokens) => tokens,
        Err(error) => error.to_compile_error(),
    }
}

/// The block with its handler. When a marked function has an error, the
/// block stays without the handler, so that the uses of its other functions
/// add no errors of their own.
fn expand_impl(attr: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    if !attr.is_empty() {
        return Err(syn::Error::new_spanned(
            attr,
            "#[beamer::handler] takes no options",
        ));
    }
    let mut block: syn::ItemImpl = syn::parse2(item)?;
    if let Some((path, _)) = &block.trait_ {
        return Err(syn::Error::new_spanned(
            path,
            "#[beamer::handler] goes on an impl block of the model itself, not of a trait",
        ));
    }

    let mut marked = Vec::new();
    let mut errors: Option<syn::Error> = None;
    let mut push = |error: syn::Error| match &mut errors {
        Some(errors) => errors.combine(error),
        None => errors = Some(error),
    };
    for item in &mut block.items {
        let syn::ImplItem::Fn(function) = item else {
            continue;
        };
        match take_marked(function) {
            Ok(Some(method)) => marked.push(method),
            Ok(None) => {}
            Err(error) => push(error),
        }
    }
    for (index, method) in marked.iter().enumerate() {
        let same_kind = |other: &&Marked| {
            std::mem::discriminant(&other.kind) == std::mem::discriminant(&method.kind)
        };
        if let Some(first) = marked[..index]
            .iter()
            .filter(same_kind)
            .find(|other| other.id == method.id)
        {
            push(syn::Error::new(
                method.ident.span(),
                format!(
                    "a second #[{}] is named \"{}\": `{}` has the name already",
                    method.kind.attribute(),
                    method.id,
                    first.ident
                ),
            ));
        }
    }
    if let Some(errors) = errors {
        let errors = errors.to_compile_error();
        return Ok(quote! {
            #block
            #errors
        });
    }

    let handler = generate_handler(&block, &marked);
    Ok(quote! {
        #block
        #handler
    })
}

/// Take the `#[method]` or `#[page_event]` attribute off `function`, with
/// what the web UI needs of the method.
fn take_marked(function: &mut syn::ImplItemFn) -> syn::Result<Option<Marked>> {
    let mut found = None;
    let mut kept = Vec::with_capacity(function.attrs.len());
    for attr in std::mem::take(&mut function.attrs) {
        let kind = if attr.path().is_ident("method") {
            Kind::Method
        } else if attr.path().is_ident("page_event") {
            Kind::PageEvent
        } else {
            kept.push(attr);
            continue;
        };
        if found.is_some() {
            return Err(syn::Error::new_spanned(
                attr,
                "a method answers the web UI under one name: one #[method] or #[page_event]",
            ));
        }
        found = Some((kind, attribute_id(&attr)?));
    }
    function.attrs = kept;
    let Some((kind, id)) = found else {
        return Ok(None);
    };
    let cfgs = function
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("cfg"))
        .cloned()
        .collect();

    let signature = &function.sig;
    let attribute = kind.attribute();
    if signature.generics.type_params().next().is_some()
        || signature.generics.const_params().next().is_some()
    {
        return Err(syn::Error::new_spanned(
            &signature.generics,
            format!("a #[{attribute}] function has no type parameters: the page's types are fixed"),
        ));
    }
    if let Some(token) = &signature.asyncness {
        return Err(syn::Error::new_spanned(
            token,
            format!("a #[{attribute}] function is not async"),
        ));
    }
    let mut inputs = signature.inputs.iter();
    match inputs.next() {
        Some(syn::FnArg::Receiver(receiver))
            if matches!(receiver.kind, syn::ReceiverKind::Reference(_, _, None)) => {}
        _ => {
            return Err(syn::Error::new_spanned(
                signature,
                format!(
                    "a #[{attribute}] function takes `&self`: the model is shared with the host \
                     and the audio thread"
                ),
            ))
        }
    }
    let mut arguments = Vec::new();
    for input in inputs {
        let syn::FnArg::Typed(typed) = input else {
            continue;
        };
        if let syn::Type::Reference(reference) = &*typed.ty {
            return Err(syn::Error::new_spanned(
                reference,
                "the page's arguments are read with serde into owned values: take `String`, \
                 not `&str`, or `Vec<T>`, not `&[T]`",
            ));
        }
        refuse_impl_trait(&typed.ty)?;
        arguments.push((*typed.ty).clone());
    }
    if matches!(kind, Kind::PageEvent) && arguments.len() > 1 {
        return Err(syn::Error::new_spanned(
            &signature.inputs,
            "a #[page_event] function takes the event's data as its one argument after `&self`, \
             or none for an event without data",
        ));
    }
    let unit: syn::Type = syn::parse_quote!(());
    let result = match &signature.output {
        syn::ReturnType::Default => unit,
        syn::ReturnType::Type(_, ty) if is_unit(ty) => unit,
        syn::ReturnType::Type(_, ty) => {
            if matches!(kind, Kind::PageEvent) {
                return Err(syn::Error::new_spanned(
                    &signature.output,
                    "a #[page_event] function returns nothing: the page does not wait for an answer",
                ));
            }
            refuse_impl_trait(ty)?;
            result_value(ty).unwrap_or_else(|| (**ty).clone())
        }
    };
    Ok(Some(Marked {
        kind,
        id,
        ident: signature.ident.clone(),
        arguments,
        result,
        cfgs,
    }))
}

/// Whether `ty` is `()`.
fn is_unit(ty: &syn::Type) -> bool {
    matches!(ty, syn::Type::Tuple(tuple) if tuple.elems.is_empty())
}

/// Refuse `impl Trait`: the page's types need the type itself.
fn refuse_impl_trait(ty: &syn::Type) -> syn::Result<()> {
    if let syn::Type::ImplTrait(ty) = ty {
        return Err(syn::Error::new_spanned(
            ty,
            "the page's types need the type itself: name it instead of `impl Trait`",
        ));
    }
    Ok(())
}

/// The id of `#[method(id = "...")]` or `#[page_event(id = "...")]`.
fn attribute_id(attr: &syn::Attribute) -> syn::Result<String> {
    let name = if attr.path().is_ident("method") {
        "method"
    } else {
        "page_event"
    };
    let requires_id = || {
        syn::Error::new_spanned(
            attr,
            format!("#[{name}] requires an id: #[{name}(id = \"...\")]"),
        )
    };
    if matches!(attr.meta, syn::Meta::Path(_)) {
        return Err(requires_id());
    }
    let mut id = None;
    attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("id") {
            if id.is_some() {
                return Err(meta.error("a second id: give one"));
            }
            id = Some(meta.value()?.parse::<syn::LitStr>()?);
            Ok(())
        } else {
            Err(meta.error("unknown attribute. Expected: id"))
        }
    })?;
    let Some(id) = id else {
        return Err(requires_id());
    };
    let value = id.value();
    if value.is_empty() {
        return Err(syn::Error::new_spanned(
            &id,
            format!("{name} id must not be empty"),
        ));
    }
    if value.starts_with("_beamer/") {
        return Err(syn::Error::new_spanned(
            &id,
            "ids that start with `_beamer/` belong to the framework",
        ));
    }
    Ok(value)
}

/// The value type of a `Result` by its name: its first type argument. A
/// `Result` under another name is answered as one all the same, but the
/// page's types cannot see its value.
fn result_value(ty: &syn::Type) -> Option<syn::Type> {
    let syn::Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;
    if segment.ident != "Result" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    arguments.args.iter().find_map(|argument| match argument {
        syn::GenericArgument::Type(ty) => Some(ty.clone()),
        _ => None,
    })
}

/// The `WebViewHandler` of the block: the dispatch of each name to its
/// method and the description of their types.
fn generate_handler(block: &syn::ItemImpl, marked: &[Marked]) -> TokenStream {
    let (impl_generics, _, where_clause) = block.generics.split_for_impl();
    let self_ty = &block.self_ty;
    let value = quote!(::beamer::webview::serde_json::Value);
    let private = quote!(::beamer::webview::__private);

    let method_arms = marked
        .iter()
        .filter(|method| matches!(method.kind, Kind::Method))
        .map(|method| {
            let id = &method.id;
            let ident = &method.ident;
            let cfgs = &method.cfgs;
            let count = method.arguments.len();
            let names: Vec<syn::Ident> = (0..count)
                .map(|index| syn::Ident::new(&format!("argument_{index}"), ident.span()))
                .collect();
            // Spanned on the type, so that a type serde cannot read is
            // reported there.
            let reads = method.arguments.iter().zip(&names).enumerate().map(|(index, (ty, name))| {
                quote_spanned!(ty.span()=> let #name = #private::argument::<#ty>(method, #index, &args[#index])?;)
            });
            quote! {
                #(#cfgs)*
                #id => {
                    // Whether a number the page would not get as it is was
                    // reported for this method.
                    static NUMBER_REPORTED: ::std::sync::atomic::AtomicBool =
                        ::std::sync::atomic::AtomicBool::new(false);
                    #private::check_argument_count(method, #count, args)?;
                    #(#reads)*
                    let result = self.#ident(#(#names),*);
                    // A `Result` resolves the page's promise with its value
                    // or rejects it with the error's text, any other value
                    // resolves it.
                    #[allow(unused_imports)]
                    use #private::{AnswerResult as _, AnswerValue as _};
                    (&#private::Answer(result)).answer(method, &NUMBER_REPORTED)
                }
            }
        });

    let event_arms = marked
        .iter()
        .filter(|method| matches!(method.kind, Kind::PageEvent))
        .map(|event| {
            let id = &event.id;
            let ident = &event.ident;
            let cfgs = &event.cfgs;
            match event.arguments.first() {
                Some(ty) => quote! {
                    #(#cfgs)*
                    #id => {
                        if let ::std::option::Option::Some(data) = #private::page_event_data::<#ty>(name, data) {
                            self.#ident(data);
                        }
                    }
                },
                None => quote! {
                    #(#cfgs)*
                    #id => self.#ident(),
                },
            }
        });

    let generics = &block.generics;
    let probe = |ty: &syn::Type| {
        let rust_type = quote!(::std::any::type_name::<#ty>());
        let type_parameter = match named_type_parameter(ty, generics) {
            Some(name) => quote!(::core::option::Option::Some(#name)),
            None => quote!(::core::option::Option::None),
        };
        let type_script = quote! {
            |types: &mut ::beamer::webview::type_script::TypeScriptTypes| {
                #[allow(unused_imports)]
                use ::beamer::webview::type_script::__private::{HasTypeScript as _, LacksTypeScript as _};
                (&::beamer::webview::type_script::__private::TypeProbe::<#ty>::new()).type_script_if_any(types)
            }
        };
        quote!(#rust_type, #type_parameter, #type_script)
    };
    let unit: syn::Type = syn::parse_quote!(());
    let descriptions = marked.iter().map(|method| {
        let id = &method.id;
        let cfgs = &method.cfgs;
        let description = match method.kind {
            Kind::Method => {
                let arguments = method.arguments.iter().map(|ty| {
                    let probe = probe(ty);
                    quote!(.argument(#probe))
                });
                let result = probe(&method.result);
                quote!(types.method(#id) #(#arguments)* .result(#result);)
            }
            Kind::PageEvent => {
                let probe = probe(method.arguments.first().unwrap_or(&unit));
                quote!(types.page_event(::std::string::String::from(#id), #probe);)
            }
        };
        quote! {
            #(#cfgs)*
            {
                #description
            }
        }
    });

    quote! {
        impl #impl_generics ::beamer::webview::WebViewHandler for #self_ty #where_clause {
            fn on_invoke(
                &self,
                method: &str,
                args: &[#value],
            ) -> ::std::result::Result<#value, ::std::string::String> {
                match method {
                    #(#method_arms)*
                    _ => ::std::result::Result::Err(#private::unknown_method(method)),
                }
            }

            #[allow(unused_variables)]
            fn on_event(&self, name: &str, data: &#value) {
                match name {
                    #(#event_arms)*
                    _ => {}
                }
            }

            fn describe_handler(types: &mut ::beamer::webview::ModelTypes) {
                types.handler();
                #(#descriptions)*
            }
        }
    }
}
