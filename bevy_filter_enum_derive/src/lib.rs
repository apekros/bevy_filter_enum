use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{parse_macro_input, Data, DeriveInput, Fields, LitStr, Path};

/// Derives marker components and a sync plugin for an enum component.
///
/// Tuple variants with exactly one field can additionally opt into payload
/// extraction with `#[enum_filter(extract)]`: while the variant is active,
/// a clone of its payload is kept in sync as its own component on the
/// entity, so systems can query the payload type directly instead of
/// matching through the enum. The payload type must implement `Component`
/// and `Clone`. The enum remains the source of truth; treat the extracted
/// component as a read-only projection.
#[proc_macro_derive(EnumFilter, attributes(strum, enum_filter))]
pub fn derive_enum_filter(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    let enum_ident = input.ident.clone();
    let vis = input.vis.clone();

    let enum_has_strum_attrs = input.attrs.iter().any(|attr| attr.path().is_ident("strum"));

    let Data::Enum(data_enum) = input.data else {
        return syn::Error::new_spanned(enum_ident, "EnumFilter can only be derived for enums")
            .to_compile_error()
            .into();
    };

    let mut variant_idents = Vec::new();
    let mut variant_patterns = Vec::new();
    let mut marker_idents = Vec::new();
    let mut marker_display_impls = Vec::new();
    let mut extract_types = Vec::new();

    for variant in data_enum.variants.iter() {
        let extract_type = match variant_extract_type(variant) {
            Ok(extract_type) => extract_type,
            Err(error) => return error.to_compile_error().into(),
        };
        extract_types.push(extract_type);
        let variant_has_strum_attrs = variant
            .attrs
            .iter()
            .any(|attr| attr.path().is_ident("strum"));

        let variant_ident = &variant.ident;
        let variant_pattern = match &variant.fields {
            Fields::Unit => quote! { #enum_ident::#variant_ident },
            Fields::Unnamed(_) => quote! { #enum_ident::#variant_ident(..) },
            Fields::Named(_) => quote! { #enum_ident::#variant_ident { .. } },
        };
        let marker_ident = format_ident!("{}{}", enum_ident, variant_ident);
        let display_impl = if matches!(variant.fields, Fields::Unit)
            && (enum_has_strum_attrs || variant_has_strum_attrs)
        {
            quote! {
                impl ::core::fmt::Display for #marker_ident {
                    fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                        ::core::fmt::Display::fmt(&#enum_ident::#variant_ident, f)
                    }
                }
            }
        } else {
            let marker_display = variant_ident.to_string();
            quote! {
                impl ::core::fmt::Display for #marker_ident {
                    fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                        f.write_str(#marker_display)
                    }
                }
            }
        };

        variant_idents.push(variant_ident.clone());
        variant_patterns.push(variant_pattern);
        marker_idents.push(marker_ident);
        marker_display_impls.push(display_impl);
    }

    let plugin_ident = format_ident!("{}EnumFilterPlugin", enum_ident);
    let register_hooks_fn_ident = format_ident!(
        "__bevy_enum_filter_register_hooks_{}",
        enum_ident.to_string().to_lowercase()
    );
    let hook_sync_fn_ident = format_ident!(
        "__bevy_enum_filter_hook_sync_{}",
        enum_ident.to_string().to_lowercase()
    );
    let hook_cleanup_fn_ident = format_ident!(
        "__bevy_enum_filter_hook_cleanup_{}",
        enum_ident.to_string().to_lowercase()
    );
    let sync_fn_ident = format_ident!(
        "__bevy_enum_filter_sync_{}",
        enum_ident.to_string().to_lowercase()
    );
    let cleanup_fn_ident = format_ident!(
        "__bevy_enum_filter_cleanup_{}",
        enum_ident.to_string().to_lowercase()
    );

    let variant_indexes: Vec<_> = (0..variant_idents.len()).collect::<Vec<_>>();

    let mut unique_payload_types: Vec<syn::Type> = Vec::new();
    let mut seen_payload_types: Vec<String> = Vec::new();
    for payload_type in extract_types.iter().flatten() {
        let token_string = quote!(#payload_type).to_string();
        if !seen_payload_types.contains(&token_string) {
            seen_payload_types.push(token_string);
            unique_payload_types.push(payload_type.clone());
        }
    }

    let mut hook_payload_lets: Vec<TokenStream2> = Vec::new();
    let mut hook_payload_inserts: Vec<TokenStream2> = Vec::new();
    for (index, payload_type) in extract_types.iter().enumerate() {
        let Some(payload_type) = payload_type else {
            continue;
        };
        let variant_ident = &variant_idents[index];
        let payload_var = format_ident!("__enum_filter_payload_{}", index);
        let other_patterns: Vec<_> = variant_patterns
            .iter()
            .enumerate()
            .filter(|(other_index, _)| *other_index != index)
            .map(|(_, pattern)| pattern.clone())
            .collect();
        hook_payload_lets.push(quote! {
            let #payload_var: ::core::option::Option<#payload_type> = match value {
                #enum_ident::#variant_ident(payload) => {
                    ::core::option::Option::Some(::core::clone::Clone::clone(payload))
                }
                #(#other_patterns => ::core::option::Option::None,)*
            };
        });
        hook_payload_inserts.push(quote! {
            if let ::core::option::Option::Some(payload) = #payload_var {
                entity_commands.insert(payload);
            }
        });
    }

    // Payload projections are synced by removing the payload types the active
    // variant does not carry and (re)inserting the active variant's payload. A
    // payload whose variant stays active is replaced in place, so it reports
    // `Discard` and `Insert` rather than a spurious `Remove` and `Add`.
    let (remove_stale_payloads, remove_all_payloads, system_payload_sync) = if unique_payload_types
        .is_empty()
    {
        (quote! {}, quote! {}, quote! {})
    } else {
        let system_payload_arms = variant_patterns.iter().enumerate().map(|(index, pattern)| {
            match &extract_types[index] {
                Some(_) => {
                    let variant_ident = &variant_idents[index];
                    quote! {
                        #enum_ident::#variant_ident(payload) => {
                            ec.insert(::core::clone::Clone::clone(payload));
                        }
                    }
                }
                None => quote! { #pattern => {} },
            }
        });
        // For each payload type, the indexes of the variants carrying it.
        // Expects `variant_index` and `ec` in scope.
        let stale_payload_removals: Vec<_> = unique_payload_types
            .iter()
            .map(|payload_type| {
                let payload_token = quote!(#payload_type).to_string();
                let carrying_indexes: Vec<_> = extract_types
                    .iter()
                    .enumerate()
                    .filter(|(_, extract_type)| {
                        extract_type
                            .as_ref()
                            .is_some_and(|ty| quote!(#ty).to_string() == payload_token)
                    })
                    .map(|(index, _)| index)
                    .collect();
                quote! {
                    if !::core::matches!(variant_index, #(#carrying_indexes)|*) {
                        ec.remove::<#payload_type>();
                    }
                }
            })
            .collect();
        (
            quote! { #(#stale_payload_removals)* },
            quote! { ec.remove::<(#(#unique_payload_types,)*)>(); },
            quote! {
                match value {
                    #(#system_payload_arms)*
                }
            },
        )
    };

    let marker_docs: Vec<_> = variant_idents
        .iter()
        .map(|v| format!("Marker component for [`{enum_ident}::{v}`]."))
        .collect();

    let plugin_doc = format!("Plugin that syncs [`{enum_ident}`] variants to marker components.");

    let expanded = quote! {
        #(
            #[doc = #marker_docs]
            #[derive(::bevy_filter_enum::__private::bevy_ecs::prelude::Component, Debug, Default)]
            #vis struct #marker_idents;
        )*

        #(#marker_display_impls)*

        impl ::bevy_filter_enum::EnumFilterValue for #enum_ident {
            fn marker_index(&self) -> usize {
                match self {
                    #(
                        #variant_patterns => #variant_indexes,
                    )*
                }
            }

            fn sync_markers(
                active: impl Fn(usize) -> bool,
                entity: &mut ::bevy_filter_enum::__private::bevy_ecs::system::EntityCommands<'_>,
            ) {
                #(
                    if active(#variant_indexes) {
                        entity.insert_if_new(#marker_idents);
                    } else {
                        entity.remove::<#marker_idents>();
                    }
                )*
            }
        }

        #[doc = #plugin_doc]
        #vis struct #plugin_ident;

        impl ::bevy_filter_enum::__private::bevy_app::Plugin for #plugin_ident {
            fn build(&self, app: &mut ::bevy_filter_enum::__private::bevy_app::App) {
                #register_hooks_fn_ident(app.world_mut());
                app.add_systems(
                    ::bevy_filter_enum::__private::bevy_app::PreUpdate,
                    (
                        #sync_fn_ident,
                        #cleanup_fn_ident,
                    ),
                );
            }
        }

        fn #register_hooks_fn_ident(
            world: &mut ::bevy_filter_enum::__private::bevy_ecs::world::World,
        ) {
            let component_id = world.register_component::<#enum_ident>();
            if world.archetypes().iter().any(|archetype| archetype.contains(component_id)) {
                return;
            }

            let Some(hooks) = world.register_component_hooks_by_id(component_id) else {
                return;
            };

            let _ = hooks.try_on_insert(#hook_sync_fn_ident);
            let _ = hooks.try_on_remove(#hook_cleanup_fn_ident);
        }

        fn #hook_sync_fn_ident(
            mut world: ::bevy_filter_enum::__private::bevy_ecs::world::DeferredWorld<'_>,
            context: ::bevy_filter_enum::__private::bevy_ecs::lifecycle::HookContext,
        ) {
            let ::core::option::Option::Some(value) = world.get::<#enum_ident>(context.entity) else {
                return;
            };
            let variant_index = ::bevy_filter_enum::EnumFilterValue::marker_index(value);
            #(#hook_payload_lets)*

            let mut commands = world.commands();
            let mut entity_commands = commands.entity(context.entity);
            <#enum_ident as ::bevy_filter_enum::EnumFilterValue>::sync_markers(
                |index| index == variant_index,
                &mut entity_commands,
            );
            {
                let ec = &mut entity_commands;
                #remove_stale_payloads
            }
            #(#hook_payload_inserts)*
        }

        fn #hook_cleanup_fn_ident(
            mut world: ::bevy_filter_enum::__private::bevy_ecs::world::DeferredWorld<'_>,
            context: ::bevy_filter_enum::__private::bevy_ecs::lifecycle::HookContext,
        ) {
            let mut commands = world.commands();
            let mut ec = commands.entity(context.entity);
            <#enum_ident as ::bevy_filter_enum::EnumFilterValue>::remove_markers(&mut ec);
            #remove_all_payloads
        }

        fn #sync_fn_ident(
            mut commands: ::bevy_filter_enum::__private::bevy_ecs::prelude::Commands,
            q: ::bevy_filter_enum::__private::bevy_ecs::prelude::Query<
                (
                    ::bevy_filter_enum::__private::bevy_ecs::prelude::Entity,
                    &#enum_ident
                ),
                ::bevy_filter_enum::__private::bevy_ecs::prelude::Or<(
                    ::bevy_filter_enum::__private::bevy_ecs::prelude::Added<#enum_ident>,
                    ::bevy_filter_enum::__private::bevy_ecs::prelude::Changed<#enum_ident>,
                )>
            >,
        ) {
            for (entity, value) in &q {
                let variant_index = ::bevy_filter_enum::EnumFilterValue::marker_index(value);
                let mut ec = commands.entity(entity);
                <#enum_ident as ::bevy_filter_enum::EnumFilterValue>::sync_markers(
                    |index| index == variant_index,
                    &mut ec,
                );
                {
                    let ec = &mut ec;
                    #remove_stale_payloads
                }
                #system_payload_sync
            }
        }

        fn #cleanup_fn_ident(
            mut commands: ::bevy_filter_enum::__private::bevy_ecs::prelude::Commands,
            mut removed: ::bevy_filter_enum::__private::bevy_ecs::lifecycle::RemovedComponents<#enum_ident>,
        ) {
            for entity in removed.read() {
                let mut ec = commands.entity(entity);
                <#enum_ident as ::bevy_filter_enum::EnumFilterValue>::remove_markers(&mut ec);
                #remove_all_payloads
            }
        }
    };

    TokenStream::from(expanded)
}

