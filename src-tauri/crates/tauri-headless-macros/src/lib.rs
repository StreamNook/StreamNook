//! Macros for the headless `tauri` stand-in used by the iOS core build.
//!
//! The real `#[tauri::command]` wraps a function for Tauri's IPC; here the
//! function is kept unchanged and a hidden module of the same name is emitted
//! next to it, holding `__headless` (the app handle plus the JSON arguments
//! object in, a boxed future of the JSON result out). Sharing the name means
//! any `use` that brings in the command brings in its module too, which is
//! how Tauri's own same-named macro travels. `generate_handler!` maps command
//! names onto those modules, keeping each entry's `#[cfg]` attributes.
//!
//! Arguments are resolved by the last segment of their type path, the same
//! way Tauri resolves them: `State` from managed state, `AppHandle` as a clone
//! of the app, `Window`/`Webview`/`WebviewWindow` as a placeholder, `Channel`
//! by the id the caller sent, `Request` from the arguments object, and every
//! other type deserialized from the argument named after the parameter
//! (camelCase unless `rename_all = "snake_case"`).

use heck::ToLowerCamelCase;
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Attribute, FnArg, Ident, ItemFn, LitStr, Pat, Path, Token, Type};

struct CommandAttr {
    snake_case: bool,
}

impl Parse for CommandAttr {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut snake_case = false;
        while !input.is_empty() {
            if input.peek(Token![async]) {
                input.parse::<Token![async]>()?;
            } else {
                let key = input.call(Ident::parse_any)?;
                if input.peek(Token![=]) {
                    input.parse::<Token![=]>()?;
                    let value: LitStr = input.parse()?;
                    if key == "rename_all" {
                        snake_case = value.value() == "snake_case";
                    }
                }
            }
            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }
        Ok(CommandAttr { snake_case })
    }
}

enum ArgKind {
    State,
    App,
    Window,
    Channel,
    Request,
    Value,
}

fn last_segment(ty: &Type) -> Option<String> {
    match ty {
        Type::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()),
        Type::Reference(r) => last_segment(&r.elem),
        Type::Group(g) => last_segment(&g.elem),
        Type::Paren(p) => last_segment(&p.elem),
        _ => None,
    }
}

fn classify(ty: &Type) -> ArgKind {
    match last_segment(ty).as_deref() {
        Some("State") => ArgKind::State,
        Some("AppHandle") => ArgKind::App,
        Some("Window") | Some("Webview") | Some("WebviewWindow") => ArgKind::Window,
        Some("Channel") => ArgKind::Channel,
        Some("Request") => ArgKind::Request,
        _ => ArgKind::Value,
    }
}

fn arg_key(pat: &Pat, snake_case: bool) -> String {
    let name = match pat {
        Pat::Ident(p) => p.ident.unraw().to_string(),
        _ => "_".to_string(),
    };
    if snake_case {
        name
    } else {
        name.to_lower_camel_case()
    }
}

