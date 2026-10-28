use rusqlite::{Connection, ErrorCode, OpenFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    error::Error,
    ffi::{OsStr, OsString},
    fmt, fs,
    fs::{File, OpenOptions},
    io::{ErrorKind, Read, Seek, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;

use crate::{
    bootstrap::{
        read_bootstrap, validate_data_root_marker, write_bootstrap_atomic,
        write_data_root_marker_atomic, write_json_atomically, BootstrapV1, DataRootMarkerV1,
        DataRootRefV1, RetainedDataRootV1, RetainedLayoutV1, BOOTSTRAP_FILE, DATA_ROOT_MARKER_FILE,
    },
    collector::browser_state::{
        browser_profile_path, canonical_windows_path, read_browser_selection,
        read_login_validation, profile_generation_path, rewrite_browser_state_for_migration,
        BrowserId,
    },
};

const TARGET_OWNERSHIP_FILE: &str = ".emily-migration-target.json";
pub const MIGRATION_JOURNAL_FILE: &str = "data-root-migration.json";
const LEGACY_ADOPTION_JOURNAL_FILE: &str = "legacy-data-root-adoption.json";
const LEGACY_MANAGED_ENTRIES: &[&str] = &[
    "history.sqlite",
    "history.sqlite-wal",
    "history.sqlite-shm",
    "profiles",
    "tmp",
    "diagnostics",
    "logs",
    "browser-selection.json",
    "device-settings.json",
];
const PROBE_FILE_CONTENTS: &[u8] = b"emily-data-root-probe-v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataRootContext {
    pub app_data_dir: PathBuf,
    pub install_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryIdentity {
    pub canonical_path: PathBuf,
    pub volume_serial_number: u32,
    pub file_index: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedDataRoot {
    pub path: PathBuf,
    pub parent_identity: DirectoryIdentity,
    pub target_existed: bool,
    pub ownership_token: String,
    pub adjusted_to_install_data: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DataRootValidationView {
    pub requested_path: PathBuf,
    pub normalized_path: PathBuf,
    pub adjusted_to_install_data: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DataRootMigrationView {
    pub data_root: PathBuf,
    pub retained_source: DataRootRefV1,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingDataRootView {
    pub data_root: PathBuf,
}

pub fn parse_uninstall_managed_data_arguments(
    arguments: &[OsString],
) -> Result<Option<PathBuf>, DataRootError> {
    if arguments.first().map(OsString::as_os_str) != Some(OsStr::new("--uninstall-managed-data")) {
        return Ok(None);
    }
    if arguments.len() != 3
        || arguments.get(1).map(OsString::as_os_str) != Some(OsStr::new("--bootstrap-dir"))
    {
        return Err(DataRootError::new("uninstall_arguments_invalid"));
    }
    let bootstrap_dir = PathBuf::from(&arguments[2]);
    validate_input_path(&bootstrap_dir)?;
    if is_drive_root(&bootstrap_dir) {
        return Err(DataRootError::new("data_root_is_drive_root"));
    }
    Ok(Some(bootstrap_dir))
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataInventory {
    pub file_count: u64,
    pub total_bytes: u64,
    pub critical_hashes: BTreeMap<String, String>,
    pub semantic_browser_hashes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MigrationPhaseV1 {
    Prepared,
    StagedVerified,
    Switching,
    TargetRenamed,
    BootstrapCommitted,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MigrationJournalV1 {
    pub schema_version: i64,
    pub migration_id: String,
    pub source: DataRootRefV1,
    pub target: DataRootRefV1,
    pub staging: PathBuf,
    pub target_token: String,
    pub phase: MigrationPhaseV1,
    pub source_inventory: DataInventory,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum LegacyAdoptionPhaseV1 {
    Prepared,
    StagedVerified,
    Switching,
    TargetRenamed,
    BootstrapCommitted,
    EntriesMoving,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LegacyAdoptionJournalV1 {
    schema_version: i64,
    adoption_id: String,
    target: DataRootRefV1,
    staging: PathBuf,
    retained: DataRootRefV1,
    target_token: String,
    phase: LegacyAdoptionPhaseV1,
    source_inventory: DataInventory,
    legacy_entries: Vec<String>,
    active_account_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LegacyDeviceSettingsV1 {
    schema_version: i64,
    active_account_id: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationFaultPoint {
    BeforeJournalWrite,
    DuringCopy,
    DuringBrowserStateRewrite,
    CheckpointBusy,
    BeforeVerification,
    AfterTargetShellRemoved,
    AfterStagingRename,
    BeforeBootstrapWrite,
    AfterBootstrapWrite,
}

#[derive(Debug, Clone, Default)]
pub struct MigrationHooks {
    interrupt_at: Option<MigrationFaultPoint>,
}

impl MigrationHooks {
    #[doc(hidden)]
    pub fn interrupt_at(point: MigrationFaultPoint) -> Self {
        Self {
            interrupt_at: Some(point),
        }
    }

    fn check(&self, point: MigrationFaultPoint) -> Result<(), DataRootError> {
        if self.interrupt_at != Some(point) {
            return Ok(());
        }
        let code = if point == MigrationFaultPoint::CheckpointBusy {
            "data_root_checkpoint_busy"
        } else {
            "data_root_migration_interrupted"
        };
        Err(DataRootError::new(code))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationResult {
    pub bootstrap: BootstrapV1,
}

impl NormalizedDataRoot {
    pub fn verify_target_ownership(&self) -> Result<(), DataRootError> {
        ensure_no_reparse_ancestors(&self.path)?;
        let parent = self
            .path
            .parent()
            .ok_or_else(|| DataRootError::new("data_root_is_drive_root"))?;
        if directory_identity(parent)? != self.parent_identity {
            return Err(DataRootError::new("data_root_permission_denied"));
        }
        ensure_target_contains_only_ownership(&self.path, &self.ownership_token)
    }

    pub fn release_target_ownership(&self) -> Result<(), DataRootError> {
        self.verify_target_ownership()?;
        fs::remove_file(self.path.join(TARGET_OWNERSHIP_FILE))
            .map_err(|_| DataRootError::new("data_root_permission_denied"))?;
        if !self.target_existed {
            fs::remove_dir(&self.path)
                .map_err(|_| DataRootError::new("data_root_permission_denied"))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataRootError {
    code: &'static str,
}

impl DataRootError {
    fn new(code: &'static str) -> Self {
        Self { code }
    }

    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl fmt::Display for DataRootError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code)
    }
}

impl Error for DataRootError {}

pub fn read_migration_journal(
    bootstrap_dir: &Path,
) -> Result<Option<MigrationJournalV1>, DataRootError> {
    let path = bootstrap_dir.join(MIGRATION_JOURNAL_FILE);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(DataRootError::new("data_root_migration_journal_invalid")),
    };
    let journal = serde_json::from_slice::<MigrationJournalV1>(&bytes)
        .map_err(|_| DataRootError::new("data_root_migration_journal_invalid"))?;
    validate_migration_journal(&journal)?;
    Ok(Some(journal))
}

pub fn write_migration_journal_atomic(
    bootstrap_dir: &Path,
    journal: &MigrationJournalV1,
) -> Result<(), DataRootError> {
    validate_migration_journal(journal)?;
    write_json_atomically(&bootstrap_dir.join(MIGRATION_JOURNAL_FILE), journal)
        .map_err(|_| DataRootError::new("data_root_migration_journal_write_failed"))
}

fn validate_migration_journal(journal: &MigrationJournalV1) -> Result<(), DataRootError> {
    if journal.schema_version != 1
        || !valid_hex_identity(&journal.migration_id)
        || !valid_hex_identity(&journal.source.root_id)
        || !valid_hex_identity(&journal.target.root_id)
        || !valid_hex_identity(&journal.target_token)
        || journal.source.root_id == journal.target.root_id
        || !journal.source.path.is_absolute()
        || !journal.target.path.is_absolute()
        || !journal.staging.is_absolute()
        || journal.source.path.to_str().is_none()
        || journal.target.path.to_str().is_none()
        || journal.staging.to_str().is_none()
        || paths_equal(&journal.source.path, &journal.target.path)
    {
        return Err(DataRootError::new("data_root_migration_journal_invalid"));
    }
    let expected_staging_name = format!(".emily-migration-{}.staging", journal.migration_id);
    if journal.staging.parent() != journal.target.path.parent()
        || journal.staging.file_name().and_then(|value| value.to_str())
            != Some(expected_staging_name.as_str())
    {
        return Err(DataRootError::new("data_root_migration_journal_invalid"));
    }
    Ok(())
}

fn valid_hex_identity(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub fn resolve_or_initialize_data_root(bootstrap_dir: &Path) -> Result<BootstrapV1, DataRootError> {
    validate_input_path(bootstrap_dir)?;
    fs::create_dir_all(bootstrap_dir)
        .map_err(|_| DataRootError::new("data_root_permission_denied"))?;
    ensure_no_reparse_ancestors(bootstrap_dir)?;
    let metadata = fs::symlink_metadata(bootstrap_dir)
        .map_err(|_| DataRootError::new("data_root_permission_denied"))?;
    if !metadata.is_dir() || metadata_is_reparse(&metadata) {
        return Err(DataRootError::new("data_root_reparse_point"));
    }

    recover_legacy_adoption_if_present(bootstrap_dir)?;
    if let Some(bootstrap) = read_bootstrap(bootstrap_dir)
        .map_err(|_| DataRootError::new("data_root_bootstrap_invalid"))?
    {
        let recovered = recover_interrupted_data_root_migration(bootstrap_dir, &bootstrap)?;
        validate_data_root_marker(&recovered.current_root.path, &recovered.current_root)
            .map_err(map_marker_error)?;
        return Ok(recovered);
    }
    if read_migration_journal(bootstrap_dir)?.is_some() {
        return Err(DataRootError::new("data_root_migration_journal_invalid"));
    }

    let legacy_entries = existing_legacy_entries(bootstrap_dir)?;
    if legacy_entries.is_empty() {
        initialize_default_data_root(bootstrap_dir)
    } else {
        adopt_legacy_flat_data_root(bootstrap_dir, legacy_entries)
    }
}

fn initialize_default_data_root(bootstrap_dir: &Path) -> Result<BootstrapV1, DataRootError> {
    let target = bootstrap_dir.join("Data");
    if target.exists() {
        return Err(DataRootError::new("data_root_untracked_default_exists"));
    }
    fs::create_dir(&target).map_err(|_| DataRootError::new("data_root_permission_denied"))?;
    let root_id = Uuid::new_v4().simple().to_string();
    let marker = DataRootMarkerV1::new(root_id.clone())
        .map_err(|_| DataRootError::new("data_root_marker_missing_or_invalid"))?;
    if write_data_root_marker_atomic(&target, &marker).is_err() {
        let _ = fs::remove_dir(&target);
        return Err(DataRootError::new("data_root_marker_missing_or_invalid"));
    }
    let bootstrap = BootstrapV1::new(target.clone(), root_id)
        .map_err(|_| DataRootError::new("data_root_bootstrap_invalid"))?;
    if write_bootstrap_atomic(bootstrap_dir, &bootstrap).is_err() {
        let _ = fs::remove_file(target.join(DATA_ROOT_MARKER_FILE));
        let _ = fs::remove_dir(&target);
        return Err(DataRootError::new("data_root_bootstrap_write_failed"));
    }
    Ok(bootstrap)
}

fn existing_legacy_entries(bootstrap_dir: &Path) -> Result<Vec<String>, DataRootError> {
    let mut entries = Vec::new();
    for name in LEGACY_MANAGED_ENTRIES {
        let path = bootstrap_dir.join(name);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(_) => return Err(DataRootError::new("data_root_permission_denied")),
        };
        if metadata_is_reparse(&metadata) {
            return Err(DataRootError::new("data_root_reparse_point"));
        }
        if metadata.is_dir() {
            ensure_tree_has_no_reparse_points(&path)?;
        } else if !metadata.is_file() {
            return Err(DataRootError::new("data_root_verification_failed"));
        }
        entries.push((*name).to_string());
    }
    Ok(entries)
}

fn adopt_legacy_flat_data_root(
    bootstrap_dir: &Path,
    mut legacy_entries: Vec<String>,
) -> Result<BootstrapV1, DataRootError> {
    if legacy_entries.iter().any(|entry| entry == "history.sqlite") {
        checkpoint_source_database(bootstrap_dir)?;
        legacy_entries = existing_legacy_entries(bootstrap_dir)?;
    }
    let source_inventory = build_legacy_data_inventory(bootstrap_dir, &legacy_entries)?;
    let active_account_id = read_legacy_active_account(bootstrap_dir, &legacy_entries)?;
    let target_path = bootstrap_dir.join("Data");
    if target_path.exists() {
        return Err(DataRootError::new("data_root_untracked_default_exists"));
    }
    fs::create_dir(&target_path).map_err(|_| DataRootError::new("data_root_permission_denied"))?;
    let target_token = Uuid::new_v4().simple().to_string();
    write_target_ownership(&target_path, &target_token)?;
    let adoption_id = Uuid::new_v4().simple().to_string();
    let target = DataRootRefV1 {
        path: target_path,
        root_id: Uuid::new_v4().simple().to_string(),
    };
    let retained_root_id = Uuid::new_v4().simple().to_string();
    let retained = DataRootRefV1 {
        path: bootstrap_dir.join(format!(".emily-legacy-retained-{retained_root_id}")),
        root_id: retained_root_id,
    };
    let staging = bootstrap_dir.join(format!(".emily-legacy-{adoption_id}.staging"));
    let mut journal = LegacyAdoptionJournalV1 {
        schema_version: 1,
        adoption_id,
        target,
        staging,
        retained,
        target_token,
        phase: LegacyAdoptionPhaseV1::Prepared,
        source_inventory,
        legacy_entries,
        active_account_id,
    };
    write_legacy_adoption_journal(bootstrap_dir, &journal)?;
    create_legacy_staging(&journal)?;
    copy_legacy_entries_to_staging(bootstrap_dir, &journal)?;
    rewrite_browser_states_in_staging(bootstrap_dir, &journal.staging)?;
    verify_data_inventory(&journal.staging, &journal.source_inventory)?;
    journal.phase = LegacyAdoptionPhaseV1::StagedVerified;
    write_legacy_adoption_journal(bootstrap_dir, &journal)?;
    journal.phase = LegacyAdoptionPhaseV1::Switching;
    write_legacy_adoption_journal(bootstrap_dir, &journal)?;
    remove_target_shell(&journal.target.path, &journal.target_token)?;
    rename_staging_atomically(&journal.staging, &journal.target.path)?;
    journal.phase = LegacyAdoptionPhaseV1::TargetRenamed;
    write_legacy_adoption_journal(bootstrap_dir, &journal)?;
    commit_legacy_adoption(bootstrap_dir, &mut journal)
}

fn commit_legacy_adoption(
    bootstrap_dir: &Path,
    journal: &mut LegacyAdoptionJournalV1,
) -> Result<BootstrapV1, DataRootError> {
    validate_legacy_target(journal)?;
    let mut bootstrap =
        BootstrapV1::new(journal.target.path.clone(), journal.target.root_id.clone())
            .map_err(|_| DataRootError::new("data_root_bootstrap_invalid"))?;
    bootstrap.active_account_id = journal.active_account_id;
    bootstrap.retained_data_roots.push(RetainedDataRootV1 {
        root: journal.retained.clone(),
        layout: RetainedLayoutV1::LegacyFlat,
        legacy_entries: journal.legacy_entries.clone(),
    });
    write_bootstrap_atomic(bootstrap_dir, &bootstrap)
        .map_err(|_| DataRootError::new("data_root_bootstrap_write_failed"))?;
    journal.phase = LegacyAdoptionPhaseV1::BootstrapCommitted;
    write_legacy_adoption_journal(bootstrap_dir, journal)?;
    move_legacy_entries_to_retained(bootstrap_dir, journal)?;
    remove_legacy_adoption_journal(bootstrap_dir)?;
    Ok(bootstrap)
}

fn recover_legacy_adoption_if_present(bootstrap_dir: &Path) -> Result<(), DataRootError> {
    let Some(mut journal) = read_legacy_adoption_journal(bootstrap_dir)? else {
        return Ok(());
    };
    let bootstrap = read_bootstrap(bootstrap_dir)
        .map_err(|_| DataRootError::new("data_root_bootstrap_invalid"))?;
    if let Some(existing) = bootstrap.as_ref() {
        if !root_refs_match(&existing.current_root, &journal.target) {
            return Err(DataRootError::new("data_root_migration_journal_invalid"));
        }
        validate_legacy_target(&journal)?;
        move_legacy_entries_to_retained(bootstrap_dir, &mut journal)?;
        remove_legacy_adoption_journal(bootstrap_dir)?;
        return Ok(());
    }

    match journal.phase {
        LegacyAdoptionPhaseV1::Prepared => {
            remove_legacy_staging_if_present(&journal)?;
            remove_legacy_target_shell_if_present(&journal)?;
            remove_legacy_adoption_journal(bootstrap_dir)
        }
        LegacyAdoptionPhaseV1::StagedVerified | LegacyAdoptionPhaseV1::Switching => {
            finish_legacy_target_switch(&journal)?;
            journal.phase = LegacyAdoptionPhaseV1::TargetRenamed;
            write_legacy_adoption_journal(bootstrap_dir, &journal)?;
            commit_legacy_adoption(bootstrap_dir, &mut journal).map(|_| ())
        }
        LegacyAdoptionPhaseV1::TargetRenamed => {
            commit_legacy_adoption(bootstrap_dir, &mut journal).map(|_| ())
        }
        LegacyAdoptionPhaseV1::BootstrapCommitted | LegacyAdoptionPhaseV1::EntriesMoving => {
            Err(DataRootError::new("data_root_migration_journal_invalid"))
        }
    }
}

fn finish_legacy_target_switch(journal: &LegacyAdoptionJournalV1) -> Result<(), DataRootError> {
    if journal.target.path.exists() {
        if validate_legacy_target(journal).is_ok() && !journal.staging.exists() {
            return Ok(());
        }
        remove_target_shell(&journal.target.path, &journal.target_token)?;
    }
    validate_legacy_staging(journal)?;
    rename_staging_atomically(&journal.staging, &journal.target.path)?;
    validate_legacy_target(journal)
}

fn create_legacy_staging(journal: &LegacyAdoptionJournalV1) -> Result<(), DataRootError> {
    if journal.staging.exists() || journal.retained.path.exists() {
        return Err(DataRootError::new("data_root_copy_failed"));
    }
    fs::create_dir(&journal.staging).map_err(|_| DataRootError::new("data_root_copy_failed"))?;
    let marker = DataRootMarkerV1::new(journal.target.root_id.clone())
        .map_err(|_| DataRootError::new("data_root_copy_failed"))?;
    write_data_root_marker_atomic(&journal.staging, &marker)
        .map_err(|_| DataRootError::new("data_root_copy_failed"))
}

fn copy_legacy_entries_to_staging(
    bootstrap_dir: &Path,
    journal: &LegacyAdoptionJournalV1,
) -> Result<(), DataRootError> {
    for name in &journal.legacy_entries {
        if name == "browser-selection.json" || name == "history.sqlite-shm" {
            continue;
        }
        let source = bootstrap_dir.join(name);
        if name == "history.sqlite-wal" {
            let metadata = fs::symlink_metadata(&source)
                .map_err(|_| DataRootError::new("data_root_copy_failed"))?;
            if metadata.len() == 0 {
                continue;
            }
        }
        copy_entry_tree(&source, &journal.staging.join(name))?;
    }
    Ok(())
}

fn copy_entry_tree(source: &Path, target: &Path) -> Result<(), DataRootError> {
    let metadata =
        fs::symlink_metadata(source).map_err(|_| DataRootError::new("data_root_copy_failed"))?;
    if metadata_is_reparse(&metadata) {
        return Err(DataRootError::new("data_root_reparse_point"));
    }
    if metadata.is_file() {
        return copy_file_no_follow(source, target);
    }
    if !metadata.is_dir() {
        return Err(DataRootError::new("data_root_copy_failed"));
    }
    fs::create_dir(target).map_err(|_| DataRootError::new("data_root_copy_failed"))?;
    let mut entries = fs::read_dir(source)
        .map_err(|_| DataRootError::new("data_root_copy_failed"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| DataRootError::new("data_root_copy_failed"))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        copy_entry_tree(&entry.path(), &target.join(entry.file_name()))?;
    }
    Ok(())
}

fn build_legacy_data_inventory(
    bootstrap_dir: &Path,
    legacy_entries: &[String],
) -> Result<DataInventory, DataRootError> {
    let (semantic_browser_hashes, browser_state_files) =
        if legacy_entries.iter().any(|entry| entry == "profiles") {
            browser_state_inventory(bootstrap_dir)?
        } else {
            (BTreeMap::new(), HashSet::new())
        };
    let mut inventory = DataInventory {
        file_count: 0,
        total_bytes: 0,
        critical_hashes: BTreeMap::new(),
        semantic_browser_hashes,
    };
    for name in legacy_entries {
        if name == "browser-selection.json" || name == "history.sqlite-shm" {
            continue;
        }
        let root = bootstrap_dir.join(name);
        let mut pending = vec![root];
        while let Some(path) = pending.pop() {
            let metadata = fs::symlink_metadata(&path)
                .map_err(|_| DataRootError::new("data_root_verification_failed"))?;
            if metadata_is_reparse(&metadata) {
                return Err(DataRootError::new("data_root_reparse_point"));
            }
            if metadata.is_dir() {
                for entry in fs::read_dir(&path)
                    .map_err(|_| DataRootError::new("data_root_verification_failed"))?
                {
                    pending.push(
                        entry
                            .map_err(|_| DataRootError::new("data_root_verification_failed"))?
                            .path(),
                    );
                }
                continue;
            }
            if !metadata.is_file() || browser_state_files.contains(&path) {
                continue;
            }
            let relative = relative_path_string(bootstrap_dir, &path)?;
            if relative == "history.sqlite-wal" && metadata.len() == 0 {
                continue;
            }
            inventory.file_count = inventory
                .file_count
                .checked_add(1)
                .ok_or_else(|| DataRootError::new("data_root_verification_failed"))?;
            inventory.total_bytes = inventory
                .total_bytes
                .checked_add(metadata.len())
                .ok_or_else(|| DataRootError::new("data_root_verification_failed"))?;
            if relative == "history.sqlite" {
                inventory
                    .critical_hashes
                    .insert(relative, hash_file(&path)?);
            }
        }
    }
    Ok(inventory)
}

fn read_legacy_active_account(
    bootstrap_dir: &Path,
    legacy_entries: &[String],
) -> Result<Option<i64>, DataRootError> {
    if !legacy_entries
        .iter()
        .any(|entry| entry == "device-settings.json")
    {
        return Ok(None);
    }
    let bytes = fs::read(bootstrap_dir.join("device-settings.json"))
        .map_err(|_| DataRootError::new("data_root_verification_failed"))?;
    let settings = serde_json::from_slice::<LegacyDeviceSettingsV1>(&bytes)
        .map_err(|_| DataRootError::new("data_root_verification_failed"))?;
    if settings.schema_version != 1 || settings.active_account_id.is_some_and(|id| id <= 0) {
        return Err(DataRootError::new("data_root_verification_failed"));
    }
    Ok(settings.active_account_id)
}

fn move_legacy_entries_to_retained(
    bootstrap_dir: &Path,
    journal: &mut LegacyAdoptionJournalV1,
) -> Result<(), DataRootError> {
    journal.phase = LegacyAdoptionPhaseV1::EntriesMoving;
    write_legacy_adoption_journal(bootstrap_dir, journal)?;
    ensure_no_reparse_ancestors(
        journal
            .retained
            .path
            .parent()
            .ok_or_else(|| DataRootError::new("data_root_switch_failed"))?,
    )?;
    if !journal.retained.path.exists() {
        fs::create_dir(&journal.retained.path)
            .map_err(|_| DataRootError::new("data_root_switch_failed"))?;
        let marker = DataRootMarkerV1::new(journal.retained.root_id.clone())
            .map_err(|_| DataRootError::new("data_root_switch_failed"))?;
        write_data_root_marker_atomic(&journal.retained.path, &marker)
            .map_err(|_| DataRootError::new("data_root_switch_failed"))?;
    } else {
        let metadata = fs::symlink_metadata(&journal.retained.path)
            .map_err(|_| DataRootError::new("data_root_switch_failed"))?;
        if !metadata.is_dir() || metadata_is_reparse(&metadata) {
            return Err(DataRootError::new("data_root_reparse_point"));
        }
        validate_data_root_marker(&journal.retained.path, &journal.retained)
            .map_err(map_marker_error)?;
    }
    for name in &journal.legacy_entries {
        let source = bootstrap_dir.join(name);
        let target = journal.retained.path.join(name);
        match (
            path_exists_no_follow(&source)?,
            path_exists_no_follow(&target)?,
        ) {
            (true, false) => {
                let metadata = fs::symlink_metadata(&source)
                    .map_err(|_| DataRootError::new("data_root_switch_failed"))?;
                if metadata_is_reparse(&metadata) {
                    return Err(DataRootError::new("data_root_reparse_point"));
                }
                if metadata.is_dir() {
                    ensure_tree_has_no_reparse_points(&source)?;
                }
                fs::rename(&source, &target)
                    .map_err(|_| DataRootError::new("data_root_switch_failed"))?;
            }
            (false, true) => {}
            _ => return Err(DataRootError::new("data_root_retained_root_changed")),
        }
    }
    Ok(())
}

fn validate_legacy_staging(journal: &LegacyAdoptionJournalV1) -> Result<(), DataRootError> {
    validate_data_root_marker(
        &journal.staging,
        &DataRootRefV1 {
            path: journal.staging.clone(),
            root_id: journal.target.root_id.clone(),
        },
    )
    .map_err(|_| DataRootError::new("data_root_verification_failed"))?;
    verify_data_inventory(&journal.staging, &journal.source_inventory)
}

fn validate_legacy_target(journal: &LegacyAdoptionJournalV1) -> Result<(), DataRootError> {
    validate_data_root_marker(&journal.target.path, &journal.target).map_err(map_marker_error)?;
    verify_data_inventory(&journal.target.path, &journal.source_inventory)
}

fn remove_legacy_staging_if_present(
    journal: &LegacyAdoptionJournalV1,
) -> Result<(), DataRootError> {
    if !journal.staging.exists() {
        return Ok(());
    }
    validate_data_root_marker(
        &journal.staging,
        &DataRootRefV1 {
            path: journal.staging.clone(),
            root_id: journal.target.root_id.clone(),
        },
    )
    .map_err(|_| DataRootError::new("data_root_migration_journal_invalid"))?;
    remove_tree_no_follow(&journal.staging)
}

fn remove_legacy_target_shell_if_present(
    journal: &LegacyAdoptionJournalV1,
) -> Result<(), DataRootError> {
    if journal.target.path.exists() {
        remove_target_shell(&journal.target.path, &journal.target_token)?;
    }
    Ok(())
}

fn read_legacy_adoption_journal(
    bootstrap_dir: &Path,
) -> Result<Option<LegacyAdoptionJournalV1>, DataRootError> {
    let path = bootstrap_dir.join(LEGACY_ADOPTION_JOURNAL_FILE);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(DataRootError::new("data_root_migration_journal_invalid")),
    };
    let journal = serde_json::from_slice::<LegacyAdoptionJournalV1>(&bytes)
        .map_err(|_| DataRootError::new("data_root_migration_journal_invalid"))?;
    validate_legacy_adoption_journal(bootstrap_dir, &journal)?;
    Ok(Some(journal))
}

fn write_legacy_adoption_journal(
    bootstrap_dir: &Path,
    journal: &LegacyAdoptionJournalV1,
) -> Result<(), DataRootError> {
    validate_legacy_adoption_journal(bootstrap_dir, journal)?;
    write_json_atomically(&bootstrap_dir.join(LEGACY_ADOPTION_JOURNAL_FILE), journal)
        .map_err(|_| DataRootError::new("data_root_migration_journal_write_failed"))
}

fn validate_legacy_adoption_journal(
    bootstrap_dir: &Path,
    journal: &LegacyAdoptionJournalV1,
) -> Result<(), DataRootError> {
    let allowed = LEGACY_MANAGED_ENTRIES
        .iter()
        .copied()
        .collect::<HashSet<_>>();
    let unique_entries = journal
        .legacy_entries
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let expected_staging = format!(".emily-legacy-{}.staging", journal.adoption_id);
    let expected_retained = format!(".emily-legacy-retained-{}", journal.retained.root_id);
    if journal.schema_version != 1
        || !valid_hex_identity(&journal.adoption_id)
        || !valid_hex_identity(&journal.target.root_id)
        || !valid_hex_identity(&journal.retained.root_id)
        || !valid_hex_identity(&journal.target_token)
        || journal.legacy_entries.is_empty()
        || unique_entries.len() != journal.legacy_entries.len()
        || journal
            .legacy_entries
            .iter()
            .any(|entry| !allowed.contains(entry.as_str()))
        || journal.active_account_id.is_some_and(|id| id <= 0)
        || journal.target.path != bootstrap_dir.join("Data")
        || journal.staging.parent() != Some(bootstrap_dir)
        || journal.staging.file_name().and_then(|value| value.to_str())
            != Some(expected_staging.as_str())
        || journal.retained.path.parent() != Some(bootstrap_dir)
        || journal
            .retained
            .path
            .file_name()
            .and_then(|value| value.to_str())
            != Some(expected_retained.as_str())
    {
        return Err(DataRootError::new("data_root_migration_journal_invalid"));
    }
    Ok(())
}

fn remove_legacy_adoption_journal(bootstrap_dir: &Path) -> Result<(), DataRootError> {
    match fs::remove_file(bootstrap_dir.join(LEGACY_ADOPTION_JOURNAL_FILE)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(_) => Err(DataRootError::new(
            "data_root_migration_journal_write_failed",
        )),
    }
}

fn path_exists_no_follow(path: &Path) -> Result<bool, DataRootError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(_) => Err(DataRootError::new("data_root_permission_denied")),
    }
}

pub fn migrate_data_root(
    source: &DataRootRefV1,
    target: &NormalizedDataRoot,
    bootstrap_dir: &Path,
    hooks: &MigrationHooks,
) -> Result<MigrationResult, DataRootError> {
    if read_migration_journal(bootstrap_dir)?.is_some() {
        return finish_before_journal(
            target,
            DataRootError::new("data_root_migration_in_progress"),
        );
    }
    target.verify_target_ownership()?;
    validate_data_root_marker(&source.path, source)
        .map_err(|_| DataRootError::new("data_root_source_mismatch"))?;
    let bootstrap = read_bootstrap(bootstrap_dir)
        .map_err(|_| DataRootError::new("data_root_bootstrap_invalid"))?
        .ok_or_else(|| DataRootError::new("data_root_bootstrap_missing"))?;
    if !root_refs_match(&bootstrap.current_root, source) {
        return finish_before_journal(target, DataRootError::new("data_root_source_mismatch"));
    }

    if let Err(error) = hooks.check(MigrationFaultPoint::CheckpointBusy) {
        return finish_before_journal(target, error);
    }
    if let Err(error) = checkpoint_source_database(&source.path) {
        return finish_before_journal(target, error);
    }
    let source_inventory = match build_source_data_inventory(&source.path) {
        Ok(inventory) => inventory,
        Err(error) => return finish_before_journal(target, error),
    };
    if let Err(error) = hooks.check(MigrationFaultPoint::BeforeJournalWrite) {
        return finish_before_journal(target, error);
    }

    let migration_id = Uuid::new_v4().simple().to_string();
    let target_ref = DataRootRefV1 {
        path: target.path.clone(),
        root_id: Uuid::new_v4().simple().to_string(),
    };
    let staging = target
        .path
        .parent()
        .ok_or_else(|| DataRootError::new("data_root_switch_failed"))?
        .join(format!(".emily-migration-{migration_id}.staging"));
    let mut journal = MigrationJournalV1 {
        schema_version: 1,
        migration_id,
        source: source.clone(),
        target: target_ref,
        staging,
        target_token: target.ownership_token.clone(),
        phase: MigrationPhaseV1::Prepared,
        source_inventory,
    };
    if let Err(error) = write_migration_journal_atomic(bootstrap_dir, &journal) {
        return finish_before_journal(target, error);
    }

    create_owned_staging(&journal)?;
    hooks.check(MigrationFaultPoint::DuringCopy)?;
    copy_data_root_tree(&journal.source.path, &journal.staging)?;
    hooks.check(MigrationFaultPoint::DuringBrowserStateRewrite)?;
    rewrite_browser_states_in_staging(&journal.source.path, &journal.staging)?;
    hooks.check(MigrationFaultPoint::BeforeVerification)?;
    verify_data_inventory(&journal.staging, &journal.source_inventory)?;

    journal.phase = MigrationPhaseV1::StagedVerified;
    write_migration_journal_atomic(bootstrap_dir, &journal)?;
    target.verify_target_ownership()?;
    verify_staging(&journal)?;
    journal.phase = MigrationPhaseV1::Switching;
    write_migration_journal_atomic(bootstrap_dir, &journal)?;
    remove_target_shell(&journal.target.path, &journal.target_token)?;
    hooks.check(MigrationFaultPoint::AfterTargetShellRemoved)?;

    rename_staging_atomically(&journal.staging, &journal.target.path)?;
    journal.phase = MigrationPhaseV1::TargetRenamed;
    write_migration_journal_atomic(bootstrap_dir, &journal)?;
    hooks.check(MigrationFaultPoint::AfterStagingRename)?;
    validate_completed_target(&journal)?;
    hooks.check(MigrationFaultPoint::BeforeBootstrapWrite)?;

    let committed = committed_bootstrap(&bootstrap, &journal)?;
    write_bootstrap_atomic(bootstrap_dir, &committed)
        .map_err(|_| DataRootError::new("data_root_bootstrap_write_failed"))?;
    hooks.check(MigrationFaultPoint::AfterBootstrapWrite)?;

    journal.phase = MigrationPhaseV1::BootstrapCommitted;
    let _ = write_migration_journal_atomic(bootstrap_dir, &journal);
    let _ = remove_migration_journal(bootstrap_dir);
    Ok(MigrationResult {
        bootstrap: committed,
    })
}

pub fn recover_interrupted_data_root_migration(
    bootstrap_dir: &Path,
    bootstrap: &BootstrapV1,
) -> Result<BootstrapV1, DataRootError> {
    let Some(mut journal) = read_migration_journal(bootstrap_dir)? else {
        return Ok(bootstrap.clone());
    };
    let bootstrap_is_source = root_refs_match(&bootstrap.current_root, &journal.source);
    let bootstrap_is_target = root_refs_match(&bootstrap.current_root, &journal.target);
    if !bootstrap_is_source && !bootstrap_is_target {
        return Err(DataRootError::new("data_root_migration_journal_invalid"));
    }

    if bootstrap_is_target {
        validate_completed_target(&journal)?;
        remove_owned_staging_if_present(&journal)?;
        remove_migration_journal(bootstrap_dir)?;
        return Ok(bootstrap.clone());
    }

    validate_data_root_marker(&journal.source.path, &journal.source)
        .map_err(|_| DataRootError::new("data_root_source_mismatch"))?;
    match journal.phase {
        MigrationPhaseV1::Prepared => {
            remove_owned_staging_if_present(&journal)?;
            remove_owned_target_shell_if_present(&journal)?;
            remove_migration_journal(bootstrap_dir)?;
            Ok(bootstrap.clone())
        }
        MigrationPhaseV1::StagedVerified | MigrationPhaseV1::Switching => {
            verify_staging(&journal)?;
            if journal.target.path.exists() {
                remove_target_shell(&journal.target.path, &journal.target_token)?;
            }
            rename_staging_atomically(&journal.staging, &journal.target.path)?;
            journal.phase = MigrationPhaseV1::TargetRenamed;
            write_migration_journal_atomic(bootstrap_dir, &journal)?;
            complete_recovered_switch(bootstrap_dir, bootstrap, &journal)
        }
        MigrationPhaseV1::TargetRenamed => {
            complete_recovered_switch(bootstrap_dir, bootstrap, &journal)
        }
        MigrationPhaseV1::BootstrapCommitted => {
            Err(DataRootError::new("data_root_migration_journal_invalid"))
        }
    }
}

pub fn delete_retained_data_root(
    bootstrap_dir: &Path,
    expected_root: &DataRootRefV1,
) -> Result<BootstrapV1, DataRootError> {
    let mut bootstrap = read_bootstrap(bootstrap_dir)
        .map_err(|_| DataRootError::new("data_root_bootstrap_invalid"))?
        .ok_or_else(|| DataRootError::new("data_root_bootstrap_missing"))?;
    if root_refs_match(&bootstrap.current_root, expected_root) {
        return Err(DataRootError::new("data_root_is_current"));
    }
    let index = bootstrap
        .retained_data_roots
        .iter()
        .position(|retained| root_refs_match(&retained.root, expected_root))
        .ok_or_else(|| DataRootError::new("data_root_not_retained"))?;
    let retained = bootstrap.retained_data_roots[index].clone();
    if is_drive_root(&retained.root.path) || paths_equal(&retained.root.path, bootstrap_dir) {
        return Err(DataRootError::new("data_root_retained_root_changed"));
    }

    if retained.root.path.exists() {
        validate_data_root_marker(&retained.root.path, &retained.root).map_err(map_marker_error)?;
        match retained.layout {
            RetainedLayoutV1::Standard => delete_standard_retained_root(&retained.root.path)?,
            RetainedLayoutV1::LegacyFlat => {
                delete_legacy_retained_root(&retained.root.path, &retained.legacy_entries)?
            }
        }
    }
    bootstrap.retained_data_roots.remove(index);
    write_bootstrap_atomic(bootstrap_dir, &bootstrap)
        .map_err(|_| DataRootError::new("data_root_bootstrap_write_failed"))?;
    Ok(bootstrap)
}

pub fn uninstall_managed_data(bootstrap_dir: &Path) -> Result<(), DataRootError> {
    validate_input_path(bootstrap_dir)?;
    if is_drive_root(bootstrap_dir) {
        return Err(DataRootError::new("data_root_is_drive_root"));
    }
    ensure_no_reparse_ancestors(bootstrap_dir)?;
    let metadata = fs::symlink_metadata(bootstrap_dir)
        .map_err(|_| DataRootError::new("data_root_bootstrap_missing"))?;
    if !metadata.is_dir() || metadata_is_reparse(&metadata) {
        return Err(DataRootError::new("data_root_reparse_point"));
    }
    if read_migration_journal(bootstrap_dir)?.is_some()
        || read_legacy_adoption_journal(bootstrap_dir)?.is_some()
    {
        return Err(DataRootError::new("data_root_migration_in_progress"));
    }

    let bootstrap = read_bootstrap(bootstrap_dir)
        .map_err(|_| DataRootError::new("data_root_bootstrap_invalid"))?
        .ok_or_else(|| DataRootError::new("data_root_bootstrap_missing"))?;
    if is_drive_root(&bootstrap.current_root.path)
        || paths_equal(&bootstrap.current_root.path, bootstrap_dir)
    {
        return Err(DataRootError::new("data_root_retained_root_changed"));
    }
    validate_data_root_marker(&bootstrap.current_root.path, &bootstrap.current_root)
        .map_err(map_marker_error)?;
    ensure_tree_has_no_reparse_points(&bootstrap.current_root.path)?;
    for retained in &bootstrap.retained_data_roots {
        preflight_retained_root_deletion(retained, bootstrap_dir)?;
    }

    for retained in bootstrap.retained_data_roots {
        delete_retained_data_root(bootstrap_dir, &retained.root)?;
    }
    delete_standard_retained_root(&bootstrap.current_root.path)?;
    fs::remove_file(bootstrap_dir.join(BOOTSTRAP_FILE))
        .map_err(|_| DataRootError::new("data_root_bootstrap_write_failed"))?;
    Ok(())
}

fn preflight_retained_root_deletion(
    retained: &RetainedDataRootV1,
    bootstrap_dir: &Path,
) -> Result<(), DataRootError> {
    if is_drive_root(&retained.root.path) || paths_equal(&retained.root.path, bootstrap_dir) {
        return Err(DataRootError::new("data_root_retained_root_changed"));
    }
    if !retained.root.path.exists() {
        return Ok(());
    }
    validate_data_root_marker(&retained.root.path, &retained.root).map_err(map_marker_error)?;
    match retained.layout {
        RetainedLayoutV1::Standard => ensure_tree_has_no_reparse_points(&retained.root.path),
        RetainedLayoutV1::LegacyFlat => {
            validate_legacy_retained_root(&retained.root.path, &retained.legacy_entries)
        }
    }
}

fn delete_standard_retained_root(root: &Path) -> Result<(), DataRootError> {
    ensure_tree_has_no_reparse_points(root)?;
    let mut entries = fs::read_dir(root)
        .map_err(|_| DataRootError::new("data_root_retained_root_changed"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| DataRootError::new("data_root_retained_root_changed"))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if entry.file_name() == DATA_ROOT_MARKER_FILE {
            continue;
        }
        remove_retained_entry(&entry.path())?;
    }
    remove_marker_and_root(root)
}

fn delete_legacy_retained_root(
    root: &Path,
    legacy_entries: &[String],
) -> Result<(), DataRootError> {
    validate_legacy_retained_root(root, legacy_entries)?;
    for name in legacy_entries {
        let path = root.join(name);
        if path.exists() {
            remove_retained_entry(&path)?;
        }
    }
    remove_marker_and_root(root)
}

fn validate_legacy_retained_root(
    root: &Path,
    legacy_entries: &[String],
) -> Result<(), DataRootError> {
    ensure_tree_has_no_reparse_points(root)?;
    let allowed = legacy_entries
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let entries = fs::read_dir(root)
        .map_err(|_| DataRootError::new("data_root_retained_root_changed"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| DataRootError::new("data_root_retained_root_changed"))?;
    for entry in &entries {
        let name = entry
            .file_name()
            .to_str()
            .ok_or_else(|| DataRootError::new("data_root_retained_root_changed"))?
            .to_string();
        if name != DATA_ROOT_MARKER_FILE && !allowed.contains(name.as_str()) {
            return Err(DataRootError::new("data_root_retained_root_changed"));
        }
    }
    Ok(())
}

fn remove_retained_entry(path: &Path) -> Result<(), DataRootError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| DataRootError::new("data_root_retained_root_changed"))?;
    if metadata_is_reparse(&metadata) {
        return Err(DataRootError::new("data_root_reparse_point"));
    }
    if metadata.is_dir() {
        remove_tree_no_follow(path)
    } else if metadata.is_file() {
        fs::remove_file(path).map_err(|_| DataRootError::new("data_root_retained_root_changed"))
    } else {
        Err(DataRootError::new("data_root_retained_root_changed"))
    }
}

fn remove_marker_and_root(root: &Path) -> Result<(), DataRootError> {
    fs::remove_file(root.join(DATA_ROOT_MARKER_FILE))
        .map_err(|_| DataRootError::new("data_root_retained_root_changed"))?;
    fs::remove_dir(root).map_err(|_| DataRootError::new("data_root_retained_root_changed"))
}

fn map_marker_error(error: String) -> DataRootError {
    match error.as_str() {
        "data_root_identity_mismatch" => DataRootError::new("data_root_identity_mismatch"),
        "data_root_path_mismatch" => DataRootError::new("data_root_path_mismatch"),
        _ => DataRootError::new("data_root_marker_missing_or_invalid"),
    }
}

fn complete_recovered_switch(
    bootstrap_dir: &Path,
    bootstrap: &BootstrapV1,
    journal: &MigrationJournalV1,
) -> Result<BootstrapV1, DataRootError> {
    validate_completed_target(journal)?;
    let committed = committed_bootstrap(bootstrap, journal)?;
    write_bootstrap_atomic(bootstrap_dir, &committed)
        .map_err(|_| DataRootError::new("data_root_bootstrap_write_failed"))?;
    remove_migration_journal(bootstrap_dir)?;
    Ok(committed)
}

fn finish_before_journal<T>(
    target: &NormalizedDataRoot,
    error: DataRootError,
) -> Result<T, DataRootError> {
    match target.release_target_ownership() {
        Ok(()) => Err(error),
        Err(cleanup_error) => Err(cleanup_error),
    }
}

fn checkpoint_source_database(source: &Path) -> Result<(), DataRootError> {
    let database_path = source.join("history.sqlite");
    let metadata = fs::symlink_metadata(&database_path)
        .map_err(|_| DataRootError::new("data_root_checkpoint_failed"))?;
    if !metadata.is_file() || metadata_is_reparse(&metadata) {
        return Err(DataRootError::new("data_root_checkpoint_failed"));
    }
    let connection = Connection::open_with_flags(
        &database_path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| DataRootError::new("data_root_checkpoint_failed"))?;
    connection
        .busy_timeout(Duration::ZERO)
        .map_err(|_| DataRootError::new("data_root_checkpoint_failed"))?;
    let (busy, _, _): (i64, i64, i64) = connection
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .map_err(|_| DataRootError::new("data_root_checkpoint_failed"))?;
    if busy != 0 {
        return Err(DataRootError::new("data_root_checkpoint_busy"));
    }
    drop(connection);
    remove_clean_wal_sidecars(source)
}

fn remove_clean_wal_sidecars(root: &Path) -> Result<(), DataRootError> {
    let wal = root.join("history.sqlite-wal");
    match fs::symlink_metadata(&wal) {
        Ok(metadata) if metadata_is_reparse(&metadata) || !metadata.is_file() => {
            return Err(DataRootError::new("data_root_wal_not_clean"));
        }
        Ok(metadata) if metadata.len() != 0 => {
            return Err(DataRootError::new("data_root_wal_not_clean"));
        }
        Ok(_) => {
            // TRUNCATE plus the zero-length check above prove that no WAL
            // frames remain. A dormant SQLite connection can still hold the
            // ordinary file without delete sharing on Windows, so deletion is
            // best-effort and the state is verified again below.
            let _ = fs::remove_file(&wal);
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(_) => return Err(DataRootError::new("data_root_wal_not_clean")),
    }

    let shm = root.join("history.sqlite-shm");
    match fs::symlink_metadata(&shm) {
        Ok(metadata) if metadata_is_reparse(&metadata) || !metadata.is_file() => {
            return Err(DataRootError::new("data_root_wal_not_clean"));
        }
        Ok(_) => {
            // A successful TRUNCATE checkpoint makes the WAL authoritative: no
            // unapplied frames remain. On Windows the app runtime may still own
            // an ordinary SHM handle without delete sharing, so its best-effort
            // removal must not turn an otherwise clean migration into a failure.
            let _ = fs::remove_file(&shm);
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(_) => return Err(DataRootError::new("data_root_wal_not_clean")),
    }

    ensure_clean_source_wal_state(root)
}

fn ensure_clean_source_wal_state(root: &Path) -> Result<(), DataRootError> {
    let shm = root.join("history.sqlite-shm");
    match fs::symlink_metadata(&shm) {
        Ok(metadata) if metadata_is_reparse(&metadata) || !metadata.is_file() => {
            return Err(DataRootError::new("data_root_wal_not_clean"));
        }
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(_) => return Err(DataRootError::new("data_root_wal_not_clean")),
    }
    let wal = root.join("history.sqlite-wal");
    match fs::symlink_metadata(&wal) {
        Ok(metadata) if metadata_is_reparse(&metadata) || !metadata.is_file() => {
            Err(DataRootError::new("data_root_wal_not_clean"))
        }
        Ok(metadata) if metadata.len() != 0 => Err(DataRootError::new("data_root_wal_not_clean")),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(_) => Err(DataRootError::new("data_root_wal_not_clean")),
    }
}

fn create_owned_staging(journal: &MigrationJournalV1) -> Result<(), DataRootError> {
    ensure_no_reparse_ancestors(
        journal
            .staging
            .parent()
            .ok_or_else(|| DataRootError::new("data_root_copy_failed"))?,
    )?;
    if journal.staging.exists() {
        return Err(DataRootError::new("data_root_copy_failed"));
    }
    fs::create_dir(&journal.staging).map_err(|_| DataRootError::new("data_root_copy_failed"))?;
    let marker = DataRootMarkerV1::new(journal.target.root_id.clone())
        .map_err(|_| DataRootError::new("data_root_copy_failed"))?;
    write_data_root_marker_atomic(&journal.staging, &marker)
        .map_err(|_| DataRootError::new("data_root_copy_failed"))
}

fn copy_data_root_tree(source: &Path, staging: &Path) -> Result<(), DataRootError> {
    ensure_tree_has_no_reparse_points(source)?;
    let mut pending = vec![source.to_path_buf()];
    while let Some(source_dir) = pending.pop() {
        let mut entries = fs::read_dir(&source_dir)
            .map_err(|_| DataRootError::new("data_root_copy_failed"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| DataRootError::new("data_root_copy_failed"))?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let source_path = entry.path();
            let metadata = fs::symlink_metadata(&source_path)
                .map_err(|_| DataRootError::new("data_root_copy_failed"))?;
            if metadata_is_reparse(&metadata) {
                return Err(DataRootError::new("data_root_reparse_point"));
            }
            let relative = source_path
                .strip_prefix(source)
                .map_err(|_| DataRootError::new("data_root_copy_failed"))?;
            let relative_text = relative_path_string(source, &source_path)?;
            if relative_text == DATA_ROOT_MARKER_FILE || relative_text == "history.sqlite-shm" {
                continue;
            }
            if relative_text == "history.sqlite-wal" && metadata.len() == 0 {
                continue;
            }
            let target_path = staging.join(relative);
            if metadata.is_dir() {
                fs::create_dir(&target_path)
                    .map_err(|_| DataRootError::new("data_root_copy_failed"))?;
                pending.push(source_path);
            } else if metadata.is_file() {
                copy_file_no_follow(&source_path, &target_path)?;
            } else {
                return Err(DataRootError::new("data_root_copy_failed"));
            }
        }
    }
    Ok(())
}

fn copy_file_no_follow(source: &Path, target: &Path) -> Result<(), DataRootError> {
    let mut source_file = open_file_no_follow(source)?;
    let metadata = source_file
        .metadata()
        .map_err(|_| DataRootError::new("data_root_copy_failed"))?;
    if !metadata.is_file() || metadata_is_reparse(&metadata) {
        return Err(DataRootError::new("data_root_reparse_point"));
    }
    let mut target_file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(target)
        .map_err(|_| DataRootError::new("data_root_copy_failed"))?;
    std::io::copy(&mut source_file, &mut target_file)
        .and_then(|_| target_file.sync_all())
        .map_err(|_| DataRootError::new("data_root_copy_failed"))?;
    Ok(())
}

#[cfg(windows)]
fn open_file_no_follow(path: &Path) -> Result<File, DataRootError> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;

    OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|_| DataRootError::new("data_root_copy_failed"))
}

#[cfg(not(windows))]
fn open_file_no_follow(path: &Path) -> Result<File, DataRootError> {
    File::open(path).map_err(|_| DataRootError::new("data_root_copy_failed"))
}

fn rewrite_browser_states_in_staging(source: &Path, staging: &Path) -> Result<(), DataRootError> {
    let source_profiles = source.join("profiles");
    if !source_profiles.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(&source_profiles)
        .map_err(|_| DataRootError::new("data_root_browser_state_rewrite_failed"))?
    {
        let source_account = entry
            .map_err(|_| DataRootError::new("data_root_browser_state_rewrite_failed"))?
            .path();
        let metadata = fs::symlink_metadata(&source_account)
            .map_err(|_| DataRootError::new("data_root_browser_state_rewrite_failed"))?;
        if metadata_is_reparse(&metadata) {
            return Err(DataRootError::new("data_root_reparse_point"));
        }
        if !metadata.is_dir() {
            continue;
        }
        let name = source_account
            .file_name()
            .ok_or_else(|| DataRootError::new("data_root_browser_state_rewrite_failed"))?;
        rewrite_browser_state_for_migration(&source_account, &staging.join("profiles").join(name))
            .map_err(|_| DataRootError::new("data_root_browser_state_rewrite_failed"))?;
    }
    Ok(())
}

fn verify_staging(journal: &MigrationJournalV1) -> Result<(), DataRootError> {
    validate_data_root_marker(
        &journal.staging,
        &DataRootRefV1 {
            path: journal.staging.clone(),
            root_id: journal.target.root_id.clone(),
        },
    )
    .map_err(|_| DataRootError::new("data_root_verification_failed"))?;
    verify_data_inventory(&journal.staging, &journal.source_inventory)
}

fn validate_completed_target(journal: &MigrationJournalV1) -> Result<(), DataRootError> {
    validate_data_root_marker(&journal.target.path, &journal.target)
        .map_err(|_| DataRootError::new("data_root_verification_failed"))?;
    verify_data_inventory(&journal.target.path, &journal.source_inventory)
}

fn remove_target_shell(target: &Path, token: &str) -> Result<(), DataRootError> {
    ensure_target_contains_only_ownership(target, token)?;
    fs::remove_file(target.join(TARGET_OWNERSHIP_FILE))
        .map_err(|_| DataRootError::new("data_root_switch_failed"))?;
    fs::remove_dir(target).map_err(|_| DataRootError::new("data_root_switch_failed"))
}

fn committed_bootstrap(
    bootstrap: &BootstrapV1,
    journal: &MigrationJournalV1,
) -> Result<BootstrapV1, DataRootError> {
    let mut committed = bootstrap.clone();
    committed.current_root = journal.target.clone();
    if !committed
        .retained_data_roots
        .iter()
        .any(|retained| root_refs_match(&retained.root, &journal.source))
    {
        committed.retained_data_roots.push(RetainedDataRootV1 {
            root: journal.source.clone(),
            layout: RetainedLayoutV1::Standard,
            legacy_entries: Vec::new(),
        });
    }
    Ok(committed)
}

fn root_refs_match(left: &DataRootRefV1, right: &DataRootRefV1) -> bool {
    left.root_id == right.root_id && paths_equal(&left.path, &right.path)
}

fn remove_owned_staging_if_present(journal: &MigrationJournalV1) -> Result<(), DataRootError> {
    if !journal.staging.exists() {
        return Ok(());
    }
    validate_data_root_marker(
        &journal.staging,
        &DataRootRefV1 {
            path: journal.staging.clone(),
            root_id: journal.target.root_id.clone(),
        },
    )
    .map_err(|_| DataRootError::new("data_root_migration_journal_invalid"))?;
    remove_tree_no_follow(&journal.staging)
}

fn remove_owned_target_shell_if_present(journal: &MigrationJournalV1) -> Result<(), DataRootError> {
    if !journal.target.path.exists() {
        return Ok(());
    }
    remove_target_shell(&journal.target.path, &journal.target_token)
}

fn remove_tree_no_follow(root: &Path) -> Result<(), DataRootError> {
    ensure_tree_has_no_reparse_points(root)?;
    let mut directories = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        directories.push(directory.clone());
        for entry in
            fs::read_dir(&directory).map_err(|_| DataRootError::new("data_root_switch_failed"))?
        {
            let path = entry
                .map_err(|_| DataRootError::new("data_root_switch_failed"))?
                .path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|_| DataRootError::new("data_root_switch_failed"))?;
            if metadata_is_reparse(&metadata) {
                return Err(DataRootError::new("data_root_reparse_point"));
            }
            if metadata.is_dir() {
                pending.push(path);
            } else if metadata.is_file() {
                fs::remove_file(path).map_err(|_| DataRootError::new("data_root_switch_failed"))?;
            } else {
                return Err(DataRootError::new("data_root_switch_failed"));
            }
        }
    }
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for directory in directories {
        fs::remove_dir(directory).map_err(|_| DataRootError::new("data_root_switch_failed"))?;
    }
    Ok(())
}

fn remove_migration_journal(bootstrap_dir: &Path) -> Result<(), DataRootError> {
    match fs::remove_file(bootstrap_dir.join(MIGRATION_JOURNAL_FILE)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(_) => Err(DataRootError::new(
            "data_root_migration_journal_write_failed",
        )),
    }
}

#[cfg(windows)]
fn rename_staging_atomically(source: &Path, target: &Path) -> Result<(), DataRootError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};

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
    if unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), MOVEFILE_WRITE_THROUGH) } == 0 {
        return Err(DataRootError::new("data_root_switch_failed"));
    }
    Ok(())
}

#[cfg(not(windows))]
fn rename_staging_atomically(source: &Path, target: &Path) -> Result<(), DataRootError> {
    fs::rename(source, target).map_err(|_| DataRootError::new("data_root_switch_failed"))
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TargetOwnershipV1 {
    schema_version: u32,
    token: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BrowserStateSemanticV1 {
    browser_id: BrowserId,
    executable_path: String,
    profile_generation: String,
    selected_at: String,
    validation: Option<LoginValidationSemanticV1>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LoginValidationSemanticV1 {
    validation_id: Option<String>,
    browser_id: BrowserId,
    executable_path: String,
    validated_at: String,
    history_url: String,
}

pub fn build_data_inventory(root: &Path) -> Result<DataInventory, DataRootError> {
    build_data_inventory_with_source_sidecars(root, false)
}

fn build_source_data_inventory(root: &Path) -> Result<DataInventory, DataRootError> {
    build_data_inventory_with_source_sidecars(root, true)
}

fn build_data_inventory_with_source_sidecars(
    root: &Path,
    allow_clean_source_shm: bool,
) -> Result<DataInventory, DataRootError> {
    let metadata = fs::symlink_metadata(root)
        .map_err(|_| DataRootError::new("data_root_verification_failed"))?;
    if !metadata.is_dir() {
        return Err(DataRootError::new("data_root_verification_failed"));
    }
    ensure_tree_has_no_reparse_points(root)?;
    let (semantic_browser_hashes, browser_state_files) = browser_state_inventory(root)?;
    let mut inventory = DataInventory {
        file_count: 0,
        total_bytes: 0,
        critical_hashes: BTreeMap::new(),
        semantic_browser_hashes,
    };
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let mut entries = fs::read_dir(&directory)
            .map_err(|_| DataRootError::new("data_root_verification_failed"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| DataRootError::new("data_root_verification_failed"))?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|_| DataRootError::new("data_root_verification_failed"))?;
            if metadata_is_reparse(&metadata) {
                return Err(DataRootError::new("data_root_reparse_point"));
            }
            if metadata.is_dir() {
                pending.push(path);
                continue;
            }
            if !metadata.is_file() {
                return Err(DataRootError::new("data_root_verification_failed"));
            }
            let relative = relative_path_string(root, &path)?;
            if relative == DATA_ROOT_MARKER_FILE || browser_state_files.contains(&path) {
                continue;
            }
            if path.file_name().and_then(|value| value.to_str()) == Some(".login-validated.json")
                || path.file_name().and_then(|value| value.to_str())
                    == Some("browser-selection.json")
            {
                return Err(DataRootError::new("data_root_verification_failed"));
            }
            if relative == "history.sqlite-shm" {
                if allow_clean_source_shm {
                    continue;
                }
                return Err(DataRootError::new("data_root_wal_not_clean"));
            }
            if relative == "history.sqlite-wal" {
                if metadata.len() != 0 {
                    return Err(DataRootError::new("data_root_wal_not_clean"));
                }
                continue;
            }
            inventory.file_count = inventory
                .file_count
                .checked_add(1)
                .ok_or_else(|| DataRootError::new("data_root_verification_failed"))?;
            inventory.total_bytes = inventory
                .total_bytes
                .checked_add(metadata.len())
                .ok_or_else(|| DataRootError::new("data_root_verification_failed"))?;
            if matches!(relative.as_str(), "history.sqlite" | "settings.json") {
                inventory
                    .critical_hashes
                    .insert(relative, hash_file(&path)?);
            }
        }
    }
    Ok(inventory)
}

pub fn verify_data_inventory(root: &Path, expected: &DataInventory) -> Result<(), DataRootError> {
    let actual = build_data_inventory(root)?;
    if &actual != expected {
        return Err(DataRootError::new("data_root_verification_failed"));
    }
    Ok(())
}

fn browser_state_inventory(
    root: &Path,
) -> Result<(BTreeMap<String, String>, HashSet<PathBuf>), DataRootError> {
    let profiles = root.join("profiles");
    if !profiles.exists() {
        return Ok((BTreeMap::new(), HashSet::new()));
    }
    let mut hashes = BTreeMap::new();
    let mut state_files = HashSet::new();
    for entry in
        fs::read_dir(&profiles).map_err(|_| DataRootError::new("data_root_verification_failed"))?
    {
        let account_root = entry
            .map_err(|_| DataRootError::new("data_root_verification_failed"))?
            .path();
        let metadata = fs::symlink_metadata(&account_root)
            .map_err(|_| DataRootError::new("data_root_verification_failed"))?;
        if metadata_is_reparse(&metadata) {
            return Err(DataRootError::new("data_root_reparse_point"));
        }
        if !metadata.is_dir() {
            continue;
        }
        let selection_path = account_root.join("browser-selection.json");
        if !selection_path.exists() {
            continue;
        }
        let selection = read_browser_selection(&account_root)
            .map_err(|_| DataRootError::new("data_root_verification_failed"))?
            .ok_or_else(|| DataRootError::new("data_root_verification_failed"))?;
        let validation = read_login_validation(&account_root, &selection)
            .map_err(|_| DataRootError::new("data_root_verification_failed"))?;
        let semantic = BrowserStateSemanticV1 {
            browser_id: selection.browser_id,
            executable_path: canonical_windows_path(&selection.executable_path),
            profile_generation: selection
                .profile_generation
                .clone()
                .ok_or_else(|| DataRootError::new("data_root_verification_failed"))?,
            selected_at: selection.selected_at,
            validation: validation
                .as_ref()
                .map(|validation| LoginValidationSemanticV1 {
                    validation_id: validation.validation_id.clone(),
                    browser_id: validation.browser_id,
                    executable_path: canonical_windows_path(&validation.executable_path),
                    validated_at: validation.validated_at.clone(),
                    history_url: validation.history_url.clone(),
                }),
        };
        let bytes = serde_json::to_vec(&semantic)
            .map_err(|_| DataRootError::new("data_root_verification_failed"))?;
        let key = relative_path_string(root, &account_root)?;
        hashes.insert(key, format!("{:x}", Sha256::digest(bytes)));
        state_files.insert(selection_path);
        state_files.insert(profile_generation_path(
            &account_root,
            selection.browser_id,
        ));
        if validation.is_some() {
            state_files.insert(
                browser_profile_path(&account_root, selection.browser_id)
                    .join(".login-validated.json"),
            );
        }
    }
    Ok((hashes, state_files))
}

fn relative_path_string(root: &Path, path: &Path) -> Result<String, DataRootError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| DataRootError::new("data_root_verification_failed"))?;
    let value = relative
        .to_str()
        .ok_or_else(|| DataRootError::new("data_root_not_unicode"))?;
    Ok(value.replace('\\', "/"))
}

fn hash_file(path: &Path) -> Result<String, DataRootError> {
    let mut file =
        File::open(path).map_err(|_| DataRootError::new("data_root_verification_failed"))?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| DataRootError::new("data_root_verification_failed"))?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub fn normalize_requested_root(
    context: &DataRootContext,
    requested: &Path,
) -> Result<NormalizedDataRoot, DataRootError> {
    validate_input_path(requested)?;
    if is_unc_path(requested) {
        return Err(DataRootError::new("data_root_is_remote"));
    }
    if is_drive_root(requested) {
        return Err(DataRootError::new("data_root_is_drive_root"));
    }

    let current_root = canonicalize_existing_directory(&context.app_data_dir)?;
    let install_dir = canonicalize_existing_directory(&context.install_dir)?;
    let requested_path = canonicalize_target_path(requested)?;
    let adjusted_to_install_data = paths_equal(&requested_path, &install_dir);
    let target = if adjusted_to_install_data {
        canonicalize_target_path(&install_dir.join("Data"))?
    } else {
        requested_path
    };

    if is_drive_root(&target) {
        return Err(DataRootError::new("data_root_is_drive_root"));
    }
    reject_remote_volume(&target)?;
    if paths_overlap(&target, &current_root)? {
        return Err(DataRootError::new("data_root_overlaps_current"));
    }

    let parent = target
        .parent()
        .ok_or_else(|| DataRootError::new("data_root_is_drive_root"))?;
    ensure_no_reparse_ancestors(parent)?;
    let parent_identity = directory_identity(parent)?;
    let target_existed = match fs::symlink_metadata(&target) {
        Ok(metadata) => {
            if !metadata.is_dir() {
                return Err(DataRootError::new("data_root_not_empty"));
            }
            if metadata_is_reparse(&metadata) {
                return Err(DataRootError::new("data_root_reparse_point"));
            }
            ensure_tree_has_no_reparse_points(&target)?;
            if fs::read_dir(&target)
                .map_err(|_| DataRootError::new("data_root_permission_denied"))?
                .next()
                .is_some()
            {
                return Err(DataRootError::new("data_root_not_empty"));
            }
            true
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(&target)
                .map_err(|_| DataRootError::new("data_root_permission_denied"))?;
            false
        }
        Err(_) => return Err(DataRootError::new("data_root_permission_denied")),
    };

    let ownership_token = Uuid::new_v4().simple().to_string();
    if let Err(error) = write_target_ownership(&target, &ownership_token) {
        if !target_existed {
            let _ = fs::remove_dir(&target);
        }
        return Err(error);
    }
    let result = NormalizedDataRoot {
        path: target,
        parent_identity,
        target_existed,
        ownership_token,
        adjusted_to_install_data,
    };
    if let Err(error) = result.verify_target_ownership() {
        let _ = fs::remove_file(result.path.join(TARGET_OWNERSHIP_FILE));
        if !result.target_existed {
            let _ = fs::remove_dir(&result.path);
        }
        return Err(error);
    }
    Ok(result)
}

pub fn probe_data_root_permissions(path: &Path) -> Result<(), DataRootError> {
    probe_data_root_permissions_with_observer(path, |_| Ok(()))
}

pub fn validate_data_root_request(
    context: &DataRootContext,
    requested: &Path,
) -> Result<DataRootValidationView, DataRootError> {
    let normalized = normalize_requested_root(context, requested)?;
    let probe = probe_data_root_permissions(&normalized.path);
    let cleanup = normalized.release_target_ownership();
    match (probe, cleanup) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(DataRootValidationView {
            requested_path: requested.to_path_buf(),
            normalized_path: normalized.path,
            adjusted_to_install_data: normalized.adjusted_to_install_data,
        }),
    }
}

#[doc(hidden)]
pub fn probe_data_root_permissions_with_observer<F>(
    path: &Path,
    observer: F,
) -> Result<(), DataRootError>
where
    F: FnOnce(&Path) -> Result<(), DataRootError>,
{
    validate_input_path(path)?;
    if is_unc_path(path) {
        return Err(DataRootError::new("data_root_is_remote"));
    }
    if is_drive_root(path) {
        return Err(DataRootError::new("data_root_is_drive_root"));
    }
    reject_remote_volume(path)?;
    let existed = path.try_exists().unwrap_or(false);
    if existed {
        let metadata = fs::symlink_metadata(path)
            .map_err(|_| DataRootError::new("data_root_permission_denied"))?;
        if !metadata.is_dir() {
            return Err(DataRootError::new("data_root_permission_denied"));
        }
        if metadata_is_reparse(&metadata) {
            return Err(DataRootError::new("data_root_reparse_point"));
        }
    } else {
        let parent = path
            .parent()
            .ok_or_else(|| DataRootError::new("data_root_permission_denied"))?;
        ensure_no_reparse_ancestors(parent)?;
        fs::create_dir(path).map_err(|_| DataRootError::new("data_root_permission_denied"))?;
    }

    let probe_dir = path.join(format!(
        ".emily-permission-probe-{}",
        Uuid::new_v4().simple()
    ));
    let result = run_permission_probe(&probe_dir, observer);
    let cleanup = cleanup_permission_probe(&probe_dir, path, !existed);
    match (result, cleanup) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn run_permission_probe<F>(probe_dir: &Path, observer: F) -> Result<(), DataRootError>
where
    F: FnOnce(&Path) -> Result<(), DataRootError>,
{
    fs::create_dir(probe_dir).map_err(|_| DataRootError::new("data_root_permission_denied"))?;
    let source_file = probe_dir.join("write-test.tmp");
    let renamed_file = probe_dir.join("rename-test.tmp");
    let mut file = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(&source_file)
        .map_err(|_| DataRootError::new("data_root_permission_denied"))?;
    file.write_all(PROBE_FILE_CONTENTS)
        .and_then(|_| file.sync_all())
        .map_err(|_| DataRootError::new("data_root_permission_denied"))?;
    file.rewind()
        .map_err(|_| DataRootError::new("data_root_permission_denied"))?;
    let mut read_back = Vec::new();
    file.read_to_end(&mut read_back)
        .map_err(|_| DataRootError::new("data_root_permission_denied"))?;
    if read_back != PROBE_FILE_CONTENTS {
        return Err(DataRootError::new("data_root_permission_denied"));
    }
    drop(file);
    fs::rename(&source_file, &renamed_file)
        .map_err(|_| DataRootError::new("data_root_permission_denied"))?;
    if fs::read(&renamed_file).map_err(|_| DataRootError::new("data_root_permission_denied"))?
        != PROBE_FILE_CONTENTS
    {
        return Err(DataRootError::new("data_root_permission_denied"));
    }

    let database = probe_dir.join("probe.sqlite");
    let first = Connection::open(&database)
        .map_err(|_| DataRootError::new("data_root_wal_probe_failed"))?;
    first
        .busy_timeout(Duration::ZERO)
        .map_err(|_| DataRootError::new("data_root_wal_probe_failed"))?;
    let journal_mode: String = first
        .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
        .map_err(|_| DataRootError::new("data_root_wal_probe_failed"))?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
        return Err(DataRootError::new("data_root_wal_probe_failed"));
    }
    first
        .execute_batch(
            "CREATE TABLE probe(value INTEGER NOT NULL); INSERT INTO probe(value) VALUES (1);",
        )
        .map_err(|_| DataRootError::new("data_root_wal_probe_failed"))?;
    let second = Connection::open(&database)
        .map_err(|_| DataRootError::new("data_root_wal_probe_failed"))?;
    second
        .busy_timeout(Duration::ZERO)
        .map_err(|_| DataRootError::new("data_root_wal_probe_failed"))?;
    let count: i64 = second
        .query_row("SELECT COUNT(*) FROM probe", [], |row| row.get(0))
        .map_err(|_| DataRootError::new("data_root_wal_probe_failed"))?;
    if count != 1 {
        return Err(DataRootError::new("data_root_wal_probe_failed"));
    }
    first
        .execute_batch("BEGIN IMMEDIATE; INSERT INTO probe(value) VALUES (2);")
        .map_err(|_| DataRootError::new("data_root_wal_probe_failed"))?;
    match second.execute("INSERT INTO probe(value) VALUES (3)", []) {
        Err(rusqlite::Error::SqliteFailure(error, _))
            if matches!(
                error.code,
                ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked
            ) => {}
        _ => return Err(DataRootError::new("data_root_wal_probe_failed")),
    }
    observer(probe_dir)?;
    first
        .execute_batch("ROLLBACK")
        .map_err(|_| DataRootError::new("data_root_wal_probe_failed"))?;
    Ok(())
}

fn cleanup_permission_probe(
    probe_dir: &Path,
    target: &Path,
    remove_target: bool,
) -> Result<(), DataRootError> {
    for name in [
        "write-test.tmp",
        "rename-test.tmp",
        "probe.sqlite",
        "probe.sqlite-wal",
        "probe.sqlite-shm",
        "probe.sqlite-journal",
    ] {
        remove_file_if_present(&probe_dir.join(name))?;
    }
    if probe_dir.exists() {
        fs::remove_dir(probe_dir).map_err(|_| DataRootError::new("data_root_permission_denied"))?;
    }
    if remove_target && target.exists() {
        fs::remove_dir(target).map_err(|_| DataRootError::new("data_root_permission_denied"))?;
    }
    Ok(())
}

fn remove_file_if_present(path: &Path) -> Result<(), DataRootError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(DataRootError::new("data_root_permission_denied")),
    }
}

