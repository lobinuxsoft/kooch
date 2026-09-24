//! `#[derive(Reflect)]` and `#[system]` for `kooch_ecs`; the struct must implement [`Default`].
//! Entity fields: `Option<EntityRef>` for authored links, which keeps an unresolved `Persistent`;
//! `Entity`/`Option<Entity>` for handles the engine resolves.
//!
//! # Example
//!
//! ```ignore
//! #[derive(Default, Reflect)]
//! struct Transform {
//!     pub position: Vec3,
//!     pub rotation: Quat,
//!     pub scale: Vec3,
//! }
//! ```

mod asset_list;
mod attrs;
mod struct_list;
mod system_attr;
mod type_mapping;
mod unit_struct;
mod util;

use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, parse_macro_input};

use crate::attrs::{
    parse_category_attr, parse_field_asset_type, parse_field_bits, parse_field_choices,
    parse_field_doc, parse_field_group, parse_field_requires, parse_field_shown_when,
    parse_field_skip, parse_inspector_attr,
};
use crate::type_mapping::type_mapping;
use crate::unit_struct::unit_struct_impl;
use crate::util::{is_entity, is_entity_ref, option_inner, vec_inner};

/// Declares which frame stage a system binds into, and how.
///
/// ```ignore
/// #[system]                     // Update, gated by Play — the default
/// #[system(PreUpdate)]          // PreUpdate, gated by Play
/// #[system(PostUpdate, always)] // PostUpdate, runs while editing too
/// ```
///
/// 🔴 Expands to the function unchanged: the editor's codegen reads it when writing
/// `registrations.rs`, and a mistyped stage is a compile error.
#[proc_macro_attribute]
pub fn system(args: TokenStream, item: TokenStream) -> TokenStream {
    crate::system_attr::system_impl(args, item)
}

