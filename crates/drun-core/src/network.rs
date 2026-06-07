//! Network access policy passed at session creation to control what the sandbox
//! can reach.

#[derive(Clone, Copy)]
pub enum NetworkPolicy {
    Packages, // allow only package registries (default)
    Full,     // unrestricted network access
    None,     // no network
}
