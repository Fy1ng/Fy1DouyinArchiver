use crate::schema::assert_schema_latest;
use chrono::DateTime;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const BACKUP_MAGIC: &[u8; 8] = b"EMLYBKP1";
const BACKUP_ARCHIVE_SCHEMA_VERSION: u32 = 1;
const DATABASE_SCHEMA_VERSION: i64 = 8;
const MAX_MANIFEST_BYTES: u64 = 1_048_576;
const MAX_SETTINGS_BYTES: u64 = 16 * 1_048_576;
const MAX_DATABASE_BYTES: u64 = 1_099_511_627_776;
const HEADER_BYTES: u64 = 8 + 4 + 8 + 8;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackupPayloadManifestV1 {
    pub name: String,
    pub byte_size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackupManifestV1 {
    pub schema_version: u32,
    pub created_at: String,
    pub application_version: String,
    pub database_schema_version: i64,
    pub settings_schema_version: i64,
    pub payloads: Vec<BackupPayloadManifestV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackupArchiveSummaryV1 {
    pub created_at: String,
    pub application_version: String,
    pub database_schema_version: i64,
    pub settings_schema_version: i64,
    pub byte_size: u64,
}

pub struct BackupArchiveCreateRequestV1<'a> {
    pub database_path: &'a Path,
    pub settings_bytes: &'a [u8],
    pub settings_schema_version: i64,
    pub destination: &'a Path,
    pub created_at: &'a str,
    pub application_version: &'a str,
}

#[derive(Debug)]
pub struct ExtractedBackupArchiveV1 {
    pub manifest: BackupManifestV1,
    pub summary: BackupArchiveSummaryV1,
    pub settings_bytes: Vec<u8>,
    pub database_path: PathBuf,
}

pub fn create_backup_archive(
    request: BackupArchiveCreateRequestV1<'_>,
) -> Result<BackupArchiveSummaryV1, String> {
    validate_create_request(&request)?;
    let destination_parent = request
        .destination
        .parent()
        .ok_or_else(|| "backup_destination_has_no_parent".to_string())?;
    fs::create_dir_all(io_path(destination_parent)).map_err(|error| error.to_string())?;
    let snapshot_path = temporary_sibling(request.destination, "snapshot.sqlite")?;
    let temporary_archive = temporary_sibling(request.destination, "archive.tmp")?;

    let result = (|| {
        create_database_snapshot(request.database_path, &snapshot_path)?;
        let database_size = file_size(&snapshot_path)?;
        if database_size > MAX_DATABASE_BYTES {
            return Err("backup_database_too_large".to_string());
        }
        let database_sha256 = sha256_file(&snapshot_path)?;
        let settings_size = u64::try_from(request.settings_bytes.len())
            .map_err(|_| "backup_settings_too_large".to_string())?;
        let settings_sha256 = sha256_bytes(request.settings_bytes);
        let manifest = BackupManifestV1 {
            schema_version: BACKUP_ARCHIVE_SCHEMA_VERSION,
            created_at: request.created_at.to_string(),
            application_version: request.application_version.to_string(),
            database_schema_version: DATABASE_SCHEMA_VERSION,
            settings_schema_version: request.settings_schema_version,
            payloads: vec![
                BackupPayloadManifestV1 {
                    name: "settings.json".to_string(),
                    byte_size: settings_size,
                    sha256: settings_sha256,
                },
                BackupPayloadManifestV1 {
                    name: "history.sqlite".to_string(),
                    byte_size: database_size,
                    sha256: database_sha256,
                },
            ],
        };
        validate_manifest(&manifest, settings_size, database_size)?;
        let manifest_bytes = serde_json::to_vec(&manifest)
            .map_err(|_| "backup_manifest_serialize_failed".to_string())?;
        let manifest_size = u64::try_from(manifest_bytes.len())
            .map_err(|_| "backup_manifest_too_large".to_string())?;
        if manifest_size > MAX_MANIFEST_BYTES || manifest_size > u32::MAX as u64 {
            return Err("backup_manifest_too_large".to_string());
        }

        write_archive(
            &temporary_archive,
            &manifest_bytes,
            request.settings_bytes,
            &snapshot_path,
            database_size,
        )?;
        replace_file(&temporary_archive, request.destination)?;
        let byte_size = file_size(request.destination)?;
        Ok(summary_from_manifest(&manifest, byte_size))
    })();

    let _ = fs::remove_file(io_path(&snapshot_path));
    if result.is_err() {
        let _ = fs::remove_file(io_path(&temporary_archive));
    }
    result
}

pub fn extract_backup_archive(
    archive_path: &Path,
    destination_root: &Path,
) -> Result<ExtractedBackupArchiveV1, String> {
    if !archive_path.is_absolute() || !destination_root.is_absolute() {
        return Err("backup_path_must_be_absolute".to_string());
    }
    let archive_size = file_size(archive_path)?;
    let mut reader =
        BufReader::new(File::open(io_path(archive_path)).map_err(|error| error.to_string())?);
    let mut magic = [0_u8; 8];
    reader
        .read_exact(&mut magic)
        .map_err(|_| "backup_header_invalid".to_string())?;
    if &magic != BACKUP_MAGIC {
        return Err("backup_magic_invalid".to_string());
    }
    let manifest_size = u64::from(read_u32(&mut reader)?);
    let settings_size = read_u64(&mut reader)?;
    let database_size = read_u64(&mut reader)?;
    validate_declared_sizes(manifest_size, settings_size, database_size, archive_size)?;

    let mut manifest_bytes = vec![0_u8; manifest_size as usize];
    reader
        .read_exact(&mut manifest_bytes)
        .map_err(|_| "backup_manifest_truncated".to_string())?;
    let manifest = serde_json::from_slice::<BackupManifestV1>(&manifest_bytes)
        .map_err(|_| "backup_manifest_invalid".to_string())?;
    validate_manifest(&manifest, settings_size, database_size)?;

    let mut settings_bytes = vec![0_u8; settings_size as usize];
    reader
        .read_exact(&mut settings_bytes)
        .map_err(|_| "backup_payload_truncated".to_string())?;
    if sha256_bytes(&settings_bytes) != manifest.payloads[0].sha256 {
        return Err("backup_payload_hash_mismatch".to_string());
    }

    fs::create_dir_all(io_path(destination_root)).map_err(|error| error.to_string())?;
    let database_path = destination_root.join("history.sqlite");
    if database_path.exists() {
        return Err("backup_extract_destination_exists".to_string());
    }
    let database_temporary = destination_root.join(".history.sqlite.tmp");
    let extract_result = (|| {
        let mut output = BufWriter::new(
            File::create(io_path(&database_temporary)).map_err(|error| error.to_string())?,
        );
        let mut remaining = database_size;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0_u8; 1024 * 1024];
        while remaining > 0 {
            let wanted = usize::try_from(remaining.min(buffer.len() as u64))
                .map_err(|_| "backup_payload_too_large".to_string())?;
            let read = reader
                .read(&mut buffer[..wanted])
                .map_err(|error| error.to_string())?;
            if read == 0 {
                return Err("backup_payload_truncated".to_string());
            }
            output
                .write_all(&buffer[..read])
                .map_err(|error| error.to_string())?;
            hasher.update(&buffer[..read]);
            remaining -= read as u64;
        }
        output.flush().map_err(|error| error.to_string())?;
        output
            .get_ref()
            .sync_all()
            .map_err(|error| error.to_string())?;
        drop(output);
        if format!("{:x}", hasher.finalize()) != manifest.payloads[1].sha256 {
            return Err("backup_payload_hash_mismatch".to_string());
        }
        let mut trailing = [0_u8; 1];
        if reader
            .read(&mut trailing)
            .map_err(|error| error.to_string())?
            != 0
        {
            return Err("backup_trailing_data".to_string());
        }
        fs::rename(io_path(&database_temporary), io_path(&database_path))
            .map_err(|error| error.to_string())?;
        validate_extracted_database(&database_path)?;
        Ok(())
    })();
    if let Err(error) = extract_result {
        let _ = fs::remove_file(io_path(&database_temporary));
        let _ = fs::remove_file(io_path(&database_path));
        return Err(error);
    }

    let summary = summary_from_manifest(&manifest, archive_size);
    Ok(ExtractedBackupArchiveV1 {
        manifest,
        summary,
        settings_bytes,
        database_path,
    })
}

