// Host kernel (design §3). Slice 2+: failures, natives; slice 4a+: routes and relays.

mod ctx;
mod host;
pub(crate) mod object;
mod reference;

pub use ctx::{CallContext, HostContext};
pub use host::{connect_hosts, create_host, Host, HostError, HostHandle, HostOptions};
pub use object::{factory_async, factory_sync, LiveObject, ObjectFactory, ObjectMemberKind, ObjectMethod};
pub use reference::Reference;

// TODO(slice 2): activation failures, backoff, closeTimeout, readyTimeout.
// TODO(slice 3a): remote objects, principals on links, ctx.get.
// TODO(slice 4a): host.route, placement routing, connect/accept handshake.
// TODO(slice 4b): revocation, host.references, host.activity.
// TODO(slice 7): reload, revisions, accepts mappings.
// TODO(slice 8): host.root, registries.
