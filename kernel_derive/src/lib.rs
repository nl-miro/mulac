use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput};

/// Derive macro for ApplicationCommand trait.
///
/// Automatically implements ApplicationCommand by generating a command_type() method
/// that returns the name of the struct.
#[proc_macro_derive(ApplicationCommand)]
pub fn derive_application_command(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let command_type_str = name.to_string();

    let expanded = quote! {
        impl mulac_kernel::ApplicationCommand for #name {
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

    let expanded = quote! {
        impl mulac_kernel::ApplicationEvent for #name {
            fn event_type(&self) -> &'static str {
                #event_type_str
            }
        }
    };

    TokenStream::from(expanded)
}