fn validate_create_request(request: &BackupArchiveCreateRequestV1<'_>) -> Result<(), String> {
    if !request.database_path.is_absolute() || !request.destination.is_absolute() {
        return Err("backup_path_must_be_absolute".to_string());
    }
    if !request.database_path.is_file() {
        return Err("backup_database_missing".to_string());
    }
    if request.settings_bytes.is_empty() || request.settings_bytes.len() as u64 > MAX_SETTINGS_BYTES
    {
        return Err("backup_settings_too_large".to_string());
    }
    if request.settings_schema_version <= 0 {
        return Err("backup_settings_schema_invalid".to_string());
    }
    if request.application_version.is_empty() || request.application_version.len() > 64 {
        return Err("backup_application_version_invalid".to_string());
    }
    DateTime::parse_from_rfc3339(request.created_at)
        .map_err(|_| "backup_created_at_invalid".to_string())?;
    if request.destination.exists()
        && fs::canonicalize(io_path(request.database_path)).ok()
            == fs::canonicalize(io_path(request.destination)).ok()
    {
        return Err("backup_destination_conflicts_with_database".to_string());
    }
    Ok(())
}

fn create_database_snapshot(source: &Path, destination: &Path) -> Result<(), String> {
    let connection = Connection::open(io_path(source)).map_err(|error| error.to_string())?;
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    if version != DATABASE_SCHEMA_VERSION || assert_schema_latest(&connection).is_err() {
        return Err("backup_database_schema_unsupported".to_string());
    }
    if destination.exists() {
        fs::remove_file(io_path(destination)).map_err(|error| error.to_string())?;
    }
    connection
        .execute("VACUUM INTO ?1", [destination.to_string_lossy().as_ref()])
        .map_err(|error| format!("backup_database_snapshot_failed:{error}"))?;
    validate_extracted_database(destination)
}

