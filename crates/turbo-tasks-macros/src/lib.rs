#![feature(proc_macro_diagnostic)]
#![feature(allow_internal_unstable)]

extern crate proc_macro;

use proc_macro::TokenStream;
use proc_macro2::{Ident, Literal, TokenStream as TokenStream2};
use quote::quote;
use syn::{
    parenthesized,
    parse::{Parse, ParseStream},
    parse_macro_input, parse_quote,
    punctuated::Punctuated,
    spanned::Spanned,
    token::Paren,
    Attribute, Error, Expr, Field, Fields, FieldsNamed, FieldsUnnamed, FnArg, ImplItem,
    ImplItemMethod, Item, ItemEnum, ItemFn, ItemImpl, ItemStruct, ItemTrait, Pat, PatIdent,
    PatType, Path, PathArguments, PathSegment, Receiver, Result, ReturnType, Signature, Token,
    TraitItem, TraitItemMethod, Type, TypePath, TypeTuple, AngleBracketedGenericArguments, GenericArgument, TypeReference, 
};

fn get_ref_ident(ident: &Ident) -> Ident {
    Ident::new(&(ident.to_string() + "Ref"), ident.span())
}

fn get_internal_function_ident(ident: &Ident) -> Ident {
    Ident::new(&(ident.to_string() + "_inline"), ident.span())
}

fn get_internal_trait_impl_function_ident(trait_ident: &Ident, ident: &Ident) -> Ident {
    Ident::new(
        &("__trait_call_".to_string() + &trait_ident.to_string() + "_" + &ident.to_string()),
        ident.span(),
    )
}

fn get_trait_mod_ident(ident: &Ident) -> Ident {
    Ident::new(&(ident.to_string() + "TurboTasksMethods"), ident.span())
}

fn get_slot_value_type_ident(ident: &Ident) -> Ident {
    Ident::new(
        &(ident.to_string().to_uppercase() + "_NODE_TYPE"),
        ident.span(),
    )
}

fn get_trait_type_ident(ident: &Ident) -> Ident {
    Ident::new(
        &(ident.to_string().to_uppercase() + "_TRAIT_TYPE"),
        ident.span(),
    )
}

fn get_register_trait_methods_ident(trait_ident: &Ident, struct_ident: &Ident) -> Ident {
    Ident::new(
        &("__register_".to_string()
            + &struct_ident.to_string()
            + "_"
            + &trait_ident.to_string()
            + "_trait_methods"),
        trait_ident.span(),
    )
}

fn get_function_ident(ident: &Ident) -> Ident {
    Ident::new(
        &(ident.to_string().to_uppercase() + "_FUNCTION"),
        ident.span(),
    )
}

fn get_trait_impl_function_ident(struct_ident: &Ident, ident: &Ident) -> Ident {
    Ident::new(
        &(struct_ident.to_string().to_uppercase()
            + "_IMPL_"
            + &ident.to_string().to_uppercase()
            + "_FUNCTION"),
        ident.span(),
    )
}

enum IntoMode {
    None,
    New,
    Shared,

    // TODO remove that
    Value
}

impl Parse for IntoMode {
    fn parse(input: ParseStream) -> Result<Self> {
        let ident = input.parse::<Ident>()?;
        match ident.to_string().as_str() {
            "none" => Ok(IntoMode::None),
            "new" => Ok(IntoMode::New),
            "shared" => Ok(IntoMode::Shared),
            "value" => Ok(IntoMode::Value),
            _ => {
                return Err(Error::new_spanned(
                    &ident,
                    format!("unexpected {}, expected \"none\", \"new\", \"shared\" or \"value\"", ident.to_string()),
                ))
            },
        }
    }
}

struct ValueArguments {
    traits: Vec<Ident>,
    into_mode: IntoMode,
    slot_mode: IntoMode,
}

impl Parse for ValueArguments {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut result = ValueArguments { traits: Vec::new(), into_mode: IntoMode::None, slot_mode: IntoMode::Shared };
        if input.is_empty() {
            return Ok(result);
        }
        loop {
            let ident = input.parse::<Ident>()?;
            match ident.to_string().as_str() {
                "value" => {
                    result.into_mode = IntoMode::Value;
                    result.slot_mode = IntoMode::Value;
                },
                "shared" => {
                    result.into_mode = IntoMode::Shared;
                    result.slot_mode = IntoMode::Shared;
                },
                "into" => {
                    input.parse::<Token![:]>()?;
                    result.into_mode = input.parse::<IntoMode>()?;
                },
                "slot" => {
                    input.parse::<Token![:]>()?;
                    result.slot_mode = input.parse::<IntoMode>()?;
                },
                _ => {
                    result.traits.push(ident);
                    while input.peek(Token![+]) {
                        input.parse::<Token![+]>()?;
                        let ident = input.parse::<Ident>()?;
                        result.traits.push(ident);
                    }
                }
            }
            if input.is_empty() {
                return Ok(result);
            } else if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            } else {
                return Err(input.error("expected \",\" or end of attribute"));
            }
        }
    }
}

