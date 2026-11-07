//! Validated view of an enum deriving `EnumFilter`.
//!
//! Every rule the generated code relies on is checked here, so `expand` can
//! assume a well-formed model and stay a dumb template.

use quote::{ToTokens, format_ident};
use syn::{Attribute, Data, DeriveInput, Fields, Ident, LitStr, Type, Variant, Visibility};

const ATTR: &str = "enum_filter";

pub struct FilterEnum {
    pub ident: Ident,
    pub vis: Visibility,
    pub storage: Storage,
    pub variants: Vec<FilterVariant>,
}

pub struct FilterVariant {
    pub ident: Ident,
    pub marker: Ident,
    pub shape: Shape,
    /// The payload mirrored onto the entity, for `#[enum_filter(extract)]`.
    pub payload: Option<Payload>,
}

#[derive(Clone, Copy)]
pub enum Shape {
    Unit,
    Tuple,
    Struct,
}

pub struct Payload {
    pub ty: Type,
    /// Token spelling of `ty`. Macros cannot resolve types, so two variants
    /// share a payload when they spell its type the same way.
    pub key: String,
}

/// Mirrors `bevy_ecs::component::StorageType`, spelled the way Bevy's own
/// `#[component(storage = "...")]` spells it.
#[derive(Clone, Copy)]
pub enum Storage {
    Table,
    SparseSet,
}

impl Storage {
    pub fn ident(self) -> Ident {
        match self {
            Self::Table => format_ident!("Table"),
            Self::SparseSet => format_ident!("SparseSet"),
        }
    }
}

impl FilterEnum {
    pub fn parse(input: &DeriveInput) -> syn::Result<Self> {
        let Data::Enum(data) = &input.data else {
            return Err(syn::Error::new_spanned(
                &input.ident,
                "EnumFilter can only be derived for enums",
            ));
        };
        if !input.generics.params.is_empty() || input.generics.where_clause.is_some() {
            return Err(syn::Error::new_spanned(
                &input.generics,
                "EnumFilter does not support generic enums, the generated markers would need the same generics",
            ));
        }
        if data.variants.is_empty() {
            return Err(syn::Error::new_spanned(
                &input.ident,
                "EnumFilter needs at least one variant to have anything to filter on",
            ));
        }

        let storage = parse_enum_attrs(&input.attrs)?;
        let mut errors: Option<syn::Error> = None;
        let mut variants = Vec::with_capacity(data.variants.len());
        for variant in &data.variants {
            match FilterVariant::parse(&input.ident, variant) {
                Ok(variant) => variants.push(variant),
                Err(error) => match &mut errors {
                    Some(errors) => errors.combine(error),
                    None => errors = Some(error),
                },
            }
        }
        if let Some(errors) = errors {
            return Err(errors);
        }

        Ok(Self {
            ident: input.ident.clone(),
            vis: input.vis.clone(),
            storage,
            variants,
        })
    }

    /// Payload types in declaration order, each listed once.
    pub fn unique_payloads(&self) -> Vec<&Payload> {
        let mut unique: Vec<&Payload> = Vec::new();
        for payload in self.variants.iter().filter_map(|v| v.payload.as_ref()) {
            if !unique.iter().any(|seen| seen.key == payload.key) {
                unique.push(payload);
            }
        }
        unique
    }
}

impl FilterVariant {
    fn parse(enum_ident: &Ident, variant: &Variant) -> syn::Result<Self> {
        let shape = match &variant.fields {
            Fields::Unit => Shape::Unit,
            Fields::Unnamed(_) => Shape::Tuple,
            Fields::Named(_) => Shape::Struct,
        };
        let payload = if parse_variant_attrs(&variant.attrs)? {
            Some(extract_payload(variant)?)
        } else {
            None
        };
        Ok(Self {
            ident: variant.ident.clone(),
            marker: format_ident!("{}{}", enum_ident, variant.ident),
            shape,
            payload,
        })
    }
}

fn extract_payload(variant: &Variant) -> syn::Result<Payload> {
    let single_field = match &variant.fields {
        Fields::Unnamed(fields) if fields.unnamed.len() == 1 => fields.unnamed.first(),
        _ => None,
    };
    let field = single_field.ok_or_else(|| {
        syn::Error::new_spanned(
            &variant.ident,
            "#[enum_filter(extract)] needs a tuple variant with exactly one field, like `Active(Progress)`",
        )
    })?;
    Ok(Payload {
        ty: field.ty.clone(),
        key: field.ty.to_token_stream().to_string(),
    })
}

