//! Smoke test for examples/wasm-extension-hello (ABI v1).

#[cfg(all(test, feature = "wasm-extensions"))]
mod tests {
    use std::path::PathBuf;
    use std::sync::{Arc, RwLock};

    use crate::extensions::wasm_runtime::load_wasm_extension;
    use crate::extensions::{
        ExtensionHook, ExtensionLoadContext, ExtensionRegistry, ExtensionUiEffects,
    };

    fn hello_wat() -> &'static str {
        include_str!("../../../../examples/wasm-extension-hello/hello.wat")
    }

    #[test]
    fn wasm_hello_registers_command_and_notifies_on_agent_end() {
        let bytes = wat::parse_str(hello_wat()).expect("assemble hello.wat");
        let ui = Arc::new(RwLock::new(ExtensionUiEffects::default()));
        let tools = Arc::new(RwLock::new(Vec::new()));
        let path = PathBuf::from("hello.wasm");
        let ext = load_wasm_extension(&path, &bytes, tools, Arc::clone(&ui), Some("hello"))
            .expect("load hello wasm");

        let mut registry = ExtensionRegistry::default();
        {
            let extension_id = ext.id().to_string();
            let mut ctx = ExtensionLoadContext {
                registry: &mut registry,
                extension_id,
            };
            ext.load(&mut ctx);
        }

        assert!(
            registry.commands.iter().any(|c| c.name == "hello"),
            "expected /hello command, got {:?}",
            registry.commands
        );
        assert!(
            registry
                .hooks
                .iter()
                .any(|h| h.hook == ExtensionHook::AgentEnd),
            "expected agent_end subscription"
        );

        {
            let mut guard = ui.write().unwrap();
            *guard = ExtensionUiEffects::default();
        }
        ext.on_hook(ExtensionHook::AgentEnd, &serde_json::json!({}));
        let effects = ui.read().unwrap().clone();
        assert!(
            effects
                .notices
                .iter()
                .any(|n| n.contains("hello from wasm")),
            "expected ui_notify notice, got {:?}",
            effects.notices
        );

        let reply = ext.on_command("hello", "").expect("command reply");
        assert!(reply.contains("hello from /hello"), "got {reply}");
    }
}