/// Creates a ValueRef struct for a `struct` or `enum` that represent
/// that type placed into a slot in a [Task].
/// 
/// That ValueRef object can be `.await?`ed to get a readonly reference
/// to the original value.
/// 
/// `into` argument (`#[turbo_tasks::value(into: xxx)]`)
/// 
/// When provided the ValueRef implement `From<Value>` to allow to convert
/// a Value to a ValueRef by placing it into a slot in a Task.
/// 
/// `into: new`: Always overrides the value in the slot. Invalidating all dependent tasks.
/// 
/// `into: shared`: Compares with the existing value in the slot, before overriding it.
/// Requires Value to implement [Eq].
/// 
/// TODO: add more documentation: presets, traits
#[allow_internal_unstable(into_future, trivial_bounds)]
#[proc_macro_attribute]
pub fn value(args: TokenStream, input: TokenStream) -> TokenStream {
    let item = parse_macro_input!(input as Item);
    let ValueArguments { traits, into_mode, slot_mode } = parse_macro_input!(args as ValueArguments);

    let (vis, ident) = match &item {
        Item::Enum(ItemEnum { vis, ident, .. }) => (vis, ident),
        Item::Struct(ItemStruct { vis, ident, .. }) => (vis, ident),
        _ => {
            item.span().unwrap().error("unsupported syntax").emit();

            return quote! {
                #item
            }
            .into();
        }
    };

    let ref_ident = get_ref_ident(&ident);
    let slot_value_type_ident = get_slot_value_type_ident(&ident);
    let trait_refs: Vec<_> = traits.iter().map(|ident| get_ref_ident(&ident)).collect();

    let into = match into_mode {
        IntoMode::None => quote! {} ,
        IntoMode::New => quote! {
            impl From<#ident> for #ref_ident {
                fn from(content: #ident) -> Self {
                    Self { node: turbo_tasks::macro_helpers::match_previous_node_by_type::<#ident, _>(
                        |__slot| {
                            __slot.update_shared(&#slot_value_type_ident, content);
                        }
                    ) }
                }
            }
            
            #(impl From<#ident> for #trait_refs {
                fn from(content: #ident) -> Self {
                    std::convert::From::<turbo_tasks::SlotRef>::from(turbo_tasks::macro_helpers::match_previous_node_by_type::<dyn #traits, _>(
                        |__slot| {
                            __slot.update_shared(&#slot_value_type_ident, content);
                        }
                    ))
                }
            })*
        },
        IntoMode::Value => quote! {
            impl From<#ident> for #ref_ident {
                fn from(content: #ident) -> Self {
                    Self { node: turbo_tasks::macro_helpers::match_previous_node_by_type::<#ident, _>(
                        |__slot| {
                            __slot.compare_and_update_cloneable(&#slot_value_type_ident, content);
                        }
                    ) }
                }
            }
            
            #(impl From<#ident> for #trait_refs {
                fn from(content: #ident) -> Self {
                    std::convert::From::<turbo_tasks::SlotRef>::from(turbo_tasks::macro_helpers::match_previous_node_by_type::<dyn #traits, _>(
                        |__slot| {
                            __slot.compare_and_update_cloneable(&#slot_value_type_ident, content);
                        }
                    ))
                }
            })*
        },
        IntoMode::Shared => quote! {
            impl From<#ident> for #ref_ident {
                fn from(content: #ident) -> Self {
                    Self { node: turbo_tasks::macro_helpers::match_previous_node_by_type::<#ident, _>(
                        |__slot| {
                            __slot.compare_and_update_shared(&#slot_value_type_ident, content);
                        }
                    ) }
                }
            }

            #(impl From<#ident> for #trait_refs {
                fn from(content: #ident) -> Self {
                    std::convert::From::<turbo_tasks::SlotRef>::from(turbo_tasks::macro_helpers::match_previous_node_by_type::<dyn #traits, _>(
                        |__slot| {
                            __slot.compare_and_update_shared(&#slot_value_type_ident, content);
                        }
                    ))
                }
            })*
        },
    };

    let slot = match slot_mode {
        IntoMode::None => quote! {} ,
        IntoMode::New => quote! {
            /// Places a value in a slot of the current task.
            /// Overrides the current value. Doesn't check of equallity.
            ///
            /// Slot is selected based on the value type and call order of `slot`.
            fn slot(content: #ident) -> #ref_ident {
                #ref_ident { node: turbo_tasks::macro_helpers::match_previous_node_by_type::<#ident, _>(
                    |__slot| {
                        __slot.update_shared(&#slot_value_type_ident, content);
                    }
                ) }
            }

            /// Places a value in a slot of the current task.
            /// Overrides the current value. Doesn't check of equallity.
            ///
            /// Slot is selected by the provided `key`. `key` must not be used twice during the current task.
            fn keyed_slot<T: std::hash::Hash + std::cmp::PartialEq + std::cmp::Eq + Send + Sync + 'static>(key: T, content: #ident) -> #ref_ident {
                #ref_ident { node: turbo_tasks::macro_helpers::match_previous_node_by_key::<#ident, T, _>(
                    key,
                    |__slot| {
                        __slot.update_shared(&#slot_value_type_ident, content);
                    }
                ) }
            }
        },
        IntoMode::Value => quote! {
            fn slot(content: #ident) -> #ref_ident {
                #ref_ident { node: turbo_tasks::macro_helpers::match_previous_node_by_type::<#ident, _>(
                    |__slot| {
                        __slot.compare_and_update_cloneable(&#slot_value_type_ident, content);
                    }
                ) }
            }

            fn keyed_slot<T: std::hash::Hash + std::cmp::PartialEq + std::cmp::Eq + Send + Sync + 'static>(key: T, content: #ident) -> #ref_ident {
                #ref_ident { node: turbo_tasks::macro_helpers::match_previous_node_by_key::<#ident, T, _>(
                    key,
                    |__slot| {
                        __slot.compare_and_update_cloneable(&#slot_value_type_ident, content);
                    }
                ) }
            }
        },
        IntoMode::Shared => quote! {
            /// Places a value in a slot of the current task.
            /// If there is already a value in the slot it only overrides the value when
            /// it's not equal to the provided value. (Requires `Eq` trait to be implemented on the type.)
            ///
            /// Slot is selected based on the value type and call order of `slot`.
            fn slot(content: #ident) -> #ref_ident {
                #ref_ident { node: turbo_tasks::macro_helpers::match_previous_node_by_type::<#ident, _>(
                    |__slot| {
                        __slot.compare_and_update_shared(&#slot_value_type_ident, content);
                    }
                ) }
            }

            /// Places a value in a slot of the current task.
            /// If there is already a value in the slot it only overrides the value when
            /// it's not equal to the provided value. (Requires `Eq` trait to be implemented on the type.)
            ///
            /// Slot is selected by the provided `key`. `key` must not be used twice during the current task.
            fn keyed_slot<T: std::hash::Hash + std::cmp::PartialEq + std::cmp::Eq + Send + Sync + 'static>(key: T, content: #ident) -> #ref_ident {
                #ref_ident { node: turbo_tasks::macro_helpers::match_previous_node_by_key::<#ident, T, _>(
                    key,
                    |__slot| {
                        __slot.compare_and_update_shared(&#slot_value_type_ident, content);
                    }
                ) }
            }
        },
    };

    let trait_registrations: Vec<_> = traits
        .iter()
        .map(|trait_ident| {
            let register = get_register_trait_methods_ident(trait_ident, &ident);
            quote! {
                #register(&mut slot_value_type);
            }
        })
        .collect();
    let expanded = quote! {
        #[derive(turbo_tasks::trace::TraceSlotRefs)]
        #item

        lazy_static::lazy_static! {
            static ref #slot_value_type_ident: turbo_tasks::SlotValueType = {
                let mut slot_value_type = turbo_tasks::SlotValueType::new(std::any::type_name::<#ident>().to_string());
                #(#trait_registrations)*
                slot_value_type
            };
        }

        /// A reference to a value created by a turbo-tasks function.
        /// The type can either point to a slot in a [turbo_tasks::Task] or to the output of
        /// a [turbo_tasks::Task], which then transitively points to a slot again, or
        /// to an fatal execution error.
        /// 
        /// `.resolve().await?` can be used to resolve it until it points to a slot.
        /// This is useful when storing the reference somewhere or when comparing it with other references.
        /// 
        /// A reference is equal to another reference with it points to the same thing. No resolving is applied on comparision.
        #[derive(Clone, Debug, std::hash::Hash, std::cmp::Eq, std::cmp::PartialEq)]
        #vis struct #ref_ident {
            node: turbo_tasks::SlotRef,
        }

        impl #ref_ident {
            #slot

            /// Reads the value of the reference.
            /// 
            /// This is async and will rethrow any fatal error that happened during task execution.
            /// 
            /// Reading the value will make the current task depend on the slot and the task outputs.
            /// This will lead to invalidation of the current task when one of these changes.
            pub async fn get(&self) -> turbo_tasks::Result<turbo_tasks::SlotRefReadResult<#ident>> {
                self.node.clone().into_read::<#ident>().await
            }

            /// Resolve the reference until it points to a slot directly.
            /// 
            /// This is async and will rethrow any fatal error that happened during task execution.
            pub async fn resolve(self) -> turbo_tasks::Result<Self> {
                Ok(Self { node: self.node.resolve().await? })
            }
        }

        // #[cfg(feature = "into_future")]
        impl std::future::IntoFuture for #ref_ident {
            type Output = turbo_tasks::Result<turbo_tasks::macro_helpers::SlotRefReadResult<#ident>>;
            type IntoFuture = std::pin::Pin<std::boxed::Box<dyn std::future::Future<Output = turbo_tasks::Result<turbo_tasks::macro_helpers::SlotRefReadResult<#ident>>> + Send + Sync + 'static>>;
            fn into_future(self) -> Self::IntoFuture {
                Box::pin(self.node.clone().into_read::<#ident>())
            }
        }
                
        impl std::convert::TryFrom<&turbo_tasks::TaskInput> for #ref_ident {
            type Error = turbo_tasks::Error;

            fn try_from(value: &turbo_tasks::TaskInput) -> Result<Self, Self::Error> {
                Ok(Self { node: value.try_into()? })
            }
        }

        impl From<turbo_tasks::SlotRef> for #ref_ident {
            fn from(node: turbo_tasks::SlotRef) -> Self {
                Self { node }
            }
        }

        impl From<#ref_ident> for turbo_tasks::SlotRef {
            fn from(node_ref: #ref_ident) -> Self {
                node_ref.node
            }
        }

        impl From<&#ref_ident> for turbo_tasks::SlotRef {
            fn from(node_ref: &#ref_ident) -> Self {
                node_ref.node.clone()
            }
        }

        impl From<#ref_ident> for turbo_tasks::TaskInput {
            fn from(node_ref: #ref_ident) -> Self {
                node_ref.node.into()
            }
        }

        impl From<&#ref_ident> for turbo_tasks::TaskInput {
            fn from(node_ref: &#ref_ident) -> Self {
                node_ref.node.clone().into()
            }
        }

        #(impl From<#ref_ident> for #trait_refs {
            fn from(node_ref: #ref_ident) -> Self {
                std::convert::From::<turbo_tasks::SlotRef>::from(node_ref.into())
            }
        })*

        #into

        impl turbo_tasks::trace::TraceSlotRefs for #ref_ident {
            fn trace_node_refs(&self, context: &mut turbo_tasks::trace::TraceSlotRefsContext) {
                turbo_tasks::trace::TraceSlotRefs::trace_node_refs(&self.node, context);
            }
        }
    };

    expanded.into()
}