/// Returns the payload type for variants opting into payload extraction via
/// `#[enum_filter(extract)]`.
fn variant_extract_type(variant: &syn::Variant) -> syn::Result<Option<syn::Type>> {
    let mut extract = false;
    for attr in &variant.attrs {
        if !attr.path().is_ident("enum_filter") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("extract") {
                extract = true;
                Ok(())
            } else {
                Err(meta.error("unsupported enum_filter attribute key, expected `extract`"))
            }
        })?;
    }
    if !extract {
        return Ok(None);
    }

    let error = || {
        syn::Error::new_spanned(
            variant,
            "#[enum_filter(extract)] requires a tuple variant with exactly one field",
        )
    };
    let Fields::Unnamed(fields) = &variant.fields else {
        return Err(error());
    };
    let mut field_types = fields.unnamed.iter().map(|field| &field.ty);
    match (field_types.next(), field_types.next()) {
        (Some(payload_type), None) => Ok(Some(payload_type.clone())),
        _ => Err(error()),
    }
}

#[proc_macro_derive(EnumFilterCollection, attributes(enum_filter_collection))]
pub fn derive_enum_filter_collection(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let component_ident = input.ident.clone();
    let vis = input.vis.clone();

    let config = match EnumFilterCollectionConfig::parse(&input) {
        Ok(config) => config,
        Err(error) => return error.to_compile_error().into(),
    };

    let Data::Struct(data_struct) = &input.data else {
        return syn::Error::new_spanned(
            component_ident,
            "EnumFilterCollection can only be derived for structs",
        )
        .to_compile_error()
        .into();
    };

    let Fields::Named(fields) = &data_struct.fields else {
        return syn::Error::new_spanned(
            component_ident,
            "EnumFilterCollection only supports structs with named fields",
        )
        .to_compile_error()
        .into();
    };

    if !fields.named.iter().any(|field| {
        field
            .ident
            .as_ref()
            .is_some_and(|ident| ident == &config.iter_field)
    }) {
        return syn::Error::new_spanned(
            &component_ident,
            format!(
                "EnumFilterCollection iter field `{}` does not exist",
                config.iter_field
            ),
        )
        .to_compile_error()
        .into();
    }

    let enum_path = config.enum_path;
    let iter_field = config.iter_field;
    let marker_value = match config.map_field {
        Some(map_field) => quote! { &item.#map_field },
        None => quote! { item },
    };
    let plugin_ident = format_ident!("{}EnumFilterPlugin", component_ident);
    let register_hooks_fn_ident = format_ident!(
        "__bevy_enum_filter_collection_register_hooks_{}",
        component_ident.to_string().to_lowercase()
    );
    let hook_sync_fn_ident = format_ident!(
        "__bevy_enum_filter_collection_hook_sync_{}",
        component_ident.to_string().to_lowercase()
    );
    let hook_cleanup_fn_ident = format_ident!(
        "__bevy_enum_filter_collection_hook_cleanup_{}",
        component_ident.to_string().to_lowercase()
    );
    let sync_fn_ident = format_ident!(
        "__bevy_enum_filter_collection_sync_{}",
        component_ident.to_string().to_lowercase()
    );
    let cleanup_fn_ident = format_ident!(
        "__bevy_enum_filter_collection_cleanup_{}",
        component_ident.to_string().to_lowercase()
    );
    let plugin_doc = format!(
        "Plugin that syncs [`{component_ident}`] enum collection values to marker components."
    );

    let expanded = quote! {
        #[doc = #plugin_doc]
        #vis struct #plugin_ident;

        impl ::bevy_filter_enum::__private::bevy_app::Plugin for #plugin_ident {
            fn build(&self, app: &mut ::bevy_filter_enum::__private::bevy_app::App) {
                #register_hooks_fn_ident(app.world_mut());
                app.add_systems(
                    ::bevy_filter_enum::__private::bevy_app::PreUpdate,
                    (
                        #sync_fn_ident,
                        #cleanup_fn_ident,
                    ),
                );
            }
        }

        fn #register_hooks_fn_ident(
            world: &mut ::bevy_filter_enum::__private::bevy_ecs::world::World,
        ) {
            let component_id = world.register_component::<#component_ident>();
            if world.archetypes().iter().any(|archetype| archetype.contains(component_id)) {
                return;
            }

            let Some(hooks) = world.register_component_hooks_by_id(component_id) else {
                return;
            };

            let _ = hooks.try_on_insert(#hook_sync_fn_ident);
            let _ = hooks.try_on_remove(#hook_cleanup_fn_ident);
        }

        fn #hook_sync_fn_ident(
            mut world: ::bevy_filter_enum::__private::bevy_ecs::world::DeferredWorld<'_>,
            context: ::bevy_filter_enum::__private::bevy_ecs::lifecycle::HookContext,
        ) {
            let marker_indexes: ::std::vec::Vec<usize> = match world.get::<#component_ident>(context.entity) {
                Some(value) => value
                    .#iter_field
                    .iter()
                    .map(|item| ::bevy_filter_enum::EnumFilterValue::marker_index(#marker_value))
                    .collect(),
                None => return,
            };

            let mut commands = world.commands();
            let mut entity_commands = commands.entity(context.entity);
            <#enum_path as ::bevy_filter_enum::EnumFilterValue>::sync_markers(
                |index| marker_indexes.contains(&index),
                &mut entity_commands,
            );
        }

        fn #hook_cleanup_fn_ident(
            mut world: ::bevy_filter_enum::__private::bevy_ecs::world::DeferredWorld<'_>,
            context: ::bevy_filter_enum::__private::bevy_ecs::lifecycle::HookContext,
        ) {
            let mut commands = world.commands();
            let mut entity_commands = commands.entity(context.entity);
            <#enum_path as ::bevy_filter_enum::EnumFilterValue>::remove_markers(&mut entity_commands);
        }

        fn #sync_fn_ident(
            mut commands: ::bevy_filter_enum::__private::bevy_ecs::prelude::Commands,
            q: ::bevy_filter_enum::__private::bevy_ecs::prelude::Query<
                (
                    ::bevy_filter_enum::__private::bevy_ecs::prelude::Entity,
                    &#component_ident
                ),
                ::bevy_filter_enum::__private::bevy_ecs::prelude::Or<(
                    ::bevy_filter_enum::__private::bevy_ecs::prelude::Added<#component_ident>,
                    ::bevy_filter_enum::__private::bevy_ecs::prelude::Changed<#component_ident>,
                )>
            >,
        ) {
            for (entity, collection) in &q {
                let marker_indexes: ::std::vec::Vec<usize> = collection
                    .#iter_field
                    .iter()
                    .map(|item| ::bevy_filter_enum::EnumFilterValue::marker_index(#marker_value))
                    .collect();
                let mut ec = commands.entity(entity);
                <#enum_path as ::bevy_filter_enum::EnumFilterValue>::sync_markers(
                    |index| marker_indexes.contains(&index),
                    &mut ec,
                );
            }
        }

        fn #cleanup_fn_ident(
            mut commands: ::bevy_filter_enum::__private::bevy_ecs::prelude::Commands,
            mut removed: ::bevy_filter_enum::__private::bevy_ecs::lifecycle::RemovedComponents<#component_ident>,
        ) {
            for entity in removed.read() {
                let mut ec = commands.entity(entity);
                <#enum_path as ::bevy_filter_enum::EnumFilterValue>::remove_markers(&mut ec);
            }
        }
    };

    TokenStream::from(expanded)
}

