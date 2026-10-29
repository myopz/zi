//! Workspace maintenance tasks (catalog generation, checks).

use std::env;
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();
    let sub = args.get(1).map(String::as_str).unwrap_or("help");

    match sub {
        "check" => cmd_check(),
        "gen-catalog" => cmd_gen_catalog_stub(),
        "help" | "-h" | "--help" => print_help(),
        other => {
            eprintln!("xtask: unknown command {other:?}");
            print_help();
            process::exit(1);
        }
    }
}

fn print_help() {
    eprintln!(
        "Usage: cargo run -p xtask -- <command>\n\
         \n\
         Commands:\n\
           check        Smoke notes for workspace/docs (stub)\n\
           gen-catalog  Model catalog generation (not implemented)\n\
           help         This message\n\
         \n\
         See xtask/README.md for catalog generation plans."
    );
}

/// Stub for CI/docs: verify xtask is wired; full checks live in evals/smoke.sh and `cargo test`.
fn cmd_check() {
    eprintln!("xtask check: ok (stub)");
    eprintln!("  - eval smoke: ./evals/smoke.sh");
    eprintln!("  - catalog: pending `gen-catalog` (see xtask/README.md)");
}

fn cmd_gen_catalog_stub() {
    eprintln!("xtask gen-catalog: not implemented yet");
    eprintln!("  Planned: emit zi-ai catalog from shared definitions; use --check in CI.");
    process::exit(2);
}
