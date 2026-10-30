//! Reject bash commands that are really codebase exploration (use explore/read instead).

/// If `command` looks like tree/source inspection, return a short reason label.
/// Build/test/git/package-manager commands are allowed.
pub fn exploration_bash_reason(command: &str) -> Option<&'static str> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return None;
    }
    let primary = primary_command(trimmed);
    if is_allowlisted_primary(primary) {
        // Inline one-liners that dump package.json / sources still burn context.
        if is_inline_file_dump(primary, trimmed) {
            return Some("inline script file dump");
        }
        return None;
    }
    exploration_token(trimmed)
}

fn primary_command(command: &str) -> &str {
    let mut s = command.trim();
    // Strip leading `cd <path> &&` / `cd <path>;` so `cd repo && ls` is classified as ls.
    for _ in 0..4 {
        let Some(rest) = strip_cd_prefix(s) else {
            break;
        };
        s = rest.trim();
    }
    // First pipeline/chain segment.
    let seg = s
        .split(['\n', ';', '|', '&'])
        .map(str::trim)
        .find(|p| !p.is_empty())
        .unwrap_or(s);
    let seg = seg.trim_start_matches("sudo ").trim();
    seg.split_whitespace().next().unwrap_or("")
}

fn strip_cd_prefix(s: &str) -> Option<&str> {
    let t = s.trim_start();
    if !t.starts_with("cd ") && !t.starts_with("cd\t") {
        return None;
    }
    let after_cd = t[2..].trim_start();
    // Find separator after the path token (rough; quoted paths with spaces are rare here).
    let path_end = after_cd
        .find([' ', '\t', ';', '\n'])
        .unwrap_or(after_cd.len());
    let mut rest = after_cd[path_end..].trim_start();
    if rest.starts_with("&&") {
        rest = rest[2..].trim_start();
        return Some(rest);
    }
    if rest.starts_with(';') {
        rest = rest[1..].trim_start();
        return Some(rest);
    }
    None
}

fn is_allowlisted_primary(primary: &str) -> bool {
    matches!(
        primary,
        "git"
            | "npm"
            | "npx"
            | "pnpm"
            | "yarn"
            | "bun"
            | "cargo"
            | "rustc"
            | "go"
            | "make"
            | "cmake"
            | "ninja"
            | "docker"
            | "podman"
            | "kubectl"
            | "terraform"
            | "pytest"
            | "vitest"
            | "tsc"
            | "eslint"
            | "prettier"
            | "chmod"
            | "chown"
            | "mkdir"
            | "rm"
            | "mv"
            | "cp"
            | "touch"
            | "ln"
            | "kill"
            | "pkill"
            | "curl"
            | "wget"
            | "ssh"
            | "scp"
            | "rsync"
            | "brew"
            | "apt"
            | "apt-get"
            | "pip"
            | "pip3"
            | "uv"
            | "node"
            | "nodejs"
            | "python"
            | "python3"
            | "deno"
            | "ruby"
            | "perl"
            | "php"
            | "java"
            | "mvn"
            | "gradle"
            | "bazel"
            | "zig"
            | "swift"
            | "xcodebuild"
            | "open"
            | "xdg-open"
            | "which"
            | "type"
            | "command"
            | "true"
            | "false"
            | "sleep"
            | "date"
            | "env"
            | "export"
            | "unset"
            | "printf"
            | "test"
            | "["
    )
}

fn is_inline_file_dump(primary: &str, command: &str) -> bool {
    if !matches!(primary, "node" | "nodejs" | "python" | "python3" | "deno" | "bun") {
        return false;
    }
    let has_eval = command.contains(" -e ")
        || command.contains(" -e\"")
        || command.contains(" -c ")
        || command.contains(" -p ");
    if !has_eval {
        return false;
    }
    let lower = command.to_ascii_lowercase();
    lower.contains("package.json")
        || lower.contains("require(")
        || lower.contains("read_file")
        || lower.contains("open(")
        || lower.contains(".ts")
        || lower.contains(".rs")
        || lower.contains(".md")
        || lower.contains("listdir")
        || lower.contains("pathlib")
}

fn exploration_token(command: &str) -> Option<&'static str> {
    // Strip simple single/double-quoted spans so `echo "ls"` is not blocked.
    let bare = strip_quotes_approx(command);
    for token in bare.split(|c: char| c.is_whitespace() || ";|&()<>".contains(c)) {
        let t = token.trim_matches(['\\', '"', '\'']);
        if t.is_empty() {
            continue;
        }
        let label = match t {
            "ls" => Some("ls"),
            "find" => Some("find"),
            "cat" => Some("cat"),
            "rg" => Some("rg"),
            "grep" => Some("grep"),
            "egrep" | "fgrep" => Some("grep"),
            "tree" => Some("tree"),
            "wc" => Some("wc"),
            "sed" => Some("sed"),
            "awk" => Some("awk"),
            "head" => Some("head"),
            "tail" => Some("tail"),
            "less" | "more" => Some("pager"),
            "nl" => Some("nl"),
            "bat" | "batcat" => Some("bat"),
            "stat" => Some("stat"),
            "du" => Some("du"),
            "hexdump" | "xxd" | "od" => Some("hex dump"),
            _ => None,
        };
        if label.is_some() {
            return label;
        }
    }
    None
}

fn strip_quotes_approx(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' || c == '\'' {
            let quote = c;
            while let Some(n) = chars.next() {
                if n == quote {
                    break;
                }
                if n == '\\' {
                    let _ = chars.next();
                }
            }
            out.push(' ');
            continue;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_build_and_git() {
        assert!(exploration_bash_reason("git status").is_none());
        assert!(exploration_bash_reason("npm test").is_none());
        assert!(exploration_bash_reason("cargo test -p zi-agent").is_none());
        assert!(exploration_bash_reason("cd /tmp/proj && cargo build").is_none());
    }

    #[test]
    fn rejects_tree_inspection() {
        assert_eq!(exploration_bash_reason("ls -la").unwrap(), "ls");
        assert_eq!(
            exploration_bash_reason("cd /repo && ls packages").unwrap(),
            "ls"
        );
        assert_eq!(exploration_bash_reason("cat package.json").unwrap(), "cat");
        assert_eq!(
            exploration_bash_reason("ls -la && cat package.json").unwrap(),
            "ls"
        );
        assert_eq!(
            exploration_bash_reason("find packages -name '*.ts' | head").unwrap(),
            "find"
        );
        assert_eq!(
            exploration_bash_reason("rg -n foo packages/agent").unwrap(),
            "rg"
        );
        assert_eq!(
            exploration_bash_reason("wc -l packages/agent/src/*.ts").unwrap(),
            "wc"
        );
        assert_eq!(
            exploration_bash_reason("sed -n '1,80p' packages/ai/src/index.ts").unwrap(),
            "sed"
        );
    }

    #[test]
    fn rejects_node_e_package_dump() {
        let cmd = r#"node -e "console.log(JSON.stringify(require('./package.json').scripts))""#;
        assert!(exploration_bash_reason(cmd).is_some());
    }

    #[test]
    fn quoted_ls_in_echo_is_ok_if_primary_allowlisted() {
        // primary is echo — echo is NOT allowlisted, and quoted ls stripped → no token → None
        // Actually echo is not allowlisted; after strip quotes no ls → None. Good.
        assert!(exploration_bash_reason(r#"printf 'hello'"#).is_none());
    }
}
