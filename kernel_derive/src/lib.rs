use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::Span;
use quote::quote;
use syn::{DeriveInput, Ident, parse_macro_input};

/// Path to the kernel crate as seen from the crate invoking the derive.
///
/// Consumers usually rename the dependency (`kernel = { package = "mulac-kernel" }`),
/// so the name is looked up in the caller's Cargo.toml instead of being hard-coded.
/// Inside mulac-kernel itself (and its tests) `::mulac_kernel` resolves via
/// `extern crate self as mulac_kernel`.
fn kernel_path() -> proc_macro2::TokenStream {
    match crate_name("mulac-kernel") {
        Ok(FoundCrate::Name(name)) => {
            let ident = Ident::new(&name, Span::call_site());
            quote!(::#ident)
        }
        Ok(FoundCrate::Itself) | Err(_) => quote!(::mulac_kernel),
    }
}

/// Derive macro for ApplicationCommand trait.
///
/// Automatically implements ApplicationCommand by generating a command_type() method
/// that returns the name of the struct.
#[proc_macro_derive(ApplicationCommand)]
pub fn derive_application_command(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let command_type_str = name.to_string();
    let kernel = kernel_path();

    let expanded = quote! {
        impl #kernel::ApplicationCommand for #name {
            fn command_type(&self) -> &'static str {
                #command_type_str
            }
        }
    };

    TokenStream::from(expanded)
}

/// Derive macro for ApplicationEvent trait.
///
/// Automatically implements ApplicationEvent by generating an event_type() method
/// that returns the name of the struct.
#[proc_macro_derive(ApplicationEvent)]
pub fn derive_application_event(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let event_type_str = name.to_string();
    let kernel = kernel_path();

    let expanded = quote! {
        impl #kernel::ApplicationEvent for #name {
            fn event_type(&self) -> &'static str {
                #event_type_str
            }
        }
    };

    TokenStream::from(expanded)
}