#[proc_macro_attribute]
pub fn command(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = syn::parse_macro_input!(attr as CommandAttr);
    let item_fn = syn::parse_macro_input!(item as ItemFn);
    match expand_command(&attr, &item_fn) {
        Ok(hidden) => quote!(#item_fn #hidden).into(),
        Err(e) => {
            let err = e.to_compile_error();
            quote!(#item_fn #err).into()
        }
    }
}

fn expand_command(attr: &CommandAttr, item_fn: &ItemFn) -> syn::Result<TokenStream2> {
    let name = &item_fn.sig.ident;
    let vis = &item_fn.vis;
    let private = quote!(::sn_tauri_headless::__private);

    let mut lets = Vec::new();
    let mut call_args = Vec::new();
    let mut wants_request = false;
    for (i, input) in item_fn.sig.inputs.iter().enumerate() {
        let pt = match input {
            FnArg::Typed(pt) => pt,
            FnArg::Receiver(r) => {
                return Err(syn::Error::new_spanned(r, "a command cannot take `self`"));
            }
        };
        let var = format_ident!("__a{}", i);
        let key = arg_key(&pt.pat, attr.snake_case);
        let resolve = match classify(&pt.ty) {
            ArgKind::State => quote!(#private::state(&__app)),
            ArgKind::App => quote!(#private::app(&__app)),
            ArgKind::Window => quote!(#private::placeholder(&__app)),
            ArgKind::Channel => quote!(#private::channel(&__app, &__args, #key)),
            ArgKind::Request => {
                wants_request = true;
                quote!(#private::request(&__body, &__headers))
            }
            ArgKind::Value => quote!(#private::arg(&__args, #key)),
        };
        lets.push(quote! {
            let #var = match #resolve {
                ::core::result::Result::Ok(v) => v,
                ::core::result::Result::Err(e) => return ::core::result::Result::Err(e),
            };
        });
        call_args.push(var);
    }

    let request_locals = if wants_request {
        quote! {
            let __body = #private::body(&__args);
            let __headers = #private::HeaderMap::new();
        }
    } else {
        quote!()
    };

    let call = quote!(super::#name(#(#call_args),*));
    let finish = if item_fn.sig.asyncness.is_some() {
        quote! {
            let __ret = #call.await;
            (&__ret).__headless_kind().__headless_into(__ret)
        }
    } else {
        quote! {
            #private::blocking(move || {
                let __ret = #call;
                (&__ret).__headless_kind().__headless_into(__ret)
            })
            .await
        }
    };

    Ok(quote! {
        #[doc(hidden)]
        #[allow(non_snake_case, unused_variables, unused_mut, clippy::all)]
        #vis mod #name {
            pub fn __headless(
                __app: ::sn_tauri_headless::AppHandle,
                __args: #private::Value,
            ) -> #private::CmdFuture {
                ::std::boxed::Box::pin(async move {
                    #[allow(unused_imports)]
                    use #private::{PlainKind as _, ResultKind as _};
                    #request_locals
                    #(#lets)*
                    #finish
                })
            }
        }
    })
}

struct HandlerEntry {
    attrs: Vec<Attribute>,
    path: Path,
}

impl Parse for HandlerEntry {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Ok(HandlerEntry {
            attrs: input.call(Attribute::parse_outer)?,
            path: input.parse()?,
        })
    }
}

struct HandlerList(Punctuated<HandlerEntry, Token![,]>);

impl Parse for HandlerList {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Ok(HandlerList(Punctuated::parse_terminated(input)?))
    }
}

#[proc_macro]
pub fn generate_handler(input: TokenStream) -> TokenStream {
    let list = syn::parse_macro_input!(input as HandlerList);
    let private = quote!(::sn_tauri_headless::__private);
    let mut arms = Vec::new();
    for entry in list.0.iter() {
        let attrs = &entry.attrs;
        let path = &entry.path;
        let command_name = match path.segments.last() {
            Some(last) => last.ident.unraw().to_string(),
            None => continue,
        };
        arms.push(quote! {
            #(#attrs)*
            #command_name => ::core::option::Option::Some(#path::__headless(__app, __args)),
        });
    }
    quote! {
        ::sn_tauri_headless::ipc::Handler::__new(
            |__cmd: &str,
             __app: ::sn_tauri_headless::AppHandle,
             __args: #private::Value|
             -> ::core::option::Option<#private::CmdFuture> {
                #[allow(unreachable_patterns)]
                let __found = match __cmd {
                    #(#arms)*
                    _ => {
                        let _ = (__app, __args);
                        ::core::option::Option::None
                    }
                };
                __found
            },
        )
    }
    .into()
}

#[proc_macro]
pub fn generate_context(_input: TokenStream) -> TokenStream {
    quote! {
        ::sn_tauri_headless::Context::__new(
            ::core::env!("CARGO_PKG_NAME"),
            ::core::env!("CARGO_PKG_VERSION"),
        )
    }
    .into()
}

/// The Swift app calls the core through UniFFI, not through a generated
/// mobile entry point, so the function is left as written.
#[proc_macro_attribute]
pub fn mobile_entry_point(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}
