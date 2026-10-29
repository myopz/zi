// Facets (design §16): synchronous setup only; no behaviour on the facet itself.

use std::sync::Arc;

use super::contract::{ContractSpec, LocalContract, RemoteContract};
use crate::host::object::ObjectFactory;

#[derive(Clone, Default)]
pub struct ServeOptions {
    pub linger_ms: u64,
}

struct ServedEntry {
    spec: Arc<ContractSpec>,
    factory: ObjectFactory,
    options: ServeOptions,
}

/// Setup surface while a facet runs.
pub struct FacetHost {
    open: bool,
    served: Vec<ServedEntry>,
}

impl FacetHost {
    pub fn serve_local(&mut self, contract: LocalContract, factory: ObjectFactory, options: ServeOptions) {
        self.serve(contract.spec, factory, options);
    }

    pub fn serve_remote(&mut self, contract: RemoteContract, factory: ObjectFactory, options: ServeOptions) {
        self.serve(contract.spec, factory, options);
    }

    fn serve(&mut self, spec: Arc<ContractSpec>, factory: ObjectFactory, options: ServeOptions) {
        if !self.open {
            panic!("host.serve is valid only during facet setup");
        }
        self.served.push(ServedEntry {
            spec,
            factory,
            options,
        });
    }

    pub(crate) fn take_served(self) -> Vec<(Arc<ContractSpec>, ObjectFactory, ServeOptions)> {
        self.served
            .into_iter()
            .map(|e| (e.spec, e.factory, e.options))
            .collect()
    }
}

/// Plugin facet: runs once at `load`.
pub struct Facet {
    setup: Arc<dyn Fn(&mut FacetHost) + Send + Sync>,
}

/// Defines a facet from a setup closure.
pub fn define_facet(setup: impl Fn(&mut FacetHost) + Send + Sync + 'static) -> Facet {
    Facet {
        setup: Arc::new(setup),
    }
}

pub struct PluginSpec {
    pub name: String,
    pub facet: Facet,
}

/// Runs facet setup and returns served types (used by `Host::load`).
pub(crate) fn set_up_facet(name: &str, facet: &Facet) -> Vec<(Arc<ContractSpec>, ObjectFactory, ServeOptions)> {
    let mut host = FacetHost {
        open: true,
        served: Vec::new(),
    };
    let setup = facet.setup.clone();
    setup(&mut host);
    host.open = false;
    let _ = name;
    host.take_served()
}