enum Constructor {
    Default,
    Compare(Option<Ident>),
    CompareEnum(Option<Ident>),
    KeyAndCompare(Option<Expr>, Option<Ident>),
    KeyAndCompareEnum(Option<Expr>, Option<Ident>),
    Key(Option<Expr>),
}

impl Parse for Constructor {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut result = Constructor::Default;
        if input.is_empty() {
            return Ok(result);
        }
        let content;
        parenthesized!(content in input);
        loop {
            let ident = content.parse::<Ident>()?;
            match ident.to_string().as_str() {
                "compare" => {
                    let compare_name = if content.peek(Token![:]) {
                        content.parse::<Token![:]>()?;
                        Some(content.parse::<Ident>()?)
                    } else {
                        None
                    };
                    result = match result {
                        Constructor::Default => Constructor::Compare(compare_name),
                        Constructor::Key(key_expr) => {
                            Constructor::KeyAndCompare(key_expr, compare_name)
                        }
                        _ => {
                            return Err(content.error(format!(
                                "\"compare\" can't be combined with previous values"
                            )));
                        }
                    }
                }
                "compare_enum" => {
                    let compare_name = if content.peek(Token![:]) {
                        content.parse::<Token![:]>()?;
                        Some(content.parse::<Ident>()?)
                    } else {
                        None
                    };
                    result = match result {
                        Constructor::Default => Constructor::CompareEnum(compare_name),
                        Constructor::Key(key_expr) => {
                            Constructor::KeyAndCompareEnum(key_expr, compare_name)
                        }
                        _ => {
                            return Err(content.error(format!(
                                "\"compare\" can't be combined with previous values"
                            )));
                        }
                    }
                }
                "key" => {
                    let key_expr = if content.peek(Token![:]) {
                        content.parse::<Token![:]>()?;
                        Some(content.parse::<Expr>()?)
                    } else {
                        None
                    };
                    result = match result {
                        Constructor::Default => Constructor::Key(key_expr),
                        Constructor::Compare(compare_name) => {
                            Constructor::KeyAndCompare(key_expr, compare_name)
                        }
                        _ => {
                            return Err(content
                                .error(format!("\"key\" can't be combined with previous values")));
                        }
                    };
                }
                _ => {
                    return Err(Error::new_spanned(
                        &ident,
                        format!("unexpected {}, expected \"key\", \"compare\" or \"compare_enum\"", ident.to_string()),
                    ))
                }
            }
            if content.is_empty() {
                return Ok(result);
            } else if content.peek(Token![,]) {
                content.parse::<Token![,]>()?;
            } else {
                return Err(content.error("expected \",\" or end of attribute"));
            }
        }
    }
}

