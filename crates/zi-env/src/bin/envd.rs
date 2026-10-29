//! zi-envd binary: `zi-envd serve --token <hex>`

use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("serve") => {
            let token = args
                .iter()
                .position(|arg| arg == "--token")
                .and_then(|index| args.get(index + 1));
            let Some(token) = token else {
                eprintln!("usage: zi-envd serve --token <hex>");
                std::process::exit(2);
            };
            if let Err(error) = zi_env::daemon::serve(token) {
                eprintln!("zi-envd: {error}");
                std::process::exit(1);
            }
        }
        Some("--version") => println!("zi-envd {}", env!("CARGO_PKG_VERSION")),
        _ => {
            eprintln!("usage: zi-envd serve --token <hex>");
            std::process::exit(2);
        }
    }
}
