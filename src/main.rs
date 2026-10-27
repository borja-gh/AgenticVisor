//! Binary entry point.
//!
//!   agenticvisor                 starts the dashboard at http://127.0.0.1:8756
//!   agenticvisor --port 9000     otro puerto
//!   agenticvisor --no-browser    does not open the browser
//!   agenticvisor scan-json       prints the scan as JSON (useful for scripts)
//!   agenticvisor usage-json      prints token usage as JSON

use std::path::PathBuf;
use std::process::Command;

use agenticvisor::scanner::scan_json;
use agenticvisor::search::index::new_shared;
use agenticvisor::server::{serve, start_initial_index, Ctx};
use agenticvisor::usage;

const DEFAULT_PORT: u16 = 8756;

/// The dashboard's own data folder (trash, backups...). Can be changed
/// with AGENTICVISOR_DATA, for example in tests.
fn data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("AGENTICVISOR_DATA") {
        return PathBuf::from(dir);
    }
    dirs::data_local_dir()
        .unwrap_or_else(|| home().join(".local").join("share"))
        .join("agenticvisor")
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

fn open_browser(url: &str) {
    let result = if cfg!(target_os = "macos") {
        Command::new("open").arg(url).status()
    } else if cfg!(target_os = "windows") {
        Command::new("cmd").args(["/C", "start", "", url]).status()
    } else {
        Command::new("xdg-open").arg(url).status()
    };
    if result.is_err() {
        println!("(could not open the browser: open {url} manually)");
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("scan-json") => {
            println!("{}", serde_json::to_string_pretty(&scan_json(&home())).unwrap_or_default());
            return;
        }
        Some("usage-json") => {
            let records = usage::collect(&home());
            if args.iter().any(|a| a == "--per-session") {
                let rows: Vec<_> = records.iter().map(|r| serde_json::json!({"id": r.id, "by_model": r.usage.by_model})).collect();
                println!("{}", serde_json::to_string_pretty(&rows).unwrap_or_default());
            } else {
                println!("{}", serde_json::to_string_pretty(&usage::summary(&records)).unwrap_or_default());
            }
            return;
        }
        _ => {}
    }

    let mut port = DEFAULT_PORT;
    let mut open = true;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--port" => {
                port = iter.next().and_then(|p| p.parse().ok()).unwrap_or(DEFAULT_PORT);
            }
            "--no-browser" => open = false,
            "--version" | "-V" => {
                println!("agenticvisor {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "--help" | "-h" => {
                println!("agenticvisor [--port N] [--no-browser] | scan-json | usage-json | --version");
                return;
            }
            other => {
                eprintln!("Unknown option: {other}");
                std::process::exit(2);
            }
        }
    }

    let addr = format!("127.0.0.1:{port}");
    let url = format!("http://{addr}");
    println!("Dashboard at {url}  (Ctrl+C to stop)");
    if open {
        open_browser(&url);
    }
    let ctx = Ctx { home: home(), data_dir: data_dir(), index: new_shared() };
    start_initial_index(&ctx);
    if let Err(err) = serve(&addr, ctx) {
        eprintln!("Could not start on {addr}: {err}");
        std::process::exit(1);
    }
}
