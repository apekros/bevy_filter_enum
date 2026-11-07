//! Derive macro for [`bevy_filter_enum`](https://docs.rs/bevy_filter_enum).
//!
//! Use it through the `bevy_filter_enum` crate, which re-exports it and owns
//! the runtime the generated code calls into.

mod expand;
mod model;

use proc_macro::TokenStream;
use syn::{DeriveInput, parse_macro_input};

/// Generates one marker component per variant and implements `EnumFilter`.
///
/// See the `bevy_filter_enum` crate docs for the full attribute reference.
#[proc_macro_derive(EnumFilter, attributes(enum_filter))]
pub fn derive_enum_filter(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    model::FilterEnum::parse(&input)
        .map_or_else(syn::Error::into_compile_error, |model| {
            expand::expand(&model)
        })
        .into()
}
