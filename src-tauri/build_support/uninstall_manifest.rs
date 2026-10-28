use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs::{self, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub const PACKAGE_MANIFEST_SCHEMA_VERSION: u32 = 1;
pub const PACKAGE_MANIFEST_FILE: &str = "uninstall-package-manifest-v1.json";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProductIdentityV1 {
    pub application_id: String,
    pub product_name: String,
    pub build_id: String,
    pub semantic_version: String,
    pub architecture: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PackageManifestEntryV1 {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageManifestV1 {
    pub schema_version: u32,
    pub application_id: String,
    pub product_name: String,
    pub build_id: String,
    pub semantic_version: String,
    pub architecture: String,
    pub files: Vec<PackageManifestEntryV1>,
}

impl PackageManifestV1 {
    pub fn product_identity(&self) -> ProductIdentityV1 {
        ProductIdentityV1 {
            application_id: self.application_id.clone(),
            product_name: self.product_name.clone(),
            build_id: self.build_id.clone(),
            semantic_version: self.semantic_version.clone(),
            architecture: self.architecture.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestVerificationModeV1 {
    RequireAll,
    AllowMissing,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PackageManifestBuildRequestV1 {
    schema_version: u32,
    staging_root: PathBuf,
    entries: Vec<PackageManifestBuildEntryV1>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PackageManifestBuildEntryV1 {
    source: PathBuf,
    destination: String,
}

pub fn canonical_package_manifest_bytes(manifest: &PackageManifestV1) -> Result<Vec<u8>, String> {
    validate_package_manifest(manifest)?;
    serde_json::to_vec(manifest).map_err(|error| error.to_string())
}

pub fn package_manifest_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn parse_package_manifest(
    bytes: &[u8],
    expected_sha256: &str,
) -> Result<PackageManifestV1, String> {
    if !valid_sha256(expected_sha256) || package_manifest_sha256(bytes) != expected_sha256 {
        return Err("package_manifest_hash_mismatch".to_string());
    }
    let manifest = serde_json::from_slice::<PackageManifestV1>(bytes)
        .map_err(|_| "package_manifest_invalid".to_string())?;
    validate_package_manifest(&manifest)?;
    if canonical_package_manifest_bytes(&manifest)? != bytes {
        return Err("package_manifest_not_canonical".to_string());
    }
    Ok(manifest)
}

pub fn validate_package_manifest(manifest: &PackageManifestV1) -> Result<(), String> {
    let identity = manifest.product_identity();
    validate_product_identity(&identity)?;
    if manifest.schema_version != PACKAGE_MANIFEST_SCHEMA_VERSION || manifest.files.is_empty() {
        return Err("package_manifest_invalid".to_string());
    }
    let mut case_folded = BTreeSet::new();
    let mut previous: Option<&str> = None;
    for entry in &manifest.files {
        validate_manifest_entry(entry)?;
        if previous.is_some_and(|value| value >= entry.path.as_str()) {
            return Err("package_manifest_paths_not_sorted".to_string());
        }
        previous = Some(&entry.path);
        if !case_folded.insert(entry.path.to_lowercase()) {
            return Err("package_manifest_path_collision".to_string());
        }
    }
    Ok(())
}

pub fn validate_product_identity(identity: &ProductIdentityV1) -> Result<(), String> {
    if identity.application_id.len() < 3
        || identity.application_id.len() > 128
        || !identity.application_id.contains('.')
        || !identity.application_id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'.' || byte == b'-'
        })
        || identity.product_name.trim() != identity.product_name
        || identity.product_name.is_empty()
        || identity.product_name.len() > 128
        || identity
            .product_name
            .chars()
            .any(|character| character.is_control())
        || !valid_build_id(&identity.build_id)
        || !valid_semantic_version(&identity.semantic_version)
        || identity.architecture != "windows-x86_64"
    {
        return Err("package_identity_invalid".to_string());
    }
    Ok(())
}

pub fn verify_package_manifest_files(
    manifest: &PackageManifestV1,
    install_dir: &Path,
    mode: ManifestVerificationModeV1,
) -> Result<(), String> {
    validate_package_manifest(manifest)?;
    validate_absolute_directory(install_dir)?;
    for entry in &manifest.files {
        let target = install_dir.join(relative_path(&entry.path)?);
        ensure_no_reparse_from_root(install_dir, &target)?;
        let metadata = match fs::symlink_metadata(&target) {
            Ok(metadata) => metadata,
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && mode == ManifestVerificationModeV1::AllowMissing =>
            {
                continue;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err("package_manifest_file_missing".to_string());
            }
            Err(_) => return Err("package_manifest_file_unreadable".to_string()),
        };
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata_is_reparse(&metadata)
            || metadata.len() != entry.bytes
            || sha256_file(&target)? != entry.sha256
        {
            return Err("package_manifest_file_mismatch".to_string());
        }
    }
    Ok(())
}

pub fn build_package_manifest(
    arguments: &[OsString],
    identity: &ProductIdentityV1,
) -> Result<(), String> {
    validate_product_identity(identity)?;
    let (response_file, manifest_output, hash_output, build_id_output) =
        parse_builder_arguments(arguments)?;
    for path in [
        &response_file,
        &manifest_output,
        &hash_output,
        &build_id_output,
    ] {
        if !path.is_absolute() || path.to_str().is_none() {
            return Err("package_manifest_builder_path_invalid".to_string());
        }
    }
    let distinct_paths = [
        &response_file,
        &manifest_output,
        &hash_output,
        &build_id_output,
    ]
    .into_iter()
    .map(|path| normalized_path(path))
    .collect::<BTreeSet<_>>();
    if distinct_paths.len() != 4 {
        return Err("package_manifest_builder_path_invalid".to_string());
    }
    let request: PackageManifestBuildRequestV1 =
        serde_json::from_slice(&fs::read(&response_file).map_err(|error| error.to_string())?)
            .map_err(|_| "package_manifest_build_request_invalid".to_string())?;
    if request.schema_version != 1 || request.entries.is_empty() {
        return Err("package_manifest_build_request_invalid".to_string());
    }
    validate_absolute_directory(&request.staging_root)?;
    let canonical_staging = fs::canonicalize(&request.staging_root)
        .map_err(|_| "package_manifest_staging_invalid".to_string())?;
    if path_is_within(&manifest_output, &request.staging_root)
        || path_is_within(&hash_output, &request.staging_root)
        || path_is_within(&build_id_output, &request.staging_root)
    {
        return Err("package_manifest_builder_output_inside_staging".to_string());
    }
    let mut files = Vec::with_capacity(request.entries.len());
    let mut previous: Option<&str> = None;
    let mut destinations = BTreeSet::new();
    let mut sources = BTreeSet::new();
    for entry in &request.entries {
        let destination = relative_path(&entry.destination)?;
        if previous.is_some_and(|value| value >= entry.destination.as_str()) {
            return Err("package_manifest_build_entries_not_sorted".to_string());
        }
        previous = Some(&entry.destination);
        if !destinations.insert(entry.destination.to_lowercase()) {
            return Err("package_manifest_path_collision".to_string());
        }
        if !entry.source.is_absolute() || entry.source.to_str().is_none() {
            return Err("package_manifest_source_invalid".to_string());
        }
        if !path_is_within(&entry.source, &request.staging_root) {
            return Err("package_manifest_source_outside_staging".to_string());
        }
        ensure_no_reparse_from_root(&request.staging_root, &entry.source)?;
        let metadata = fs::symlink_metadata(&entry.source)
            .map_err(|_| "package_manifest_source_invalid".to_string())?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata_is_reparse(&metadata)
        {
            return Err("package_manifest_source_reparse_point".to_string());
        }
        let canonical_source = fs::canonicalize(&entry.source)
            .map_err(|_| "package_manifest_source_invalid".to_string())?;
        if !path_is_within(&canonical_source, &canonical_staging) {
            return Err("package_manifest_source_outside_staging".to_string());
        }
        if !sources.insert(normalized_path(&canonical_source)) {
            return Err("package_manifest_source_duplicate".to_string());
        }
        let _ = destination;
        files.push(PackageManifestEntryV1 {
            path: entry.destination.clone(),
            bytes: metadata.len(),
            sha256: sha256_file(&entry.source)?,
        });
    }
    let manifest = PackageManifestV1 {
        schema_version: PACKAGE_MANIFEST_SCHEMA_VERSION,
        application_id: identity.application_id.clone(),
        product_name: identity.product_name.clone(),
        build_id: identity.build_id.clone(),
        semantic_version: identity.semantic_version.clone(),
        architecture: identity.architecture.clone(),
        files,
    };
    let bytes = canonical_package_manifest_bytes(&manifest)?;
    let hash = package_manifest_sha256(&bytes);
    write_bytes_atomically(&manifest_output, &bytes)?;
    write_bytes_atomically(&hash_output, hash.as_bytes())?;
    write_bytes_atomically(&build_id_output, identity.build_id.as_bytes())
}

fn parse_builder_arguments(
    arguments: &[OsString],
) -> Result<(PathBuf, PathBuf, PathBuf, PathBuf), String> {
    if arguments.len() != 8
        || arguments[0] != "--response-file"
        || arguments[2] != "--manifest-output"
        || arguments[4] != "--hash-output"
        || arguments[6] != "--build-id-output"
    {
        return Err("package_manifest_builder_arguments_invalid".to_string());
    }
    Ok((
        PathBuf::from(&arguments[1]),
        PathBuf::from(&arguments[3]),
        PathBuf::from(&arguments[5]),
        PathBuf::from(&arguments[7]),
    ))
}

fn validate_manifest_entry(entry: &PackageManifestEntryV1) -> Result<(), String> {
    relative_path(&entry.path)?;
    if !valid_sha256(&entry.sha256) {
        return Err("package_manifest_entry_invalid".to_string());
    }
    Ok(())
}

fn relative_path(value: &str) -> Result<PathBuf, String> {
    if value.is_empty()
        || value.contains('\\')
        || value.starts_with('/')
        || value.ends_with('/')
        || value
            .split('/')
            .any(|part| !valid_windows_path_component(part))
    {
        return Err("package_manifest_path_invalid".to_string());
    }
    let path = PathBuf::from(value);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::Prefix(_) | Component::RootDir | Component::ParentDir
            )
        })
    {
        return Err("package_manifest_path_invalid".to_string());
    }
    Ok(path)
}

fn valid_windows_path_component(value: &str) -> bool {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.ends_with([' ', '.'])
        || value.chars().any(|character| {
            character.is_control() || matches!(character, '<' | '>' | ':' | '"' | '|' | '?' | '*')
        })
    {
        return false;
    }
    let stem = value
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        && !matches!(
            stem.as_str(),
            "COM1" | "COM2" | "COM3" | "COM4" | "COM5" | "COM6" | "COM7" | "COM8" | "COM9"
        )
        && !matches!(
            stem.as_str(),
            "LPT1" | "LPT2" | "LPT3" | "LPT4" | "LPT5" | "LPT6" | "LPT7" | "LPT8" | "LPT9"
        )
}

fn validate_absolute_directory(path: &Path) -> Result<(), String> {
    if !path.is_absolute() || path.to_str().is_none() {
        return Err("package_manifest_root_invalid".to_string());
    }
    let metadata =
        fs::symlink_metadata(path).map_err(|_| "package_manifest_root_invalid".to_string())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() || metadata_is_reparse(&metadata) {
        return Err("package_manifest_root_reparse_point".to_string());
    }
    Ok(())
}

fn ensure_no_reparse_from_root(root: &Path, target: &Path) -> Result<(), String> {
    if !path_is_within(target, root) {
        return Err("package_manifest_path_outside_install".to_string());
    }
    let relative = target
        .strip_prefix(root)
        .map_err(|_| "package_manifest_path_outside_install".to_string())?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() || metadata_is_reparse(&metadata) => {
                return Err("package_manifest_reparse_point".to_string());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(_) => return Err("package_manifest_file_unreadable".to_string()),
        }
    }
    Ok(())
}

fn path_is_within(path: &Path, root: &Path) -> bool {
    let path = normalized_path(path);
    let root = normalized_path(root);
    path == root || path.strip_prefix(&(root + "\\")).is_some()
}

fn normalized_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

fn sha256_file(path: &Path) -> Result<String, String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(fs::read(path).map_err(|error| error.to_string())?)
    ))
}

fn valid_build_id(value: &str) -> bool {
    valid_sha256(value)
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
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

fn write_bytes_atomically(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "package_manifest_output_parent_missing".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "package_manifest_output_invalid".to_string())?;
    let temporary = path.with_file_name(format!(".{file_name}.{}.{nonce}.tmp", std::process::id()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| error.to_string())?;
        file.write_all(bytes).map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        replace_file(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(target_os = "windows")]
fn replace_file(source: &Path, target: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let target = target
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}

#[cfg(not(target_os = "windows"))]
fn replace_file(source: &Path, target: &Path) -> Result<(), String> {
    fs::rename(source, target).map_err(|error| error.to_string())
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