fn validate_extracted_database(path: &Path) -> Result<(), String> {
    let connection = Connection::open(io_path(path)).map_err(|error| error.to_string())?;
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    if version != DATABASE_SCHEMA_VERSION || assert_schema_latest(&connection).is_err() {
        return Err("backup_database_schema_unsupported".to_string());
    }
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    if integrity != "ok" {
        return Err("backup_database_integrity_failed".to_string());
    }
    Ok(())
}

fn write_archive(
    path: &Path,
    manifest: &[u8],
    settings: &[u8],
    database_path: &Path,
    database_size: u64,
) -> Result<(), String> {
    let mut writer =
        BufWriter::new(File::create(io_path(path)).map_err(|error| error.to_string())?);
    writer
        .write_all(BACKUP_MAGIC)
        .and_then(|_| writer.write_all(&(manifest.len() as u32).to_le_bytes()))
        .and_then(|_| writer.write_all(&(settings.len() as u64).to_le_bytes()))
        .and_then(|_| writer.write_all(&database_size.to_le_bytes()))
        .and_then(|_| writer.write_all(manifest))
        .and_then(|_| writer.write_all(settings))
        .map_err(|error| error.to_string())?;
    let mut database =
        BufReader::new(File::open(io_path(database_path)).map_err(|error| error.to_string())?);
    std::io::copy(&mut database, &mut writer).map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())?;
    writer
        .get_ref()
        .sync_all()
        .map_err(|error| error.to_string())
}

