//! Host extension wiring: discover/seed manifests; pack implementations live in
//! the `zi-host-extensions` crate (default `~/.zi/host-extensions`, see
//! `ZI_HOST_EXTENSIONS_PATH` / `scripts/link-host-extensions.sh`).

pub mod discover;
pub mod pack;

#[allow(unused_imports)]
pub use discover::{
    discover_enabled_dev_ext, discover_extensions, seed_default_builtin_manifests,
    DiscoveredExtension, EnabledDevExt, ExtensionKind, ALL_BUILTINS, BUILTIN_IMPORT_REPRO,
    BUILTIN_PROMPT_URL, BUILTIN_REDRAWS, BUILTIN_TPS,
};
#[allow(unused_imports)]
pub use pack::PackHost;
#[allow(unused_imports)]
pub use zi_host_extensions::{HostExtensionPack, PackHookEffects, PromptMatch, TurnUsage};

/// Compatibility module paths previously under `dev_ext::{tps,prompt_url,import_repro}`.
pub mod tps {
    #[allow(unused_imports)]
    pub use zi_host_extensions::{format_tps, format_tps_usage, sum_turn_usage, TurnUsage};
}

pub mod prompt_url {
    #[allow(unused_imports)]
    pub use zi_host_extensions::{
        desired_session_name, extract_prompt_match, fetch_gh_metadata, format_widget,
        match_from_messages, should_update_session_name, GhMetadata, PromptKind, PromptMatch,
    };
}

pub mod import_repro {
    #[allow(unused_imports)]
    pub use zi_host_extensions::{
        import_session, parse_ref, session_dir_for_cwd, ImportRef, ImportedSession,
    };
}
