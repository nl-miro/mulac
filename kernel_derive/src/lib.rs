use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{DeriveInput, Expr, Lit, Meta, parse_macro_input};

#[proc_macro_derive(ApplicationCommand, attributes(command_type))]
pub fn derive_application_command(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    derive_command(&input)
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

#[proc_macro_derive(ApplicationEvent, attributes(event_type))]
pub fn derive_application_event(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    derive_event(&input)
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

fn derive_command(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let name = &input.ident;
    let type_str = required_str_attr(&input.attrs, "command_type", "ApplicationCommand", name)?;

    Ok(quote! {
        impl ::kernel::ApplicationCommand for #name {
            fn command_type(&self) -> &'static str {
                #type_str
            }
        }

        impl #name {
            pub const COMMAND_TYPE: &'static str = #type_str;
        }
    })
}

fn derive_event(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let name = &input.ident;
    let type_str = required_str_attr(&input.attrs, "event_type", "ApplicationEvent", name)?;

    Ok(quote! {
        impl ::kernel::ApplicationEvent for #name {
            fn event_type(&self) -> &'static str {
                #type_str
            }
        }

        impl #name {
            pub const EVENT_TYPE: &'static str = #type_str;
        }
    })
}

fn required_str_attr(
    attrs: &[syn::Attribute],
    attr_name: &str,
    derive_name: &str,
    span: &impl quote::ToTokens,
) -> syn::Result<String> {
    for attr in attrs {
        if !attr.path().is_ident(attr_name) {
            continue;
        }
        let Meta::NameValue(nv) = &attr.meta else {
            return Err(syn::Error::new_spanned(
                attr,
                format!("expected #[{attr_name} = \"...\"]"),
            ));
        };
        let Expr::Lit(lit) = &nv.value else {
            return Err(syn::Error::new_spanned(
                &nv.value,
                "expected a string literal",
            ));
        };
        let Lit::Str(s) = &lit.lit else {
            return Err(syn::Error::new_spanned(
                &lit.lit,
                "expected a string literal",
            ));
        };
        return Ok(s.value());
    }
    Err(syn::Error::new_spanned(
        span,
        format!("#[derive({derive_name})] requires #[{attr_name} = \"...\"]"),
    ))
}
