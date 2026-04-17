use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    // Only build the component if we are in a testing or dev environment
    // to avoid forcing all users building syneroym to install cargo-component.
    if env::var("PROFILE").unwrap_or_default() == "release" {
        return;
    }

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    let component_dir = PathBuf::from(&manifest_dir).join("../../test-components/greeter");

    // Tell Cargo to re-run this script if the component's source changes
    println!("cargo:rerun-if-changed={}", component_dir.join("src").display());
    println!("cargo:rerun-if-changed={}", component_dir.join("wit").display());
    println!("cargo:rerun-if-changed={}", component_dir.join("Cargo.toml").display());

    // Build WASM component. Failure is non-fatal: print a warning and continue.
    // This allows builds to succeed on environments without cargo-component installed.
    let status = Command::new(env!("CARGO"))
        .arg("component")
        .arg("build")
        .arg("--release")
        .arg("--target")
        .arg("wasm32-wasip2")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("CARGO_MAKEFLAGS")
        .env("CARGO_TARGET_DIR", component_dir.join("target"))
        .current_dir(&component_dir)
        .status();

    match status {
        Ok(s) if s.success() => {
            println!("cargo:info=WASM component built successfully");
        }
        Ok(s) => {
            println!(
                "cargo:warning=Failed to build WASM component in {:?} : {}. \
                 Install cargo-component to rebuild: `cargo install cargo-component`. \
                 Continuing build without updated WASM component.",
                component_dir, s
            );
        }
        Err(e) => {
            println!(
                "cargo:warning=Failed to execute WASM component build: {}. \
                 Ensure 'cargo' is available and cargo-component is installed. \
                 Continuing build without WASM component.",
                e
            );
        }
    }
}
