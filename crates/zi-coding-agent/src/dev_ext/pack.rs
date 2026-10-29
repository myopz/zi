//! PackHost: load enabled host packs from the external `zi-host-extensions` crate.

use std::collections::HashSet;
use std::sync::Arc;

use serde_json::Value;
use zi_host_extensions::{
    instantiate, ExtensionHook, HostExtensionPack, PackCommandMeta, PackHookEffects,
    PackLoadContext, ALL_BUILTINS,
};

use super::discover::{DiscoveredExtension, EnabledDevExt};

/// Loaded host packs + their contributions (implementations live in `zi-host-extensions`).
pub struct PackHost {
    packs: Vec<Arc<dyn HostExtensionPack>>,
    enabled_ids: HashSet<String>,
    commands: Vec<PackCommandMeta>,
    /// (pack_id, hook)
    hook_subs: Vec<(String, ExtensionHook)>,
    sources: Vec<std::path::PathBuf>,
    discovered: Vec<DiscoveredExtension>,
}

impl PackHost {
    /// Instantiate packs listed as enabled by discover.
    pub fn from_enabled(enabled: &EnabledDevExt) -> Self {
        let mut loaded: Vec<Arc<dyn HostExtensionPack>> = Vec::new();
        let mut enabled_ids = HashSet::new();

        let want: HashSet<&str> = ALL_BUILTINS
            .iter()
            .copied()
            .filter(|id| match *id {
                "tps" => enabled.tps,
                "redraws" => enabled.redraws,
                "prompt-url" => enabled.prompt_url,
                "import-repro" => enabled.import_repro,
                _ => false,
            })
            .collect();

        for id in ALL_BUILTINS {
            if !want.contains(id) {
                continue;
            }
            let Some(pack) = instantiate(id) else {
                continue;
            };
            enabled_ids.insert(id.to_string());
            loaded.push(pack);
        }

        let mut commands = Vec::new();
        let mut hook_subs = Vec::new();
        for pack in &loaded {
            let mut ctx = PackLoadContext::default();
            pack.load(&mut ctx);
            for cmd in ctx.commands {
                commands.push(cmd);
            }
            for hook in ctx.hooks {
                hook_subs.push((pack.id().to_string(), hook));
            }
        }

        Self {
            packs: loaded,
            enabled_ids,
            commands,
            hook_subs,
            sources: enabled.sources.clone(),
            discovered: enabled.all.clone(),
        }
    }

    pub fn is_enabled(&self, id: &str) -> bool {
        self.enabled_ids.contains(id)
    }

    pub fn provides_command(&self, name: &str) -> bool {
        let lower = name.to_ascii_lowercase();
        self.commands.iter().any(|c| c.name == lower)
    }

    pub fn commands(&self) -> &[PackCommandMeta] {
        &self.commands
    }

    pub fn enabled_ids(&self) -> impl Iterator<Item = &str> {
        self.enabled_ids.iter().map(String::as_str)
    }

    pub fn sources(&self) -> &[std::path::PathBuf] {
        &self.sources
    }

    pub fn discovered(&self) -> &[DiscoveredExtension] {
        &self.discovered
    }

    /// Human-readable listing for `/extensions`.
    pub fn list_text(&self) -> String {
        let mut lines = vec![self.summary_line()];
        if self.discovered.is_empty() {
            lines.push(
                "No extension.json manifests found (builtins default on; WASM via *.wasm files)."
                    .into(),
            );
            return lines.join("\n");
        }
        lines.push("Manifests:".into());
        for ext in &self.discovered {
            let status = if ext.enabled { "on" } else { "off" };
            let desc = ext.description.as_deref().unwrap_or("");
            let mut line = format!(
                "  - {} [{}] {} api={}",
                ext.id,
                ext.kind.as_str(),
                status,
                ext.api_version
            );
            if !desc.is_empty() {
                line.push_str(" — ");
                line.push_str(desc);
            }
            lines.push(line);
            if !ext.contributes.prompts.is_empty() || !ext.contributes.skills.is_empty() {
                lines.push(format!(
                    "      contributes prompts={:?} skills={:?}",
                    ext.contributes.prompts, ext.contributes.skills
                ));
            }
        }
        if !self.commands.is_empty() {
            lines.push("Slash commands from packs:".into());
            for cmd in &self.commands {
                lines.push(format!("  /{} — {}", cmd.name, cmd.summary));
            }
        }
        lines.join("\n")
    }

    pub fn summary_line(&self) -> String {
        let mut ids: Vec<_> = self.enabled_ids.iter().cloned().collect();
        ids.sort();
        if ids.is_empty() {
            "host packs: (none)".into()
        } else {
            format!("host packs: {}", ids.join(", "))
        }
    }

    /// Dispatch `hook` to subscribed packs; returns aggregated effects.
    pub fn dispatch_hook(&self, hook: ExtensionHook, payload: &Value) -> PackHookEffects {
        let mut effects = PackHookEffects::default();
        for pack in &self.packs {
            let subscribed = self
                .hook_subs
                .iter()
                .any(|(id, h)| id == pack.id() && *h == hook);
            if !subscribed {
                continue;
            }
            pack.on_hook(hook, payload, &mut effects);
        }
        effects
    }
}