fn parse_enum_attrs(attrs: &[Attribute]) -> syn::Result<Storage> {
    let mut storage = None;
    for attr in attrs.iter().filter(|attr| attr.path().is_ident(ATTR)) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("storage") {
                if storage.is_some() {
                    return Err(meta.error("duplicate `storage` key"));
                }
                let value: LitStr = meta.value()?.parse()?;
                storage = Some(match value.value().as_str() {
                    "Table" => Storage::Table,
                    "SparseSet" => Storage::SparseSet,
                    _ => {
                        return Err(syn::Error::new(
                            value.span(),
                            "expected `\"Table\"` or `\"SparseSet\"`",
                        ));
                    }
                });
                Ok(())
            } else if meta.path.is_ident("extract") {
                Err(meta.error("`extract` goes on a variant, not on the enum"))
            } else {
                Err(meta.error("unknown enum_filter key, expected `storage`"))
            }
        })?;
    }
    // Variant markers come and go every time the enum changes, which is the
    // exact workload sparse sets exist for. Table storage would move every
    // table column of the entity on each transition.
    Ok(storage.unwrap_or(Storage::SparseSet))
}

/// Returns whether the variant asked for `extract`.
fn parse_variant_attrs(attrs: &[Attribute]) -> syn::Result<bool> {
    let mut extract = false;
    for attr in attrs.iter().filter(|attr| attr.path().is_ident(ATTR)) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("extract") {
                if extract {
                    return Err(meta.error("duplicate `extract` key"));
                }
                extract = true;
                Ok(())
            } else if meta.path.is_ident("storage") {
                Err(meta.error("`storage` goes on the enum, not on a variant"))
            } else {
                Err(meta.error("unknown enum_filter key, expected `extract`"))
            }
        })?;
    }
    Ok(extract)
}

#[cfg(test)]
mod tests {
    use proc_macro2::Span;

    use super::*;

    fn parse_err(input: DeriveInput) -> String {
        match FilterEnum::parse(&input) {
            Ok(_) => String::from("<parsed fine>"),
            Err(error) => error.to_string(),
        }
    }

    #[test]
    fn rejects_structs() {
        let message = parse_err(syn::parse_quote! { struct Nope; });
        assert!(message.contains("only be derived for enums"), "{message}");
    }

    #[test]
    fn rejects_generic_enums() {
        let message = parse_err(syn::parse_quote! { enum Nope<T> { A(T) } });
        assert!(message.contains("generic"), "{message}");
    }

    #[test]
    fn rejects_empty_enums() {
        let message = parse_err(syn::parse_quote! { enum Nope {} });
        assert!(message.contains("at least one variant"), "{message}");
    }

    #[test]
    fn rejects_extract_on_non_single_field_variants() {
        for input in [
            syn::parse_quote! { enum E { #[enum_filter(extract)] A } },
            syn::parse_quote! { enum E { #[enum_filter(extract)] A(u8, u8) } },
            syn::parse_quote! { enum E { #[enum_filter(extract)] A { x: u8 } } },
        ] {
            let message = parse_err(input);
            assert!(message.contains("exactly one field"), "{message}");
        }
    }

    #[test]
    fn rejects_misplaced_and_unknown_keys() {
        let message = parse_err(syn::parse_quote! { #[enum_filter(extract)] enum E { A } });
        assert!(message.contains("goes on a variant"), "{message}");

        let message =
            parse_err(syn::parse_quote! { enum E { #[enum_filter(storage = "Table")] A } });
        assert!(message.contains("goes on the enum"), "{message}");

        let message =
            parse_err(syn::parse_quote! { #[enum_filter(storage = "Heap")] enum E { A } });
        assert!(message.contains("SparseSet"), "{message}");

        let message = parse_err(syn::parse_quote! { enum E { #[enum_filter(bogus)] A } });
        assert!(message.contains("unknown enum_filter key"), "{message}");
    }

    #[test]
    fn reports_every_bad_variant_at_once() -> syn::Result<()> {
        let input: DeriveInput = syn::parse_quote! {
            enum E { #[enum_filter(extract)] A, #[enum_filter(extract)] B }
        };
        let Err(error) = FilterEnum::parse(&input) else {
            return Err(syn::Error::new(Span::call_site(), "expected an error"));
        };
        assert_eq!(error.into_iter().count(), 2);
        Ok(())
    }

    #[test]
    fn dedups_payloads_by_spelling() -> syn::Result<()> {
        let input: DeriveInput = syn::parse_quote! {
            enum E {
                #[enum_filter(extract)] A(P),
                #[enum_filter(extract)] B(P),
                #[enum_filter(extract)] C(Q),
                D,
            }
        };
        let model = FilterEnum::parse(&input)?;
        let keys: Vec<_> = model
            .unique_payloads()
            .iter()
            .map(|p| p.key.as_str())
            .collect();
        assert_eq!(keys, ["P", "Q"]);
        Ok(())
    }
}