fn is_constructor(attr: &Attribute) -> bool {
    is_attribute(attr, "constructor")
}

fn is_attribute(attr: &Attribute, name: &str) -> bool {
    let path = &attr.path;
    if path.leading_colon.is_some() {
        return false;
    }
    let mut iter = path.segments.iter();
    match iter.next() {
        Some(seg) if seg.arguments.is_empty() && seg.ident.to_string() == "turbo_tasks" => {
            match iter.next() {
                Some(seg) if seg.arguments.is_empty() && seg.ident.to_string() == name => {
                    iter.next().is_none()
                }
                _ => false,
            }
        }
        _ => false,
    }
}

#[proc_macro_attribute]
pub fn value_trait(_args: TokenStream, input: TokenStream) -> TokenStream {
    let item = parse_macro_input!(input as ItemTrait);

    let ItemTrait {
        vis, ident, items, ..
    } = &item;

    let ref_ident = get_ref_ident(&ident);
    let mod_ident = get_trait_mod_ident(&ident);
    let trait_type_ident = get_trait_type_ident(&ident);
    let mut trait_fns = Vec::new();

    for item in items.iter() {
        if let TraitItem::Method(TraitItemMethod {
            sig:
                Signature {
                    ident: method_ident,
                    inputs,
                    output,
                    ..
                },
            ..
        }) = item
        {
            let output_type = get_return_type(&output);
            let args = inputs.iter().filter_map(|arg| match arg {
                FnArg::Receiver(_) => None,
                FnArg::Typed(PatType { pat, .. }) => Some(quote! {
                    #pat.into()
                }),
            });
            let method_args: Vec<_> = inputs.iter().collect();
            let convert_result_code = if is_empty_type(&output_type) {
                quote! {}
            } else {
                quote! { std::convert::From::<turbo_tasks::SlotRef>::from(result) }
            };
            trait_fns.push(quote! {
                pub fn #method_ident(#(#method_args),*) -> #output_type {
                    // TODO use const string
                    let result = turbo_tasks::trait_call(&#trait_type_ident, stringify!(#method_ident).to_string(), vec![self.into(), #(#args),*]);
                    #convert_result_code
                }
            })
        }
    }

    let expanded = quote! {
        #item

        lazy_static::lazy_static! {
            pub static ref #trait_type_ident: turbo_tasks::TraitType = turbo_tasks::TraitType::new(std::any::type_name::<dyn #ident>().to_string());
        }

        #vis struct #mod_ident {
            __private: ()
        }

        impl #mod_ident {
            #[inline]
            pub fn __type(&self) -> &'static turbo_tasks::TraitType {
                &*#trait_type_ident
            }
        }

        #[allow(non_upper_case_globals)]
        #vis static #ident: #mod_ident = #mod_ident { __private: () };

        #[derive(Clone, Debug, std::hash::Hash, std::cmp::Eq, std::cmp::PartialEq)]
        #vis struct #ref_ident {
            node: turbo_tasks::SlotRef,
        }

        impl #ref_ident {
            pub async fn resolve(self) -> turbo_tasks::Result<Self> {
                Ok(Self { node: self.node.resolve().await? })
            }

            #(#trait_fns)*
        }

                        
        impl std::convert::TryFrom<&turbo_tasks::TaskInput> for #ref_ident {
            type Error = turbo_tasks::Error;

            fn try_from(value: &turbo_tasks::TaskInput) -> Result<Self, Self::Error> {
                Ok(Self { node: value.try_into()? })
            }
        }
        
        impl From<turbo_tasks::SlotRef> for #ref_ident {
            fn from(node: turbo_tasks::SlotRef) -> Self {
                Self { node }
            }
        }

        impl From<#ref_ident> for turbo_tasks::SlotRef {
            fn from(node_ref: #ref_ident) -> Self {
                node_ref.node
            }
        }

        impl From<&#ref_ident> for turbo_tasks::SlotRef {
            fn from(node_ref: &#ref_ident) -> Self {
                node_ref.node.clone()
            }
        }
        
        impl From<#ref_ident> for turbo_tasks::TaskInput {
            fn from(node_ref: #ref_ident) -> Self {
                node_ref.node.into()
            }
        }
        
        impl From<&#ref_ident> for turbo_tasks::TaskInput {
            fn from(node_ref: &#ref_ident) -> Self {
                node_ref.node.clone().into()
            }
        }

        impl turbo_tasks::trace::TraceSlotRefs for #ref_ident {
            fn trace_node_refs(&self, context: &mut turbo_tasks::trace::TraceSlotRefsContext) {
                turbo_tasks::trace::TraceSlotRefs::trace_node_refs(&self.node, context);
            }
        }

    };
    expanded.into()
}

