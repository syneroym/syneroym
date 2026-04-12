use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    // Only build the component if we are in a testing or dev environment
    // to avoid forcing all users building syneroym to install cargo-component.
    if env::var("PROFILE").unwrap_or_default() == "release" {
        return;
    }

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let component_dir = PathBuf::from(manifest_dir).join("../../test-components/introducer");

    // Tell Cargo to re-run this script if the component's source changes
    println!("cargo:rerun-if-changed={}", component_dir.join("src").display());
    println!("cargo:rerun-if-changed={}", component_dir.join("wit").display());
    println!("cargo:rerun-if-changed={}", component_dir.join("Cargo.toml").display());

    let status = Command::new(env!("CARGO"))
        .arg("component")
        .arg("build")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("CARGO_MAKEFLAGS")
        .env("CARGO_TARGET_DIR", component_dir.join("target"))
        .current_dir(&component_dir)
        .status()
        .expect("Failed to execute cargo component build");

    if !status.success() {
        panic!("Failed to build the WASM component in {:?}", component_dir);
    }
}
