use std::path::Path;
use std::process::Command;

fn main() {
    build_frontend();
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