/// Derives `Reflect` for a named-field struct: `reflect_fields`, `reflect_get`, `reflect_set` and
/// `reflect_default`, each field mapping to a known `FieldKind`.
#[proc_macro_derive(Reflect, attributes(reflect))]
pub fn derive_reflect(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    // Parse #[reflect(inspector = "hidden"|"read_only"|"editable")] attribute.
    let inspector_visibility = match parse_inspector_attr(&input) {
        Ok(vis) => vis,
        Err(err) => return err,
    };

    // Parse #[reflect(category = "...")] attribute.
    let category = match parse_category_attr(&input) {
        Ok(cat) => cat,
        Err(err) => return err,
    };

    let fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(named) => &named.named,
            Fields::Unit => {
                // Unit struct — no fields.
                return unit_struct_impl(name, inspector_visibility.as_ref(), category.as_deref());
            }
            Fields::Unnamed(_) => {
                return syn::Error::new_spanned(
                    name,
                    "Reflect derive does not support tuple structs",
                )
                .to_compile_error()
                .into();
            }
        },
        _ => {
            return syn::Error::new_spanned(name, "Reflect derive only supports structs")
                .to_compile_error()
                .into();
        }
    };

    let mut field_metas = Vec::new();
    let mut get_arms = Vec::new();
    let mut set_arms = Vec::new();

    for field in fields {
        let field_name = field.ident.as_ref().unwrap();
        let field_name_str = field_name.to_string();
        let ty = &field.ty;
        // #737 — the field's doc comment becomes its tooltip, harvested once so every `FieldMeta`
        // branch below agrees.
        let field_doc = parse_field_doc(field);
        // #830 — the Inspector heading, harvested beside the doc comment for the same reason.
        let field_group = match parse_field_group(field) {
            Ok(group) => group.unwrap_or_default(),
            Err(e) => return e,
        };

        // `#[reflect(skip)]` leaves opaque handle fields out of the Inspector and get/set.
        let skip = match parse_field_skip(field) {
            Ok(skip) => skip,
            Err(e) => return e,
        };
        if skip {
            continue;
        }

        let set_pattern = match crate::attrs::parse_field_alias(field) {
            Ok(Some(aliases)) => {
                // `alias = "a, b"`: a field renamed twice still reads both older names.
                let aliases = aliases.split(',').map(str::trim);
                quote! { #field_name_str #(| #aliases)* }
            }
            Ok(None) => quote! { #field_name_str },
            Err(e) => return e,
        };

        // `#[reflect(asset = ...)]` makes an `Option<Guid>` a typed asset reference
        // (`FieldKind::AssetRef`).
        let asset_type = match parse_field_asset_type(field) {
            Ok(opt) => opt,
            Err(e) => return e,
        };
        if let Some(asset_type) = &asset_type
            && vec_inner(&field.ty).and_then(option_inner).is_some()
        {
            let (meta, get, set) = asset_list::asset_list(
                field_name,
                &field_name_str,
                &set_pattern,
                asset_type,
                &field_doc,
                &field_group,
            );
            field_metas.push(meta);
            get_arms.push(get);
            set_arms.push(set);
            continue;
        }
        if let Some(asset_type) = asset_type {
            field_metas.push(quote! {
                ::kooch_ecs::reflect::FieldMeta {
                    name: #field_name_str,
                    type_name: "Option<kooch_core::Guid>",
                    kind: ::kooch_ecs::reflect::FieldKind::AssetRef,
                    choices: &[],
                    bits: &[],
                    layers: false,
                    layer: false,
                    hidden: false,
                    range: None,
                    // An asset picker has no variant to depend on yet.
                    shown_when: ::core::option::Option::None,
                    asset_type: #asset_type,
                    requires: "",
                    doc: #field_doc,
                    group: #field_group,
                    fields: &[],
                }
            });
            get_arms.push(quote! {
                #field_name_str => Some(::kooch_ecs::reflect::ReflectValue::AssetRef {
                    guid: self.#field_name,
                    asset_type: #asset_type.to_owned(),
                }),
            });
            set_arms.push(quote! {
                #set_pattern => match value {
                    ::kooch_ecs::reflect::ReflectValue::AssetRef { guid, .. } => {
                        self.#field_name = guid;
                        Ok(())
                    }
                    other => Err(::kooch_ecs::reflect::ReflectError::TypeMismatch {
                        field: #field_name_str.into(),
                        expected: ::kooch_ecs::reflect::FieldKind::AssetRef,
                        got: other.kind(),
                    }),
                },
            });
            continue;
        }

        // `Option<EntityRef>` is what an authored reference should be: code, the picker and a drag
        // store the same value, and an unresolved `Persistent` survives. Bare `EntityRef` cannot
        // point at nothing.
        if is_entity_ref(ty) {
            return syn::Error::new_spanned(
                ty,
                "Reflect derive: use `Option<EntityRef>` rather than a bare `EntityRef`. \
                 A reference field has to be able to say it points at nothing.",
            )
            .to_compile_error()
            .into();
        }
        if option_inner(ty).is_some_and(is_entity_ref) {
            let shown_when_expr = match parse_field_shown_when(field) {
                Ok(Some(expr)) => quote! { ::core::option::Option::Some(&#expr) },
                Ok(None) => quote! { ::core::option::Option::None },
                Err(e) => return e,
            };
            let requires = match parse_field_requires(field) {
                Ok(requires) => requires.unwrap_or_default(),
                Err(e) => return e,
            };

            field_metas.push(quote! {
                ::kooch_ecs::reflect::FieldMeta {
                    name: #field_name_str,
                    type_name: "Option<EntityRef>",
                    kind: ::kooch_ecs::reflect::FieldKind::EntityRef,
                    choices: &[],
                    bits: &[],
                    layers: false,
                    layer: false,
                    hidden: false,
                    range: None,
                    shown_when: #shown_when_expr,
                    asset_type: "",
                    requires: #requires,
                    doc: #field_doc,
                    group: #field_group,
                    fields: &[],
                }
            });

            get_arms.push(quote! {
                #field_name_str => Some(::kooch_ecs::reflect::ReflectValue::EntityRef(self.#field_name)),
            });

            // Both reference states are accepted: a `Persistent` one means the target's scene is
            // not open — ordinary under world-cell streaming — and is kept to resolve later.
            set_arms.push(quote! {
                #set_pattern => match value {
                    ::kooch_ecs::reflect::ReflectValue::EntityRef(reference) => {
                        self.#field_name = reference;
                        Ok(())
                    }
                    other => Err(::kooch_ecs::reflect::ReflectError::TypeMismatch {
                        field: #field_name_str.into(),
                        expected: ::kooch_ecs::reflect::FieldKind::EntityRef,
                        got: other.kind(),
                    }),
                },
            });
            continue;
        }

        // `Entity`/`Option<Entity>` hold `EntityRef::Live` only: the scene load resolves references
        // first, so a `Persistent` here means that pass was skipped.
        let optional_entity = option_inner(ty).is_some_and(is_entity);
        if optional_entity || is_entity(ty) {
            let type_name_str = if optional_entity {
                "Option<Entity>"
            } else {
                "Entity"
            };
            let shown_when_expr = match parse_field_shown_when(field) {
                Ok(Some(expr)) => quote! { ::core::option::Option::Some(&#expr) },
                Ok(None) => quote! { ::core::option::Option::None },
                Err(e) => return e,
            };

            field_metas.push(quote! {
                ::kooch_ecs::reflect::FieldMeta {
                    name: #field_name_str,
                    type_name: #type_name_str,
                    kind: ::kooch_ecs::reflect::FieldKind::EntityRef,
                    choices: &[],
                    bits: &[],
                    layers: false,
                    layer: false,
                    hidden: false,
                    range: None,
                    shown_when: #shown_when_expr,
                    asset_type: "",
                    requires: "",
                    doc: #field_doc,
                    group: #field_group,
                    fields: &[],
                }
            });

            let get_expr = if optional_entity {
                quote! { self.#field_name.map(::kooch_ecs::reflect::EntityRef::live) }
            } else {
                quote! { ::core::option::Option::Some(::kooch_ecs::reflect::EntityRef::live(self.#field_name)) }
            };
            get_arms.push(quote! {
                #field_name_str => Some(::kooch_ecs::reflect::ReflectValue::EntityRef(#get_expr)),
            });

            // A cleared field is `None` for an optional one and the
            // `INVALID` sentinel otherwise — the same distinction
            // `Option<Entity>` versus `Entity` already makes elsewhere.
            let set_body = if optional_entity {
                quote! {
                    match reference {
                        ::core::option::Option::None => {
                            self.#field_name = ::core::option::Option::None;
                            Ok(())
                        }
                        ::core::option::Option::Some(reference) => {
                            match reference.entity() {
                                ::core::option::Option::Some(entity) => {
                                    self.#field_name = ::core::option::Option::Some(entity);
                                    Ok(())
                                }
                                ::core::option::Option::None => Err(
                                    ::kooch_ecs::reflect::ReflectError::UnresolvedEntityRef {
                                        field: #field_name_str.into(),
                                    },
                                ),
                            }
                        }
                    }
                }
            } else {
                quote! {
                    match reference {
                        ::core::option::Option::None => {
                            self.#field_name = ::kooch_ecs::entity::Entity::INVALID;
                            Ok(())
                        }
                        ::core::option::Option::Some(reference) => {
                            match reference.entity() {
                                ::core::option::Option::Some(entity) => {
                                    self.#field_name = entity;
                                    Ok(())
                                }
                                ::core::option::Option::None => Err(
                                    ::kooch_ecs::reflect::ReflectError::UnresolvedEntityRef {
                                        field: #field_name_str.into(),
                                    },
                                ),
                            }
                        }
                    }
                }
            };
            set_arms.push(quote! {
                #set_pattern => match value {
                    ::kooch_ecs::reflect::ReflectValue::EntityRef(reference) => #set_body,
                    other => Err(::kooch_ecs::reflect::ReflectError::TypeMismatch {
                        field: #field_name_str.into(),
                        expected: ::kooch_ecs::reflect::FieldKind::EntityRef,
                        got: other.kind(),
                    }),
                },
            });
            continue;
        }

        // Any other `Vec<T>` is a list of reflected structs: `T` must derive `Reflect` and
        // `Default`, and the compiler says so if it does not.
        if let Some(element) = vec_inner(ty)
            && type_mapping(element).is_none()
        {
            let bare = match crate::attrs::parse_field_bare(field) {
                Ok(bare) => bare,
                Err(e) => return e,
            };
            let (meta, get, set) = struct_list::struct_list(
                field_name,
                &field_name_str,
                &set_pattern,
                element,
                bare.as_deref(),
                &field_doc,
                &field_group,
            );
            field_metas.push(meta);
            get_arms.push(get);
            set_arms.push(set);
            continue;
        }

        let Some((kind_variant, type_name_str, needs_clone)) = type_mapping(ty) else {
            return syn::Error::new_spanned(
                ty,
                format!(
                    "Reflect derive: unsupported field type `{}`. \
                     Supported: f32, f64, u8..u64, i8..i64, bool, String, \
                     Vec2, Vec3, Vec4, Quat, Mat4. \
                     Use `#[reflect(skip)]` to opt out.",
                    quote!(#ty),
                ),
            )
            .to_compile_error()
            .into();
        };

        let kind_ident: proc_macro2::TokenStream = kind_variant.parse().unwrap();
        let value_ident: proc_macro2::TokenStream = kind_variant.parse().unwrap();

        let choices_expr = match parse_field_choices(field) {
            Ok(Some(expr)) => quote! { #expr },
            Ok(None) => quote! { &[] },
            Err(e) => return e,
        };
        let bits_expr = match parse_field_bits(field) {
            Ok(Some(expr)) => quote! { #expr },
            Ok(None) => quote! { &[] },
            Err(e) => return e,
        };
        let hidden_flag = match crate::attrs::parse_field_hidden(field) {
            Ok(flag) => flag,
            Err(e) => return e,
        };
        let layer_flag = match crate::attrs::parse_field_layer(field) {
            Ok(flag) => flag,
            Err(e) => return e,
        };
        let layers_flag = match crate::attrs::parse_field_layers(field) {
            Ok(flag) => flag,
            Err(e) => return e,
        };
        let range_expr = match crate::attrs::parse_field_range(field) {
            Ok(Some(path)) => quote! { Some(&#path) },
            Ok(None) => quote! { None },
            Err(e) => return e,
        };
        let shown_when_expr = match parse_field_shown_when(field) {
            Ok(Some(expr)) => quote! { ::core::option::Option::Some(&#expr) },
            Ok(None) => quote! { ::core::option::Option::None },
            Err(e) => return e,
        };

        // FieldMeta entry.
        field_metas.push(quote! {
            ::kooch_ecs::reflect::FieldMeta {
                name: #field_name_str,
                type_name: #type_name_str,
                kind: ::kooch_ecs::reflect::FieldKind::#kind_ident,
                choices: #choices_expr,
                bits: #bits_expr,
                layers: #layers_flag,
                layer: #layer_flag,
                hidden: #hidden_flag,
                range: #range_expr,
                shown_when: #shown_when_expr,
                asset_type: "",
                requires: "",
                doc: #field_doc,
                group: #field_group,
                fields: &[],
            }
        });

        // reflect_get arm.
        if needs_clone {
            get_arms.push(quote! {
                #field_name_str => Some(::kooch_ecs::reflect::ReflectValue::#value_ident(self.#field_name.clone())),
            });
        } else {
            get_arms.push(quote! {
                #field_name_str => Some(::kooch_ecs::reflect::ReflectValue::#value_ident(self.#field_name)),
            });
        }

        // reflect_set arm.
        set_arms.push(quote! {
            #set_pattern => match value {
                ::kooch_ecs::reflect::ReflectValue::#value_ident(v) => {
                    self.#field_name = v;
                    Ok(())
                }
                other => Err(::kooch_ecs::reflect::ReflectError::TypeMismatch {
                    field: #field_name_str.into(),
                    expected: ::kooch_ecs::reflect::FieldKind::#kind_ident,
                    got: other.kind(),
                }),
            },
        });
    }

    let field_count = field_metas.len();

    let visibility_method = inspector_visibility.map(|vis| {
        quote! {
            fn inspector_visibility() -> ::kooch_ecs::reflect::InspectorVisibility {
                ::kooch_ecs::reflect::InspectorVisibility::#vis
            }
        }
    });

    let category_method = category.as_deref().map(|cat| {
        quote! {
            fn category() -> Option<&'static str> {
                Some(#cat)
            }
        }
    });

    let expanded = quote! {
        impl #name {
            /// The reflected fields, as a constant so a list of this type can carry them (#1209).
            #[doc(hidden)]
            pub const REFLECT_FIELDS: &'static [::kooch_ecs::reflect::FieldMeta] = &[
                #(#field_metas),*
            ];
        }

        impl ::kooch_ecs::reflect::Reflect for #name {
            fn reflect_fields(&self) -> &'static [::kooch_ecs::reflect::FieldMeta] {
                Self::REFLECT_FIELDS
            }

            fn reflect_get(&self, field: &str) -> Option<::kooch_ecs::reflect::ReflectValue> {
                match field {
                    #(#get_arms)*
                    _ => None,
                }
            }

            fn reflect_set(
                &mut self,
                field: &str,
                value: ::kooch_ecs::reflect::ReflectValue,
            ) -> Result<(), ::kooch_ecs::reflect::ReflectError> {
                match field {
                    #(#set_arms)*
                    _ => Err(::kooch_ecs::reflect::ReflectError::FieldNotFound(field.into())),
                }
            }

            fn reflect_default() -> Self {
                Self::default()
            }

            #visibility_method
            #category_method
        }
    };

    // Suppress "unused field_count" — it's used for the static array size hint.
    let _ = field_count;
    expanded.into()
}
