//! Load `.env` files into the process environment (no override of existing vars).

use std::path::Path;

use anyhow::{Context, Result};

/// Load `<cwd>/.env` then `<agent_dir>/.env`.
///
/// Existing process environment variables are never overwritten. Loading cwd
/// first then agent means project keys win over user defaults for unset vars.
pub fn load_env_files(cwd: &Path, agent_dir: &Path) -> Result<()> {
    load_one(&cwd.join(".env"))?;
    load_one(&agent_dir.join(".env"))?;
    Ok(())
}

fn load_one(path: &Path) -> Result<()> {
    if !path.is_file() {
        return Ok(());
    }
    dotenvy::from_path(path).with_context(|| format!("failed to load {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn unique_key(suffix: &str) -> String {
        format!(
            "ZI_TEST_DOTENV_{}_{}",
            std::process::id(),
            suffix.replace('-', "_")
        )
    }

    #[test]
    fn missing_files_ok() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = tempfile_dir("missing");
        load_env_files(&dir, &dir).unwrap();
    }

    #[test]
    fn cwd_wins_over_agent_when_unset() {
        let _g = ENV_LOCK.lock().unwrap();
        let key = unique_key("cwd_wins");
        std::env::remove_var(&key);

        let root = tempfile_dir("cwd_wins");
        let cwd = root.join("cwd");
        let agent = root.join("agent");
        fs::create_dir_all(&cwd).unwrap();
        fs::create_dir_all(&agent).unwrap();
        fs::write(cwd.join(".env"), format!("{key}=from-cwd\n")).unwrap();
        fs::write(agent.join(".env"), format!("{key}=from-agent\n")).unwrap();

        load_env_files(&cwd, &agent).unwrap();
        assert_eq!(std::env::var(&key).unwrap(), "from-cwd");
        std::env::remove_var(&key);
    }

    #[test]
    fn process_env_not_overridden() {
        let _g = ENV_LOCK.lock().unwrap();
        let key = unique_key("process_wins");
        std::env::set_var(&key, "from-process");

        let root = tempfile_dir("process");
        let cwd = root.join("cwd");
        fs::create_dir_all(&cwd).unwrap();
        fs::write(cwd.join(".env"), format!("{key}=from-file\n")).unwrap();

        load_env_files(&cwd, &root.join("agent-missing")).unwrap();
        assert_eq!(std::env::var(&key).unwrap(), "from-process");
        std::env::remove_var(&key);
    }

    #[test]
    fn invalid_syntax_errors() {
        let _g = ENV_LOCK.lock().unwrap();
        let root = tempfile_dir("bad");
        let cwd = root.join("cwd");
        fs::create_dir_all(&cwd).unwrap();
        // Unquoted value with spaces is invalid in dotenvy.
        fs::write(cwd.join(".env"), "BAD KEY=value\n").unwrap();
        let err = load_env_files(&cwd, &root.join("agent")).unwrap_err();
        assert!(err.to_string().contains("failed to load"), "{err:#}");
    }

    fn tempfile_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "zi-env-file-{}-{}-{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}
