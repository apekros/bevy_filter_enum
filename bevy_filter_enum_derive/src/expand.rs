//! Code generation for `#[derive(EnumFilter)]`.
//!
//! The output is deliberately thin: marker structs plus an `EnumFilter` impl
//! whose methods are per-variant `match`es that forward to generic functions
//! in `bevy_filter_enum::__private`. All lifecycle logic lives in the library
//! as normal, testable Rust.

use proc_macro2::TokenStream;
use quote::{ToTokens, quote, quote_spanned};
use syn::spanned::Spanned;

use crate::model::{FilterEnum, FilterVariant, Payload, Shape};

/// `Bundle` is implemented for tuples of up to this many elements.
const MAX_BUNDLE_TUPLE: usize = 15;

pub fn expand(model: &FilterEnum) -> TokenStream {
    let private = quote!(::bevy_filter_enum::__private);
    let ecs = quote!(#private::bevy_ecs);

    let markers = model
        .variants
        .iter()
        .map(|variant| expand_marker(model, variant));
    let payload_asserts = model.unique_payloads().into_iter().map(|payload| {
        let ty = &payload.ty;
        quote_spanned! {ty.span()=>
            const _: fn() = || #private::assert_payload::<#ty>();
        }
    });

    let ident = &model.ident;
    let variant_count = model.variants.len();
    let markers_tokens: Vec<_> = model
        .variants
        .iter()
        .map(|variant| variant.marker.to_token_stream())
        .collect();
    let marker_bundle = bundle_tuple(&markers_tokens);
    let index_arms = model.variants.iter().enumerate().map(|(index, variant)| {
        let pattern = wildcard_pattern(variant);
        quote!(#pattern => #index)
    });
    let visit_arms = model.variants.iter().enumerate().map(|(index, variant)| {
        let marker = &variant.marker;
        quote!(#index => ::core::option::Option::Some(visitor.visit::<#marker>()))
    });
    let sync_arms = model
        .variants
        .iter()
        .map(|variant| expand_sync_arm(model, variant));
    let remove_all_payloads = remove_payloads(model, None);

    quote! {
        #(#markers)*
        #(#payload_asserts)*

        impl ::bevy_filter_enum::EnumFilter for #ident {
            type Markers = #marker_bundle;

            const VARIANT_COUNT: usize = #variant_count;

            #[inline]
            fn variant_index(&self) -> usize {
                match self {
                    #(#index_arms,)*
                }
            }

            #[inline]
            fn visit_marker<V: #private::MarkerVisitor>(
                index: usize,
                visitor: V,
            ) -> ::core::option::Option<V::Output> {
                match index {
                    #(#visit_arms,)*
                    _ => ::core::option::Option::None,
                }
            }

            fn sync_markers<C>(entity: &mut #ecs::world::EntityWorldMut<'_>)
            where
                C: #ecs::component::Component + ::core::borrow::Borrow<Self>,
            {
                match entity.get::<C>().map(<C as ::core::borrow::Borrow<Self>>::borrow) {
                    ::core::option::Option::None => {
                        #private::clear::<Self>(entity, #remove_all_payloads);
                    }
                    #(#sync_arms)*
                }
            }
        }
    }
}

fn expand_marker(model: &FilterEnum, variant: &FilterVariant) -> TokenStream {
    let ecs = quote!(::bevy_filter_enum::__private::bevy_ecs);
    let vis = &model.vis;
    let marker = &variant.marker;
    let storage = model.storage.ident();
    let doc = format!(
        "Marker component present while [`{}::{}`] is the active variant.",
        model.ident, variant.ident,
    );
    quote! {
        #[doc = #doc]
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
        #vis struct #marker;

        // Hand-written rather than derived so the output does not depend on
        // how the caller's manifest names bevy. Markers are zero-sized, so
        // there is nothing to mutate and `Immutable` costs nothing.
        impl #ecs::component::Component for #marker {
            const STORAGE_TYPE: #ecs::component::StorageType =
                #ecs::component::StorageType::#storage;
            type Mutability = #ecs::component::Immutable;
        }
    }
}

fn expand_sync_arm(model: &FilterEnum, variant: &FilterVariant) -> TokenStream {
    let private = quote!(::bevy_filter_enum::__private);
    let marker = &variant.marker;
    let remove_stale = remove_payloads(model, variant.payload.as_ref());
    match &variant.payload {
        None => {
            let pattern = wildcard_pattern(variant);
            quote! {
                ::core::option::Option::Some(#pattern) => {
                    #private::set_variant::<Self, #marker>(entity, #remove_stale);
                }
            }
        }
        Some(payload) => {
            let variant_ident = &variant.ident;
            let ty = &payload.ty;
            quote! {
                ::core::option::Option::Some(Self::#variant_ident(payload)) => {
                    let payload: #ty = ::core::clone::Clone::clone(payload);
                    #private::set_variant_with_payload::<Self, #marker, #ty>(
                        entity,
                        payload,
                        #remove_stale,
                    );
                }
            }
        }
    }
}

/// A closure removing every extracted payload type except `keep`.
fn remove_payloads(model: &FilterEnum, keep: Option<&Payload>) -> TokenStream {
    let private = quote!(::bevy_filter_enum::__private);
    let stale: Vec<_> = model
        .unique_payloads()
        .into_iter()
        .filter(|payload| keep.is_none_or(|keep| keep.key != payload.key))
        .map(|payload| &payload.ty)
        .collect();
    if stale.is_empty() {
        return quote!(|_| {});
    }
    quote! {
        |entity| { #(#private::remove::<#stale>(entity);)* }
    }
}

fn wildcard_pattern(variant: &FilterVariant) -> TokenStream {
    let ident = &variant.ident;
    match variant.shape {
        Shape::Unit => quote!(Self::#ident),
        Shape::Tuple => quote!(Self::#ident(..)),
        Shape::Struct => quote!(Self::#ident { .. }),
    }
}

/// Nests `types` into tuples of tuples so any variant count fits within
/// Bevy's `Bundle` tuple limit.
fn bundle_tuple(types: &[TokenStream]) -> TokenStream {
    if types.len() <= MAX_BUNDLE_TUPLE {
        return quote!((#(#types,)*));
    }
    let chunks: Vec<_> = types
        .chunks(MAX_BUNDLE_TUPLE)
        .map(|chunk| quote!((#(#chunk,)*)))
        .collect();
    bundle_tuple(&chunks)
}