fn validate_declared_sizes(
    manifest_size: u64,
    settings_size: u64,
    database_size: u64,
    archive_size: u64,
) -> Result<(), String> {
    if manifest_size == 0 || manifest_size > MAX_MANIFEST_BYTES {
        return Err("backup_manifest_too_large".to_string());
    }
    if settings_size == 0 || settings_size > MAX_SETTINGS_BYTES {
        return Err("backup_settings_too_large".to_string());
    }
    if database_size == 0 || database_size > MAX_DATABASE_BYTES {
        return Err("backup_database_too_large".to_string());
    }
    let expected = HEADER_BYTES
        .checked_add(manifest_size)
        .and_then(|value| value.checked_add(settings_size))
        .and_then(|value| value.checked_add(database_size))
        .ok_or_else(|| "backup_size_overflow".to_string())?;
    if archive_size < expected {
        return Err("backup_payload_truncated".to_string());
    }
    if archive_size > expected {
        return Err("backup_trailing_data".to_string());
    }
    Ok(())
}

fn validate_manifest(
    manifest: &BackupManifestV1,
    settings_size: u64,
    database_size: u64,
) -> Result<(), String> {
    if manifest.schema_version != BACKUP_ARCHIVE_SCHEMA_VERSION
        || manifest.database_schema_version != DATABASE_SCHEMA_VERSION
        || manifest.settings_schema_version <= 0
        || manifest.application_version.is_empty()
        || manifest.application_version.len() > 64
        || DateTime::parse_from_rfc3339(&manifest.created_at).is_err()
        || manifest.payloads.len() != 2
    {
        return Err("backup_manifest_invalid".to_string());
    }
    let expected = [
        ("settings.json", settings_size),
        ("history.sqlite", database_size),
    ];
    for (payload, (name, byte_size)) in manifest.payloads.iter().zip(expected) {
        if payload.name != name || payload.byte_size != byte_size || !valid_sha256(&payload.sha256)
        {
            return Err("backup_manifest_invalid".to_string());
        }
    }
    Ok(())
}

fn summary_from_manifest(manifest: &BackupManifestV1, byte_size: u64) -> BackupArchiveSummaryV1 {
    BackupArchiveSummaryV1 {
        created_at: manifest.created_at.clone(),
        application_version: manifest.application_version.clone(),
        database_schema_version: manifest.database_schema_version,
        settings_schema_version: manifest.settings_schema_version,
        byte_size,
    }
}

fn file_size(path: &Path) -> Result<u64, String> {
    fs::metadata(io_path(path))
        .map(|metadata| metadata.len())
        .map_err(|error| error.to_string())
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut reader = BufReader::new(File::open(io_path(path)).map_err(|error| error.to_string())?);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn read_u32(reader: &mut impl Read) -> Result<u32, String> {
    let mut bytes = [0_u8; 4];
    reader
        .read_exact(&mut bytes)
        .map_err(|_| "backup_header_invalid".to_string())?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_u64(reader: &mut impl Read) -> Result<u64, String> {
    let mut bytes = [0_u8; 8];
    reader
        .read_exact(&mut bytes)
        .map_err(|_| "backup_header_invalid".to_string())?;
    Ok(u64::from_le_bytes(bytes))
}

fn temporary_sibling(path: &Path, suffix: &str) -> Result<PathBuf, String> {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "backup_destination_name_invalid".to_string())?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    Ok(path.with_file_name(format!(".{name}.{}.{}.{suffix}", std::process::id(), nonce)))
}

#[cfg(target_os = "windows")]
fn replace_file(source: &Path, destination: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let source: Vec<u16> = io_path(source)
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let destination: Vec<u16> = io_path(destination)
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn replace_file(source: &Path, destination: &Path) -> Result<(), String> {
    fs::rename(source, destination).map_err(|error| error.to_string())
}

#[cfg(target_os = "windows")]
fn io_path(path: &Path) -> PathBuf {
    let value = path.to_string_lossy().replace('/', "\\");
    if value.starts_with(r"\\?\") {
        return PathBuf::from(value);
    }
    if let Some(rest) = value.strip_prefix(r"\\") {
        return PathBuf::from(format!(r"\\?\UNC\{rest}"));
    }
    if path.is_absolute() {
        return PathBuf::from(format!(r"\\?\{value}"));
    }
    path.to_path_buf()
}

#[cfg(not(target_os = "windows"))]
fn io_path(path: &Path) -> PathBuf {
    path.to_path_buf()
}
