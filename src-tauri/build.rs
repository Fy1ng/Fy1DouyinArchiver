#[path = "build_support/frontend_manifest.rs"]
mod frontend_manifest;
#[path = "build_support/package_identity.rs"]
mod package_identity;

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=EMILY_UPDATER_ENDPOINT");
    println!("cargo:rerun-if-env-changed=EMILY_UPDATER_PUBKEY");
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is required"));
    let root = manifest_dir.parent().expect("repository root is required");
    frontend_manifest::validate_frontend(root, true)
        .unwrap_or_else(|error| panic!("frontend manifest validation failed: {error}"));
    let identity = package_identity::read_package_identity(&manifest_dir)
        .unwrap_or_else(|error| panic!("package identity validation failed: {error}"));
    let build_id = package_identity::compute_build_id(&manifest_dir, &identity, true)
        .unwrap_or_else(|error| panic!("package build identity failed: {error}"));
    println!("cargo:rustc-env=EMILY_BUILD_ID={build_id}");
    println!(
        "cargo:rustc-env=EMILY_APPLICATION_ID={}",
        identity.application_id
    );
    println!(
        "cargo:rustc-env=EMILY_PRODUCT_NAME={}",
        identity.product_name
    );
    println!(
        "cargo:rustc-env=EMILY_SEMANTIC_VERSION={}",
        identity.semantic_version
    );
    println!(
        "cargo:rustc-env=EMILY_ARCHITECTURE={}",
        identity.architecture
    );
    tauri_build::build();
}
