use std::path::{Path, PathBuf};

use chrono::{SecondsFormat, Utc};
use rusqlite::{params, Connection, TransactionBehavior};

use super::{DownloadError, FileObservation, RootAccess};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RecoverySummary {
    pub jobs_interrupted: usize,
    pub items_interrupted: usize,
    pub assets_interrupted: usize,
    pub finals_completed: usize,
    pub finals_failed: usize,
    pub partials_retained: usize,
    pub partials_deleted: usize,
    pub assets_reset: usize,
}

struct AssetRecord {
    id: i64,
    root: PathBuf,
    work_directory: PathBuf,
    final_path: PathBuf,
    part_path: PathBuf,
    expected_size: Option<u64>,
}

enum AssetRecovery {
    Completed { id: i64, len: u64 },
    Failed { id: i64 },
    Retained { id: i64, len: u64 },
    Reset { id: i64, deleted: bool },
}

pub fn recover_download_jobs(
    connection: &mut Connection,
    roots: &dyn RootAccess,
) -> Result<RecoverySummary, DownloadError> {
    let mut summary = RecoverySummary::default();
    let first = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    summary.jobs_interrupted = first
        .execute(
            "UPDATE download_jobs SET state='interrupted' WHERE state='running'",
            [],
        )
        .map_err(storage_error)?;
    summary.items_interrupted = first
        .execute(
            "UPDATE download_items SET state='interrupted', updated_at=?1
         WHERE state IN ('resolving', 'downloading')",
            [now()],
        )
        .map_err(storage_error)?;
    summary.assets_interrupted = first.execute(
        "UPDATE download_assets SET state='interrupted', updated_at=?1 WHERE state='downloading'",
        [now()],
    ).map_err(storage_error)?;
    first.commit().map_err(storage_error)?;

    let assets = load_recoverable_assets(connection)?;
    let mut observations = Vec::with_capacity(assets.len());
    for asset in assets {
        let final_path = rooted_asset_path(&asset.work_directory, &asset.final_path);
        let final_guard = roots.guard(&asset.root, &final_path)?;
        match final_guard.inspect() {
            Ok(FileObservation::File { len }) if final_is_valid(len, asset.expected_size) => {
                observations.push(AssetRecovery::Completed { id: asset.id, len });
                continue;
            }
            Ok(FileObservation::File { .. } | FileObservation::NotFile) | Err(_) => {
                observations.push(AssetRecovery::Failed { id: asset.id });
                continue;
            }
            Ok(FileObservation::Missing) => {}
        }
        drop(final_guard);

        let part_path = rooted_asset_path(&asset.work_directory, &asset.part_path);
        let owned = partial_is_owned(asset.id, &part_path);
        let part_guard = roots.guard(&asset.root, &part_path)?;
        match part_guard.inspect() {
            Ok(FileObservation::File { len }) if owned && len > 0 => {
                observations.push(AssetRecovery::Retained { id: asset.id, len });
            }
            Ok(FileObservation::Missing) => {
                observations.push(AssetRecovery::Reset {
                    id: asset.id,
                    deleted: false,
                });
            }
            Ok(_) if owned => {
                let deleted = part_guard.remove().is_ok();
                observations.push(AssetRecovery::Reset {
                    id: asset.id,
                    deleted,
                });
            }
            Ok(_) => {
                let deleted = part_guard.remove().is_ok();
                observations.push(AssetRecovery::Reset {
                    id: asset.id,
                    deleted,
                });
            }
            Err(_) => observations.push(AssetRecovery::Reset {
                id: asset.id,
                deleted: false,
            }),
        }
    }

    let second = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    let timestamp = now();
    for observation in observations {
        match observation {
            AssetRecovery::Completed { id, len } => {
                second
                    .execute(
                        "UPDATE download_assets SET state='completed', downloaded_bytes=?1,
                     validated_size=?1, validated_at=?2, completed_at=?2, updated_at=?2,
                     last_error_code=NULL WHERE id=?3",
                        params![len as i64, timestamp, id],
                    )
                    .map_err(storage_error)?;
                summary.finals_completed += 1;
            }
            AssetRecovery::Failed { id } => {
                second.execute(
                    "UPDATE download_assets SET state='failed', last_error_code='download_asset_validation_failed',
                     updated_at=?1 WHERE id=?2",
                    params![timestamp, id],
                ).map_err(storage_error)?;
                summary.finals_failed += 1;
            }
            AssetRecovery::Retained { id, len } => {
                second
                    .execute(
                        "UPDATE download_assets SET downloaded_bytes=?1, updated_at=?2 WHERE id=?3",
                        params![len as i64, timestamp, id],
                    )
                    .map_err(storage_error)?;
                summary.partials_retained += 1;
            }
            AssetRecovery::Reset { id, deleted } => {
                second
                    .execute(
                        "UPDATE download_assets SET state='queued', downloaded_bytes=0, etag=NULL,
                     last_modified=NULL, last_error_code=NULL, updated_at=?1 WHERE id=?2",
                        params![timestamp, id],
                    )
                    .map_err(storage_error)?;
                summary.assets_reset += 1;
                summary.partials_deleted += usize::from(deleted);
            }
        }
    }
    second.commit().map_err(storage_error)?;
    Ok(summary)
}

fn load_recoverable_assets(connection: &Connection) -> Result<Vec<AssetRecord>, DownloadError> {
    let mut statement = connection
        .prepare(
            "SELECT a.id, j.destination_root, i.relative_work_directory,
                a.relative_final_path, a.relative_part_path, a.expected_size
         FROM download_assets a
         JOIN download_items i ON i.id=a.item_id
         JOIN download_jobs j ON j.id=i.job_id
         WHERE a.state NOT IN ('completed', 'skipped', 'failed', 'superseded', 'cancelled')
         ORDER BY a.id",
        )
        .map_err(storage_error)?;
    let rows = statement
        .query_map([], |row| {
            Ok(AssetRecord {
                id: row.get(0)?,
                root: PathBuf::from(row.get::<_, String>(1)?),
                work_directory: PathBuf::from(row.get::<_, String>(2)?),
                final_path: PathBuf::from(row.get::<_, String>(3)?),
                part_path: PathBuf::from(row.get::<_, String>(4)?),
                expected_size: row.get::<_, Option<i64>>(5)?.map(|value| value as u64),
            })
        })
        .map_err(storage_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)
}

fn rooted_asset_path(work_directory: &Path, asset_path: &Path) -> PathBuf {
    if asset_path.starts_with(work_directory) {
        asset_path.to_path_buf()
    } else {
        work_directory.join(asset_path)
    }
}

fn final_is_valid(len: u64, expected_size: Option<u64>) -> bool {
    len > 0
        && expected_size
            .map(|expected| expected == len)
            .unwrap_or(true)
}

fn partial_is_owned(asset_id: i64, path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let Some(rest) = name.strip_prefix(&format!(".asset-{asset_id}-")) else {
        return false;
    };
    let Some((nonce, extension)) = rest.split_once('.') else {
        return false;
    };
    !nonce.is_empty()
        && nonce.chars().all(|character| character.is_ascii_digit())
        && extension.ends_with(".part")
        && extension.len() > ".part".len()
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn storage_error(_error: rusqlite::Error) -> DownloadError {
    DownloadError {
        code: "download_storage_failed".to_string(),
        message: "恢复下载任务失败".to_string(),
    }
}
