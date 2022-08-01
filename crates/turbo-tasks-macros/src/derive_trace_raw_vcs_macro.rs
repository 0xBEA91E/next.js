use proc_macro::TokenStream;
use proc_macro2::{Ident, Literal};
use quote::quote;
use syn::{
    parse_macro_input, spanned::Spanned, Field, Fields, FieldsNamed, FieldsUnnamed, Item, ItemEnum,
    ItemStruct,
};

pub fn derive_trace_raw_vcs(input: TokenStream) -> TokenStream {
    fn ignore_field(field: &Field) -> bool {
        field
            .attrs
            .iter()
            .any(|attr| attr.path.is_ident("trace_ignore"))
    }

    let item = parse_macro_input!(input as Item);

    let (ident, generics, trace_items) = match &item {
        Item::Enum(ItemEnum {
            ident,
            generics,
            variants,
            ..
        }) => (ident, generics, {
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
                                    turbo_tasks::trace::TraceRawVcs::trace_raw_vcs(#idents, context);
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
                            .filter(|ident| &**ident != "_")
                            .collect();
                        quote! {
                            #ident::#variant_ident( #(#idents),* ) => {
                                #(
                                    turbo_tasks::trace::TraceRawVcs::trace_raw_vcs(#active_idents, context);
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
        Item::Struct(ItemStruct {
            ident,
            generics,
            fields,
            ..
        }) => (
            ident,
            generics,
            match fields {
                Fields::Named(FieldsNamed { named, .. }) => {
                    let idents: Vec<_> = named
                        .iter()
                        .filter(|field| !ignore_field(field))
                        .filter_map(|field| field.ident.clone())
                        .collect();
                    quote! {
                        #(
                            turbo_tasks::trace::TraceRawVcs::trace_raw_vcs(&self.#idents, context);
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
                            turbo_tasks::trace::TraceRawVcs::trace_raw_vcs(&self.#indicies, context);
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
    let generics_params = &generics.params.iter().collect::<Vec<_>>();
    quote! {
        impl #generics turbo_tasks::trace::TraceRawVcs for #ident #generics #(where #generics_params: turbo_tasks::trace::TraceRawVcs)* {
            fn trace_raw_vcs(&self, context: &mut turbo_tasks::trace::TraceRawVcsContext) {
                #trace_items
            }
        }
    }
    .into()
}
