// Contract handles and facet declarations (design §5, §16). Slice 7: accepts / evolve.

pub(crate) mod contract;
pub(crate) mod facet;
pub(crate) mod host_kind;

pub use contract::{
    ContractId, ContractKind, ContractSpec, LocalContract, MethodSpec, ParamField, RemoteContract, LOCAL_VARIANT,
};
pub use facet::{define_facet, Facet, FacetHost, PluginSpec, ServeOptions};
pub use host_kind::{Address, HostKind, HostKindId};
