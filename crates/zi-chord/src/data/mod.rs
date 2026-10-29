// Wire-safe data: principals, identity keys (L1). Slice 3a+: link-stamped principals.

pub(crate) mod identity;
mod principal;

pub use identity::{identity_key, IdentityKey};
pub use principal::{default_principal_key, Principal, PrincipalKeyFn};