fn validate_input_path(path: &Path) -> Result<(), DataRootError> {
    if !path.is_absolute() {
        return Err(DataRootError::new("data_root_not_absolute"));
    }
    if path.to_str().is_none() {
        return Err(DataRootError::new("data_root_not_unicode"));
    }
    Ok(())
}

fn canonicalize_existing_directory(path: &Path) -> Result<PathBuf, DataRootError> {
    validate_input_path(path)?;
    ensure_no_reparse_ancestors(path)?;
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| DataRootError::new("data_root_permission_denied"))?;
    if !metadata.is_dir() || metadata_is_reparse(&metadata) {
        return Err(DataRootError::new("data_root_reparse_point"));
    }
    canonicalize_without_verbatim(path)
}

fn canonicalize_target_path(path: &Path) -> Result<PathBuf, DataRootError> {
    validate_input_path(path)?;
    ensure_no_reparse_ancestors(path)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata_is_reparse(&metadata) {
                return Err(DataRootError::new("data_root_reparse_point"));
            }
            canonicalize_without_verbatim(path)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = path
                .parent()
                .ok_or_else(|| DataRootError::new("data_root_is_drive_root"))?;
            let file_name = path
                .file_name()
                .ok_or_else(|| DataRootError::new("data_root_is_drive_root"))?;
            Ok(canonicalize_existing_directory(parent)?.join(file_name))
        }
        Err(_) => Err(DataRootError::new("data_root_permission_denied")),
    }
}

