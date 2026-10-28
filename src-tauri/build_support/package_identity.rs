use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use walkdir::WalkDir;

pub const PACKAGE_ARCHITECTURE: &str = "windows-x86_64";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageIdentityBuildInputV1 {
    pub application_id: String,
    pub product_name: String,
    pub semantic_version: String,
    pub architecture: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BuildIdentityProjection<'a> {
    schema_version: u32,
    application_id: &'a str,
    product_name: &'a str,
    semantic_version: &'a str,
    architecture: &'a str,
    files: Vec<BuildInputProjection>,
}

#[derive(Serialize)]
struct BuildInputProjection {
    path: String,
    sha256: String,
}

pub fn read_package_identity(manifest_dir: &Path) -> Result<PackageIdentityBuildInputV1, String> {
    let config_path = manifest_dir.join("tauri.conf.json");
    let config: serde_json::Value =
        serde_json::from_slice(&fs::read(&config_path).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    let application_id = required_json_string(&config, "/identifier")?;
    let product_name = required_json_string(&config, "/productName")?;
    let semantic_version = required_json_string(&config, "/version")?;
    let cargo_version = read_cargo_package_version(&manifest_dir.join("Cargo.toml"))?;
    if semantic_version != cargo_version {
        return Err("package_version_mismatch".to_string());
    }
    if !valid_identifier(&application_id)
        || product_name.trim() != product_name
        || product_name.is_empty()
        || product_name.len() > 128
        || !valid_semantic_version(&semantic_version)
    {
        return Err("invalid_package_identity".to_string());
    }
    Ok(PackageIdentityBuildInputV1 {
        application_id,
        product_name,
        semantic_version,
        architecture: PACKAGE_ARCHITECTURE.to_string(),
    })
}

pub fn compute_build_id(
    manifest_dir: &Path,
    identity: &PackageIdentityBuildInputV1,
    emit_cargo_directives: bool,
) -> Result<String, String> {
    validate_identity(identity)?;
    let inputs = discover_build_inputs(manifest_dir, emit_cargo_directives)?;
    let files = inputs
        .into_iter()
        .map(|(path, absolute)| {
            Ok(BuildInputProjection {
                path,
                sha256: sha256_file(&absolute)?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let projection = BuildIdentityProjection {
        schema_version: 1,
        application_id: &identity.application_id,
        product_name: &identity.product_name,
        semantic_version: &identity.semantic_version,
        architecture: &identity.architecture,
        files,
    };
    let bytes = serde_json::to_vec(&projection).map_err(|error| error.to_string())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn validate_identity(identity: &PackageIdentityBuildInputV1) -> Result<(), String> {
    if !valid_identifier(&identity.application_id)
        || identity.product_name.trim() != identity.product_name
        || identity.product_name.is_empty()
        || identity.product_name.len() > 128
        || !valid_semantic_version(&identity.semantic_version)
        || identity.architecture != PACKAGE_ARCHITECTURE
    {
        return Err("invalid_package_identity".to_string());
    }
    Ok(())
}

fn discover_build_inputs(
    manifest_dir: &Path,
    emit_cargo_directives: bool,
) -> Result<BTreeMap<String, PathBuf>, String> {
    let mut inputs = BTreeMap::new();
    for relative in ["Cargo.toml", "Cargo.lock", "build.rs", "tauri.conf.json"] {
        insert_input(
            &mut inputs,
            relative.to_string(),
            manifest_dir.join(relative),
            emit_cargo_directives,
        )?;
    }
    for directory in ["build_support", "src", "collector-scripts", "migrations"] {
        let root = manifest_dir.join(directory);
        validate_directory(&root)?;
        for entry in WalkDir::new(&root).sort_by_file_name() {
            let entry = entry.map_err(|error| error.to_string())?;
            if entry.path_is_symlink() {
                return Err("package_build_input_reparse_point".to_string());
            }
            if !entry.file_type().is_file() {
                continue;
            }
            validate_regular_file(entry.path())?;
            let relative = entry
                .path()
                .strip_prefix(manifest_dir)
                .map_err(|_| "package_build_input_outside_root".to_string())?;
            insert_input(
                &mut inputs,
                forward_slash(relative)?,
                entry.path().to_path_buf(),
                emit_cargo_directives,
            )?;
        }
    }
    let repository_root = manifest_dir
        .parent()
        .ok_or_else(|| "package_repository_root_missing".to_string())?;
    insert_input(
        &mut inputs,
        "dist/build-manifest.json".to_string(),
        repository_root.join("dist/build-manifest.json"),
        emit_cargo_directives,
    )?;
    Ok(inputs)
}

fn insert_input(
    inputs: &mut BTreeMap<String, PathBuf>,
    key: String,
    path: PathBuf,
    emit_cargo_directives: bool,
) -> Result<(), String> {
    validate_regular_file(&path)?;
    if inputs.insert(key, path.clone()).is_some() {
        return Err("duplicate_package_build_input".to_string());
    }
    if emit_cargo_directives {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    Ok(())
}

fn validate_directory(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() || metadata_is_reparse(&metadata) {
        return Err("package_build_input_reparse_point".to_string());
    }
    Ok(())
}

fn validate_regular_file(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata_is_reparse(&metadata) {
        return Err("package_build_input_reparse_point".to_string());
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn metadata_is_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(target_os = "windows"))]
fn metadata_is_reparse(_metadata: &fs::Metadata) -> bool {
    false
}

fn sha256_file(path: &Path) -> Result<String, String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(fs::read(path).map_err(|error| error.to_string())?)
    ))
}

fn forward_slash(path: &Path) -> Result<String, String> {
    let value = path
        .to_str()
        .ok_or_else(|| "package_build_input_non_unicode".to_string())?
        .replace('\\', "/");
    if value.is_empty() || value.starts_with('/') || value.contains("../") {
        return Err("package_build_input_outside_root".to_string());
    }
    Ok(value)
}

fn required_json_string(value: &serde_json::Value, pointer: &str) -> Result<String, String> {
    value
        .pointer(pointer)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "invalid_package_identity".to_string())
}

fn read_cargo_package_version(path: &Path) -> Result<String, String> {
    let raw = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let mut in_package = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed == "[package]";
            continue;
        }
        if in_package {
            if let Some(value) = trimmed.strip_prefix("version") {
                let value = value
                    .trim_start()
                    .strip_prefix('=')
                    .map(str::trim)
                    .and_then(|value| value.strip_prefix('"'))
                    .and_then(|value| value.strip_suffix('"'))
                    .ok_or_else(|| "invalid_cargo_package_version".to_string())?;
                return Ok(value.to_string());
            }
        }
    }
    Err("cargo_package_version_missing".to_string())
}

fn valid_identifier(value: &str) -> bool {
    value.len() >= 3
        && value.len() <= 128
        && value.contains('.')
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'.' || byte == b'-'
        })
}

fn valid_semantic_version(value: &str) -> bool {
    let core = value.split(['-', '+']).next().unwrap_or_default();
    let parts = core.split('.').collect::<Vec<_>>();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && (part == &"0" || !part.starts_with('0'))
        })
}
