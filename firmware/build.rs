use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    build_frontend();
    embed_bundled_ca();
    embuild::espidf::sysenv::output();
}

/// firmware/src/net/ap_portal.rs embeds `portal-ui/dist/*` via `include_bytes!` at compile
/// time. Building the frontend here - rather than requiring a manual `pnpm build` step before
/// `cargo build` - is what this project's own constitution (Principle VI: reproducible build
/// environment) calls for: a fresh checkout should build correctly without an undocumented
/// two-step dance.
fn build_frontend() {
    let portal_ui_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("portal-ui");

    for path in ["src", "index.html", "vite.config.ts", "package.json", "components.json"] {
        println!("cargo:rerun-if-changed={}", portal_ui_dir.join(path).display());
    }

    if !portal_ui_dir.join("node_modules").exists() {
        run(&portal_ui_dir, "pnpm", &["install"]);
    }
    run(&portal_ui_dir, "pnpm", &["build"]);
}

/// Optional escape hatch for networks where Home Assistant's hostname resolves to a certificate
/// signed by a private CA (e.g. a UniFi/UDM controller's own local CA for LAN-only HTTPS) rather
/// than a public one - see firmware/src/ha/rest.rs's `try_connect` for how this gets used as a
/// pinned trust anchor. `BUNDLED_CA_PATH` (repo-root `.env`, or an actual env var which takes
/// precedence) names a PEM file, resolved relative to the repo root if not absolute. Left
/// unset/blank, this embeds an empty file and `rest.rs` treats that as "no bundled CA".
fn embed_bundled_ca() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("firmware/ should have a parent directory")
        .to_path_buf();
    let dotenv_path = repo_root.join(".env");
    println!("cargo:rerun-if-changed={}", dotenv_path.display());

    let bundled_ca_path = std::env::var("BUNDLED_CA_PATH")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| read_dotenv_var(&dotenv_path, "BUNDLED_CA_PATH"));

    let mut pem_bytes = Vec::new();
    if let Some(path) = bundled_ca_path {
        let resolved = resolve_path(&repo_root, &path);
        println!("cargo:rerun-if-changed={}", resolved.display());
        pem_bytes = fs::read(&resolved).unwrap_or_else(|e| {
            panic!(
                "BUNDLED_CA_PATH is set to '{path}' (resolved: {}) but it could not be read: {e}",
                resolved.display()
            )
        });
        // `X509::pem_until_nul` in rest.rs scans for the first NUL byte, so the buffer must end
        // with exactly one - append it if the source file (as most PEM files do) lacks one.
        if pem_bytes.last() != Some(&0) {
            pem_bytes.push(0);
        }
    }

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR set by cargo");
    let out_path = PathBuf::from(out_dir).join("bundled_ca.pem");
    fs::write(&out_path, &pem_bytes)
        .unwrap_or_else(|e| panic!("failed to write {}: {e}", out_path.display()));
    println!("cargo:rustc-env=BUNDLED_CA_PEM_PATH={}", out_path.display());
}

fn resolve_path(repo_root: &Path, path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        repo_root.join(p)
    }
}

/// Minimal `KEY=value` parser for the repo-root `.env` file - just enough to read
/// `BUNDLED_CA_PATH` without adding a `dotenvy`-style dependency for one optional variable.
fn read_dotenv_var(dotenv_path: &Path, key: &str) -> Option<String> {
    let contents = fs::read_to_string(dotenv_path).ok()?;
    for line in contents.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix(key) else {
            continue;
        };
        let Some(value) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"');
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

fn run(dir: &Path, cmd: &str, args: &[&str]) {
    let status = Command::new(cmd)
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap_or_else(|e| {
            panic!(
                "failed to run `{cmd} {}` in {}: {e} - is pnpm installed and on PATH?",
                args.join(" "),
                dir.display()
            )
        });
    if !status.success() {
        panic!(
            "`{cmd} {}` in {} exited with {status}",
            args.join(" "),
            dir.display()
        );
    }
}