struct EnumFilterCollectionConfig {
    enum_path: Path,
    iter_field: syn::Ident,
    map_field: Option<syn::Ident>,
}

impl EnumFilterCollectionConfig {
    fn parse(input: &DeriveInput) -> syn::Result<Self> {
        let attr = input
            .attrs
            .iter()
            .find(|attr| attr.path().is_ident("enum_filter_collection"))
            .ok_or_else(|| {
                syn::Error::new_spanned(
                    input,
                    "missing #[enum_filter_collection(enum = EnumType, iter = \"field\")]",
                )
            })?;

        let mut enum_path = None;
        let mut iter_field = None;
        let mut map_field = None;

        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("enum") {
                enum_path = Some(meta.value()?.parse::<Path>()?);
                Ok(())
            } else if meta.path.is_ident("iter") {
                let value = meta.value()?.parse::<LitStr>()?;
                iter_field = Some(syn::Ident::new(&value.value(), value.span()));
                Ok(())
            } else if meta.path.is_ident("map") {
                let value = meta.value()?.parse::<LitStr>()?;
                map_field = Some(syn::Ident::new(&value.value(), value.span()));
                Ok(())
            } else {
                Err(meta.error("unsupported enum_filter_collection attribute key"))
            }
        })?;

        Ok(Self {
            enum_path: enum_path.ok_or_else(|| syn::Error::new_spanned(input, "missing enum"))?,
            iter_field: iter_field.ok_or_else(|| syn::Error::new_spanned(input, "missing iter"))?,
            map_field,
        })
    }
}