fn canonicalize_without_verbatim(path: &Path) -> Result<PathBuf, DataRootError> {
    let canonical =
        fs::canonicalize(path).map_err(|_| DataRootError::new("data_root_permission_denied"))?;
    let raw = canonical
        .to_str()
        .ok_or_else(|| DataRootError::new("data_root_not_unicode"))?;
    if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
        return Ok(PathBuf::from(format!(r"\\{rest}")));
    }
    if let Some(rest) = raw.strip_prefix(r"\\?\") {
        return Ok(PathBuf::from(rest));
    }
    Ok(canonical)
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    path_key(left).is_some_and(|left| path_key(right).is_some_and(|right| left == right))
}

fn paths_overlap(left: &Path, right: &Path) -> Result<bool, DataRootError> {
    let left = path_key(left).ok_or_else(|| DataRootError::new("data_root_not_unicode"))?;
    let right = path_key(right).ok_or_else(|| DataRootError::new("data_root_not_unicode"))?;
    Ok(left == right
        || left.starts_with(&(right.clone() + "\\"))
        || right.starts_with(&(left + "\\")))
}

fn path_key(path: &Path) -> Option<String> {
    path.to_str().map(|value| {
        let normalized = value.replace('/', "\\");
        let normalized = normalized
            .strip_prefix(r"\\?\UNC\")
            .map(|rest| format!(r"\\{rest}"))
            .or_else(|| normalized.strip_prefix(r"\\?\").map(str::to_string))
            .unwrap_or(normalized);
        normalized.trim_end_matches('\\').to_lowercase()
    })
}

fn is_unc_path(path: &Path) -> bool {
    path.to_str().is_some_and(|value| {
        let normalized = value.replace('/', "\\");
        normalized.starts_with(r"\\?\UNC\")
            || (!normalized.starts_with(r"\\?\") && normalized.starts_with(r"\\"))
    })
}

fn is_drive_root(path: &Path) -> bool {
    path.to_str().is_some_and(|value| {
        let normalized = value.replace('/', "\\");
        let bytes = normalized.as_bytes();
        bytes.len() == 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\'
    })
}

fn ensure_target_contains_only_ownership(
    target: &Path,
    expected_token: &str,
) -> Result<(), DataRootError> {
    ensure_tree_has_no_reparse_points(target)?;
    let mut entries =
        fs::read_dir(target).map_err(|_| DataRootError::new("data_root_permission_denied"))?;
    let entry = entries
        .next()
        .transpose()
        .map_err(|_| DataRootError::new("data_root_permission_denied"))?
        .ok_or_else(|| DataRootError::new("data_root_permission_denied"))?;
    if entry.file_name() != TARGET_OWNERSHIP_FILE || entries.next().is_some() {
        return Err(DataRootError::new("data_root_not_empty"));
    }
    let contents =
        fs::read(entry.path()).map_err(|_| DataRootError::new("data_root_permission_denied"))?;
    let ownership: TargetOwnershipV1 = serde_json::from_slice(&contents)
        .map_err(|_| DataRootError::new("data_root_permission_denied"))?;
    if ownership.schema_version != 1 || ownership.token != expected_token {
        return Err(DataRootError::new("data_root_permission_denied"));
    }
    Ok(())
}

fn write_target_ownership(target: &Path, token: &str) -> Result<(), DataRootError> {
    let value = TargetOwnershipV1 {
        schema_version: 1,
        token: token.to_string(),
    };
    let bytes = serde_json::to_vec(&value)
        .map_err(|_| DataRootError::new("data_root_permission_denied"))?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(target.join(TARGET_OWNERSHIP_FILE))
        .map_err(|_| DataRootError::new("data_root_permission_denied"))?;
    let result = file.write_all(&bytes).and_then(|_| file.sync_all());
    drop(file);
    if result.is_err() {
        let _ = fs::remove_file(target.join(TARGET_OWNERSHIP_FILE));
        return Err(DataRootError::new("data_root_permission_denied"));
    }
    Ok(())
}

fn ensure_no_reparse_ancestors(path: &Path) -> Result<(), DataRootError> {
    let mut ancestors = path.ancestors().collect::<Vec<_>>();
    ancestors.reverse();
    for ancestor in ancestors {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata_is_reparse(&metadata) => {
                return Err(DataRootError::new("data_root_reparse_point"));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(DataRootError::new("data_root_permission_denied")),
        }
    }
    Ok(())
}

fn ensure_tree_has_no_reparse_points(path: &Path) -> Result<(), DataRootError> {
    let mut pending = vec![path.to_path_buf()];
    while let Some(current) = pending.pop() {
        let metadata = fs::symlink_metadata(&current)
            .map_err(|_| DataRootError::new("data_root_permission_denied"))?;
        if metadata_is_reparse(&metadata) {
            return Err(DataRootError::new("data_root_reparse_point"));
        }
        if metadata.is_dir() {
            for entry in fs::read_dir(&current)
                .map_err(|_| DataRootError::new("data_root_permission_denied"))?
            {
                pending.push(
                    entry
                        .map_err(|_| DataRootError::new("data_root_permission_denied"))?
                        .path(),
                );
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
fn metadata_is_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_is_reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(windows)]
fn reject_remote_volume(path: &Path) -> Result<(), DataRootError> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetVolumePathNameW};

    let probe_path = if path.exists() {
        path
    } else {
        path.parent()
            .ok_or_else(|| DataRootError::new("data_root_is_drive_root"))?
    };
    let wide = probe_path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut volume = vec![0u16; 32_768];
    let success =
        unsafe { GetVolumePathNameW(wide.as_ptr(), volume.as_mut_ptr(), volume.len() as u32) };
    if success == 0 {
        return Err(DataRootError::new("data_root_permission_denied"));
    }
    let volume_length = volume.iter().position(|value| *value == 0).unwrap_or(0);
    let volume_root = PathBuf::from(std::ffi::OsString::from_wide(&volume[..volume_length]));
    if paths_equal(path, &volume_root) {
        return Err(DataRootError::new("data_root_is_drive_root"));
    }
    drive_type_result(unsafe { GetDriveTypeW(volume.as_ptr()) })
}

#[cfg(windows)]
fn drive_type_result(drive_type: u32) -> Result<(), DataRootError> {
    use windows_sys::Win32::System::WindowsProgramming::{
        DRIVE_NO_ROOT_DIR, DRIVE_REMOTE, DRIVE_UNKNOWN,
    };

    match drive_type {
        DRIVE_REMOTE => Err(DataRootError::new("data_root_is_remote")),
        DRIVE_UNKNOWN | DRIVE_NO_ROOT_DIR => Err(DataRootError::new("data_root_permission_denied")),
        _ => Ok(()),
    }
}

#[cfg(not(windows))]
fn reject_remote_volume(_path: &Path) -> Result<(), DataRootError> {
    Ok(())
}

#[cfg(windows)]
fn directory_identity(path: &Path) -> Result<DirectoryIdentity, DataRootError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{
            CreateFileW, GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
            FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
            FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
            OPEN_EXISTING,
        },
    };

    let canonical_path = canonicalize_without_verbatim(path)?;
    let wide = canonical_path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(DataRootError::new("data_root_permission_denied"));
    }
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    let success = unsafe { GetFileInformationByHandle(handle, &mut information) };
    unsafe {
        CloseHandle(handle);
    }
    if success == 0 {
        return Err(DataRootError::new("data_root_permission_denied"));
    }
    if information.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(DataRootError::new("data_root_reparse_point"));
    }
    Ok(DirectoryIdentity {
        canonical_path,
        volume_serial_number: information.dwVolumeSerialNumber,
        file_index: ((information.nFileIndexHigh as u64) << 32) | information.nFileIndexLow as u64,
    })
}

#[cfg(all(test, windows))]
mod tests {
    use super::drive_type_result;
    use windows_sys::Win32::System::WindowsProgramming::{
        DRIVE_FIXED, DRIVE_NO_ROOT_DIR, DRIVE_REMOTE, DRIVE_UNKNOWN,
    };

    #[test]
    fn mapped_remote_drive_type_is_rejected() {
        assert_eq!(
            drive_type_result(DRIVE_REMOTE).unwrap_err().code(),
            "data_root_is_remote"
        );
        assert_eq!(
            drive_type_result(DRIVE_UNKNOWN).unwrap_err().code(),
            "data_root_permission_denied"
        );
        assert_eq!(
            drive_type_result(DRIVE_NO_ROOT_DIR).unwrap_err().code(),
            "data_root_permission_denied"
        );
        drive_type_result(DRIVE_FIXED).unwrap();
    }
}

#[cfg(not(windows))]
fn directory_identity(path: &Path) -> Result<DirectoryIdentity, DataRootError> {
    Ok(DirectoryIdentity {
        canonical_path: canonicalize_without_verbatim(path)?,
        volume_serial_number: 0,
        file_index: 0,
    })
}