#[proc_macro_attribute]
pub fn value_impl(_args: TokenStream, input: TokenStream) -> TokenStream {
    fn generate_for_self_impl(ident: &Ident, items: &Vec<ImplItem>) -> TokenStream2 {
        let ref_ident = get_ref_ident(&ident);
        let slot_value_type_ident = get_slot_value_type_ident(&ident);
        let mut constructors = Vec::new();
        let mut i = 0;
        for item in items.iter() {
            match item {
                ImplItem::Method(ImplItemMethod {
                    attrs,
                    vis,
                    defaultness,
                    sig,
                    block: _,
                }) => {
                    if let Some(Attribute { tokens, .. }) =
                        attrs.iter().find(|attr| is_constructor(attr))
                    {
                        let constructor: Constructor = parse_quote! { #tokens };
                        let fn_name = &sig.ident;
                        let inputs = &sig.inputs;
                        let mut input_names = Vec::new();
                        let mut old_input_names = Vec::new();
                        let mut input_names_ref = Vec::new();
                        let index_literal = Literal::i32_unsuffixed(i);
                        let mut inputs_for_intern_key = vec![quote! { #index_literal }];
                        for arg in inputs.iter() {
                            if let FnArg::Typed(PatType { pat, ty, .. }) = arg {
                                if let Pat::Ident(PatIdent { ident, .. }) = &**pat {
                                    input_names.push(ident.clone());
                                    old_input_names.push(Ident::new(
                                        &(ident.to_string() + "_old"),
                                        ident.span(),
                                    ));
                                    if let Type::Reference(_) = &**ty {
                                        inputs_for_intern_key
                                            .push(quote! { std::clone::Clone::clone(#ident) });
                                        input_names_ref.push(quote! { #ident });
                                    } else {
                                        inputs_for_intern_key
                                            .push(quote! { std::clone::Clone::clone(&#ident) });
                                        input_names_ref.push(quote! { &#ident });
                                    }
                                } else {
                                    item.span()
                                        .unwrap()
                                        .error(format!(
                                            "unsupported pattern syntax in {}: {}",
                                            &ident.to_string(),
                                            quote! { #pat }
                                        ))
                                        .emit();
                                }
                            }
                        }
                        let create_new_content = quote! {
                            #ident::#fn_name(#(#input_names),*)
                        };
                        let gen_conditional_update_functor = |compare_name| {
                            let compare = match compare_name {
                                Some(name) => quote! {
                                    __self.#name(#(#input_names_ref),*)
                                },
                                None => quote! {
                                    true #(&& (#input_names_ref == &__self.#input_names))*
                                },
                            };
                            quote! {
                                |__slot| {
                                    __slot.conditional_update_shared::<#ident, _>(&#slot_value_type_ident, |__self| {
                                        if let Some(__self) = __self {
                                            if #compare {
                                                return None;
                                            }
                                        }
                                        Some(#create_new_content)
                                    })
                                }
                            }
                        };
                        let gen_compare_enum_functor = |name| {
                            let compare = if old_input_names.is_empty() {
                                quote! {
                                    if __self == Some(&#ident::#name) {
                                        return None
                                    }
                                }
                            } else {
                                quote! {
                                    if let Some(&#ident::#name(ref #(#old_input_names),*)) = __self {
                                        if true #(&& (#input_names == *#old_input_names))* {
                                            return None
                                        }
                                    }
                                }
                            };
                            quote! {
                                |__slot| {
                                    __slot.conditional_update_shared::<#ident, _>(&#slot_value_type_ident, |__self| {
                                        #compare
                                        Some(#create_new_content)
                                    })
                                }
                            }
                        };
                        let get_node = match constructor {
                            Constructor::Default => {
                                quote! {
                                    turbo_tasks::macro_helpers::match_previous_node_by_type::<#ident, _>(
                                        |__slot| {
                                            __slot.update_shared::<#ident>(&#slot_value_type_ident, #create_new_content);
                                        }
                                    )
                                }
                            }
                            Constructor::Compare(compare_name) => {
                                let functor = gen_conditional_update_functor(compare_name);
                                quote! {
                                    turbo_tasks::macro_helpers::match_previous_node_by_type::<#ident, _>(
                                        #functor
                                    )
                                }
                            }
                            Constructor::KeyAndCompare(key_expr, compare_name) => {
                                let functor = gen_conditional_update_functor(compare_name);
                                quote! {
                                    turbo_tasks::macro_helpers::match_previous_node_by_key::<#ident, _, _>(
                                        #key_expr,
                                        #functor
                                    )
                                }
                            }
                            Constructor::CompareEnum(compare_name) => {
                                let functor = gen_compare_enum_functor(compare_name);
                                quote! {
                                    turbo_tasks::macro_helpers::match_previous_node_by_type::<#ident, _>(
                                        #functor
                                    )
                                }
                            }
                            Constructor::KeyAndCompareEnum(key_expr, compare_name) => {
                                let functor = gen_compare_enum_functor(compare_name);
                                quote! {
                                    turbo_tasks::macro_helpers::match_previous_node_by_key::<#ident, _, _>(
                                        #key_expr,
                                        #functor
                                    )
                                }
                            }
                            Constructor::Key(_) => todo!(),
                        };
                        constructors.push(quote! {
                            #(#attrs)*
                            #vis #defaultness #sig {
                                let node = #get_node;
                                Self {
                                    node
                                }
                            }
                        });
                        i += 1;
                    }
                }
                _ => {}
            };
        }

        return quote! {
            impl #ref_ident {
                #(#constructors)*
            }
        };
    }

    fn generate_for_self_ref_impl(ref_ident: &Ident, items: &Vec<ImplItem>) -> TokenStream2 {
        let mut functions = Vec::new();

        for item in items.iter() {
            match item {
                ImplItem::Method(ImplItemMethod {
                    attrs,
                    vis,
                    defaultness: _,
                    sig,
                    block,
                }) => {
                    let Signature { ident, output, .. } = sig;

                    let output_type = get_return_type(output);
                    let inline_ident = get_internal_function_ident(ident);
                    let function_ident = get_trait_impl_function_ident(ref_ident, ident);

                    let mut inline_sig = sig.clone();
                    inline_sig.ident = inline_ident.clone();

                    let mut external_sig = sig.clone();
                    external_sig.asyncness = None;

                    let (native_function_code, input_slot_ref_arguments) = gen_native_function_code(
                        // use const string
                        quote! { stringify!(#ref_ident::#ident) },
                        quote! { #ref_ident::#inline_ident },
                        &function_ident,
                        sig.asyncness.is_some(),
                        &sig.inputs,
                        &output_type,
                        Some(ref_ident),
                        true,
                    );

                    let (raw_output_type, _) = unwrap_result_type(&output_type);
                    let convert_result_code = if is_empty_type(&raw_output_type) {
                        external_sig.output = ReturnType::Default;
                        quote! {}
                    } else {
                        external_sig.output = ReturnType::Type(Token![->](raw_output_type.span()), Box::new(raw_output_type.clone()));
                        quote! { std::convert::From::<turbo_tasks::SlotRef>::from(result) }
                    };


                    functions.push(quote! {
                        impl #ref_ident {
                            #(#attrs)*
                            #vis #external_sig {
                                let result = turbo_tasks::dynamic_call(&#function_ident, vec![#(#input_slot_ref_arguments),*]);
                                #convert_result_code
                            }

                            #(#attrs)*
                            #vis #inline_sig #block
                        }

                        #native_function_code
                    })
                }
                _ => {}
            }
        }

        return quote! {
            #(#functions)*
        };
    }

    fn generate_for_trait_impl(
        trait_ident: &Ident,
        struct_ident: &Ident,
        items: &Vec<ImplItem>,
    ) -> TokenStream2 {
        let register = get_register_trait_methods_ident(trait_ident, struct_ident);
        let ref_ident = get_ref_ident(struct_ident);
        let mut trait_registers = Vec::new();
        let mut impl_functions = Vec::new();
        let mut trait_functions = Vec::new();
        for item in items.iter() {
            match item {
                ImplItem::Method(ImplItemMethod {
                    sig, attrs, block, ..
                }) => {
                    let Signature {
                        ident,
                        inputs,
                        output,
                        asyncness,
                        ..
                    } = sig;
                    let output_type = get_return_type(output);
                    let function_ident = get_trait_impl_function_ident(struct_ident, ident);
                    let internal_function_ident =
                        get_internal_trait_impl_function_ident(trait_ident, ident);
                    trait_registers.push(quote! {
                        slot_value_type.register_trait_method(#trait_ident.__type(), stringify!(#ident).to_string(), &*#function_ident);
                    });
                    let name =
                        Literal::string(&(struct_ident.to_string() + "::" + &ident.to_string()));
                    let (native_function_code, input_slot_ref_arguments) = gen_native_function_code(
                        quote! { #name },
                        quote! { #struct_ident::#internal_function_ident },
                        &function_ident,
                        asyncness.is_some(),
                        inputs,
                        &output_type,
                        Some(&ref_ident),
                        false,
                    );
                    let mut new_sig = sig.clone();
                    new_sig.ident = internal_function_ident;
                    let mut external_sig = sig.clone();
                    external_sig.asyncness = None;
                    impl_functions.push(quote! {
                        impl #struct_ident {
                            #(#attrs)*
                            #[allow(non_snake_case)]
                            #new_sig #block
                        }

                        #native_function_code
                    });

                    let (raw_output_type, _) = unwrap_result_type(&output_type);
                    let convert_result_code = if is_empty_type(&raw_output_type) {
                        external_sig.output = ReturnType::Default;
                        quote! {}
                    } else {
                        external_sig.output = ReturnType::Type(Token![->](raw_output_type.span()), Box::new(raw_output_type.clone()));
                        quote! { std::convert::From::<turbo_tasks::SlotRef>::from(result) }
                    };

                    trait_functions.push(quote!{
                        #(#attrs)*
                        #external_sig {
                            let result = turbo_tasks::dynamic_call(&#function_ident, vec![#(#input_slot_ref_arguments),*]);
                            #convert_result_code                
                        }
                    });
                }
                _ => {}
            }
        }
        quote! {
            #[allow(non_snake_case)]
            fn #register(slot_value_type: &mut turbo_tasks::SlotValueType) {
                slot_value_type.register_trait(#trait_ident.__type());
                #(#trait_registers)*
            }

            #(#impl_functions)*

            impl #trait_ident for #ref_ident {
                #(#trait_functions)*
            }
        }
    }

    let item = parse_macro_input!(input as ItemImpl);

    if let Type::Path(TypePath {
        qself: None,
        path: Path { segments, .. },
    }) = &*item.self_ty
    {
        if segments.len() == 1 {
            if let Some(PathSegment {
                arguments: PathArguments::None,
                ident,
            }) = segments.first()
            {
                match &item.trait_ {
                    None => {
                        if ident.to_string().ends_with("Ref") {
                            let code = generate_for_self_ref_impl(ident, &item.items);
                            return quote! {
                                #code
                            }
                            .into();
                        } else {
                            let code = generate_for_self_impl(ident, &item.items);
                            return quote! {
                                #item

                                #code
                            }
                            .into();
                        }
                    }
                    Some((_, Path { segments, .. }, _)) => {
                        if segments.len() == 1 {
                            if let Some(PathSegment {
                                arguments: PathArguments::None,
                                ident: trait_ident,
                            }) = segments.first()
                            {
                                let code = generate_for_trait_impl(trait_ident, ident, &item.items);
                                return quote! {
                                    #code
                                }
                                .into();
                            }
                        }
                    }
                }
            }
        }
    }
    item.span().unwrap().error("unsupported syntax").emit();
    quote! {
        #item
    }
    .into()
}

fn get_return_type(output: &ReturnType) -> Type {
    match output {
        ReturnType::Default => Type::Tuple(TypeTuple {
            paren_token: Paren::default(),
            elems: Punctuated::new(),
        }),
        ReturnType::Type(_, ref output_type) => (**output_type).clone(),
    }
}

#[proc_macro_attribute]
pub fn function(_args: TokenStream, input: TokenStream) -> TokenStream {
    let item = parse_macro_input!(input as ItemFn);
    let ItemFn {
        attrs,
        vis,
        sig,
        block,
    } = &item;
    let output_type = get_return_type(&sig.output);
    let ident = &sig.ident;
    let function_ident = get_function_ident(ident);
    let inline_ident = get_internal_function_ident(ident);

    let mut inline_sig = sig.clone();
    inline_sig.ident = inline_ident.clone();

    let mut external_sig = sig.clone();
    external_sig.asyncness = None;

    let (native_function_code, input_slot_ref_arguments) = gen_native_function_code(
        quote! { stringify!(#ident) },
        quote! { #inline_ident },
        &function_ident,
        sig.asyncness.is_some(),
        &sig.inputs,
        &output_type,
        None,
        false,
    );

    let (raw_output_type, _) = unwrap_result_type(&output_type);
    let convert_result_code = if is_empty_type(&raw_output_type) {
        external_sig.output = ReturnType::Default;
        quote! {}
    } else {
        external_sig.output = ReturnType::Type(Token![->](raw_output_type.span()), Box::new(raw_output_type.clone()));
        quote! { std::convert::From::<turbo_tasks::SlotRef>::from(result) }
    };

    return quote! {
        #(#attrs)*
        #vis #external_sig {
            let result = turbo_tasks::dynamic_call(&#function_ident, vec![#(#input_slot_ref_arguments),*]);
            #convert_result_code
        }

        #(#attrs)*
        #vis #inline_sig #block

        #native_function_code
    }
    .into();
}

fn unwrap_result_type(ty: &Type) -> (&Type, bool) {
    if let Type::Path(TypePath { qself: None, path: Path { segments, .. } }) = ty {
        if let Some(PathSegment { arguments: PathArguments::AngleBracketed(AngleBracketedGenericArguments{ args, ..}), .. }) = segments.last() {
            if let Some(GenericArgument::Type(ty)) = args.first() {
                return (ty, true);
            }
        }
    }
    (ty, false)
}

fn is_empty_type(ty: &Type) -> bool {
    if let Type::Tuple(TypeTuple { elems, .. }) = ty {
        if elems.is_empty() {
            return true;
        }
    }
    false
}

fn gen_native_function_code(
    name_code: TokenStream2,
    original_function: TokenStream2,
    function_ident: &Ident,
    async_function: bool,
    inputs: &Punctuated<FnArg, Token![,]>,
    output_type: &Type,
    self_ref_type: Option<&Ident>,
    self_is_ref_type: bool,
) -> (TokenStream2, Vec<TokenStream2>) {
    let mut task_argument_options = Vec::new();
    let mut input_extraction = Vec::new();
    let mut input_convert = Vec::new();
    let mut input_clone = Vec::new();
    let mut input_final = Vec::new();
    let mut input_arguments = Vec::new();
    let mut input_slot_ref_arguments = Vec::new();

    let mut index: i32 = 1;

    for input in inputs {
        match input {
            FnArg::Receiver(Receiver { mutability, .. }) => {
                if mutability.is_some() {
                    input.span().unwrap().error("mutable self is not supported in turbo_task traits (nodes are immutable)").emit();
                }
                let self_ref_type = self_ref_type.unwrap();
                task_argument_options.push(quote! {
                    turbo_tasks::TaskArgumentOptions::Resolved
                });
                input_extraction.push(quote! {
                    let __self = __iter
                        .next()
                        .ok_or_else(|| anyhow::anyhow!(concat!(#name_code, "() self argument missing")))?;
                });
                input_convert.push(quote! {
                    let __self = std::convert::TryInto::<#self_ref_type>::try_into(__self)?;
                });
                input_clone.push(quote! {
                    let __self = std::clone::Clone::clone(&__self);
                });
                if self_is_ref_type {
                    input_final.push(quote! {});
                    input_arguments.push(quote! {
                        __self
                    });
                } else {
                    input_final.push(quote! {
                        let __self = __self.await?;
                    });
                    input_arguments.push(quote! {
                        &*__self
                    });
                }
                input_slot_ref_arguments.push(quote! {
                    self.into()
                });
            }
            FnArg::Typed(PatType { pat, ty, .. }) => {
                task_argument_options.push(quote! {
                    turbo_tasks::TaskArgumentOptions::Resolved
                });
                input_extraction.push(quote! {
                    let #pat = __iter
                        .next()
                        .ok_or_else(|| anyhow::anyhow!(concat!(#name_code, "() argument ", stringify!(#index), " (", stringify!(#pat), ") missing")))?;
                });
                input_final.push(quote! {
                });
                if let Type::Reference(TypeReference { and_token, lifetime: _, mutability, elem }) = &**ty {
                    let ty = if let Type::Path(TypePath { qself: None, path }) = &**elem {
                        if path.is_ident("str") {
                            quote! { String }
                        } else {
                            quote! { #elem }
                        }
                    } else {
                        quote! { #elem }
                    };
                    input_convert.push(quote! {
                        let #pat = std::convert::TryInto::<#ty>::try_into(#pat)?;
                    });
                    input_clone.push(quote! {
                        let #pat = std::clone::Clone::clone(&#pat);
                    });
                    input_arguments.push(quote! {
                        #and_token #mutability #pat
                    });
                } else {
                    input_convert.push(quote! {
                        let #pat = std::convert::TryInto::<#ty>::try_into(#pat)?;
                    });
                    input_clone.push(quote! {
                        let #pat = std::clone::Clone::clone(&#pat);
                    });
                    input_arguments.push(quote! {
                        #pat
                    });
                }
                input_slot_ref_arguments.push(quote! {
                    #pat.into()
                });
                index += 1;
            }
        }
    }
    let original_call_code = if async_function {
        quote! { #original_function(#(#input_arguments),*).await }
    } else {
        quote! { #original_function(#(#input_arguments),*) }
    };
    let (raw_output_type, is_result) = unwrap_result_type(output_type);
    let original_call_code = match (is_result, is_empty_type(raw_output_type)) {
        (true, true) => quote! {
            (#original_call_code).map(|_| turbo_tasks::NothingRef::new().into())
        },
        (true, false) => quote! { #original_call_code.map(|v| v.into()) },
        (false, true) => quote! {
            #original_call_code;
            Ok(turbo_tasks::NothingRef::new().into())
        },
        (false, false) => quote! { Ok(#original_call_code.into()) },
    };
    (
        quote! {
            lazy_static::lazy_static! {
                static ref #function_ident: turbo_tasks::NativeFunction = turbo_tasks::NativeFunction::new(#name_code.to_string(), vec![#(#task_argument_options),*], |inputs| {
                    let mut __iter = inputs.iter();
                    #(#input_extraction)*
                    if __iter.next().is_some() {
                        return Err(anyhow::anyhow!(concat!(#name_code, "() called with too many arguments")));
                    }
                    #(#input_convert)*
                    Ok(Box::new(move || {
                        #(#input_clone)*
                        Box::pin(async move {
                            #(#input_final)*
                            #original_call_code
                        })
                    }))
                });
            }
        },
        input_slot_ref_arguments,
    )
}

#[proc_macro_attribute]
pub fn constructor(_args: TokenStream, input: TokenStream) -> TokenStream {
    input
}

#[proc_macro_derive(TraceSlotRefs, attributes(trace_ignore))]
pub fn derive_trace_node_refs_attr(input: TokenStream) -> TokenStream {
    fn ignore_field(field: &Field) -> bool {
        field
            .attrs
            .iter()
            .any(|attr| attr.path.is_ident("trace_ignore"))
    }

    let item = parse_macro_input!(input as Item);

    let (ident, trace_items) = match &item {
        Item::Enum(ItemEnum {
            ident, variants, ..
        }) => (ident, {
            let variants_code: Vec<_> = variants.iter().map(|variant| {
                let variant_ident = &variant.ident;
                match &variant.fields {
                    Fields::Named(FieldsNamed{ named, ..}) => {
                        let idents: Vec<_> = named.iter()
                            .filter(|field| !ignore_field(field))
                            .filter_map(|field| field.ident.clone())
                            .collect();
                        let ident_pats: Vec<_> = named.iter()
                            .filter_map(|field| {
                                let ident = field.ident.as_ref()?;
                                if ignore_field(field) {
                                    Some(quote! { #ident: _ })
                                } else {
                                    Some(quote! { ref #ident })
                                }
                            })
                            .collect();
                        quote! {
                            #ident::#variant_ident{ #(#ident_pats),* } => {
                                #(
                                    turbo_tasks::trace::TraceSlotRefs::trace_node_refs(#idents, context);
                                )*
                            }
                        }
                    },
                    Fields::Unnamed(FieldsUnnamed{ unnamed, .. }) => {
                        let idents: Vec<_> = unnamed.iter()
                            .enumerate()
                            .map(|(i, field)| if ignore_field(field) {
                                Ident::new("_", field.span())
                            } else {
                                Ident::new(&format!("tuple_item_{}", i), field.span())
                            })
                            .collect();
                        let active_idents: Vec<_> = idents.iter()
                            .filter(|ident| ident.to_string() != "_")
                            .collect();
                        quote! {
                            #ident::#variant_ident( #(#idents),* ) => {
                                #(
                                    turbo_tasks::trace::TraceSlotRefs::trace_node_refs(#active_idents, context);
                                )*
                            }
                        }
                    },
                    Fields::Unit => quote! {
                        #ident::#variant_ident => {}
                    },
                }
            }).collect();
            quote! {
                match self {
                    #(#variants_code)*
                }
            }
        }),
        Item::Struct(ItemStruct { ident, fields, .. }) => (
            ident,
            match fields {
                Fields::Named(FieldsNamed { named, .. }) => {
                    let idents: Vec<_> = named
                        .iter()
                        .filter(|field| !ignore_field(field))
                        .filter_map(|field| field.ident.clone())
                        .collect();
                    quote! {
                        #(
                            turbo_tasks::trace::TraceSlotRefs::trace_node_refs(&self.#idents, context);
                        )*
                    }
                }
                Fields::Unnamed(FieldsUnnamed { unnamed, .. }) => {
                    let indicies: Vec<_> = unnamed
                        .iter()
                        .enumerate()
                        .filter(|(_, field)| !ignore_field(field))
                        .map(|(i, _)| Literal::usize_unsuffixed(i))
                        .collect();
                    quote! {
                        #(
                            turbo_tasks::trace::TraceSlotRefs::trace_node_refs(&self.#indicies, context);
                        )*
                    }
                }
                Fields::Unit => quote! {},
            },
        ),
        _ => {
            item.span().unwrap().error("unsupported syntax").emit();

            return quote! {}.into();
        }
    };
    quote! {
        impl turbo_tasks::trace::TraceSlotRefs for #ident {
            fn trace_node_refs(&self, context: &mut turbo_tasks::trace::TraceSlotRefsContext) {
                #trace_items
            }
        }
    }
    .into()
}
