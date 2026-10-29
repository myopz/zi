//! Chord v2 kernel for **zi**: hosts, contracts, documents, store, links, jobs.
//!
//! Phase 7 skeleton aligned with `packages/chord/docs/next/` (slice 1 + stubs for later slices).

pub mod contracts;
pub mod data;
pub mod documents;
pub mod error;
pub mod host;
pub mod jobs;
pub mod link;
pub mod state;
pub mod store;

pub use contracts::{
    define_facet, Address, ContractKind, Facet, FacetHost, HostKind, LocalContract, MethodSpec, ParamField,
    PluginSpec, RemoteContract, ServeOptions,
};
pub use data::{default_principal_key, Principal, PrincipalKeyFn};
pub use error::{ChordError, ChordErrorKind, ChordResult};
pub use host::{
    connect_hosts, create_host, factory_async, factory_sync, CallContext, Host, HostError, HostHandle, HostOptions,
    LiveObject, ObjectMemberKind, ObjectMethod, Reference,
};
pub use jobs::{JobDefinition, JobHandle, JobQueue, JobRunId, JobStatus};
pub use link::{memory_link_pair, Link, LinkEnvelope, LinkMessage};
pub use state::{ReplicatedState, WatchHandle, Watchable};
pub use store::{InMemoryStore, StoreDocument, StoreError, StoreResult};
