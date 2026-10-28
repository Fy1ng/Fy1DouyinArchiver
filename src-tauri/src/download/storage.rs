use std::{io::Read, path::PathBuf};

use chrono::{SecondsFormat, Utc};
use rusqlite::{params, Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    build_work_directory, expand_candidates, partial_filename, DownloadError, DownloadQuality,
    DownloadSelection, FileObservation, FilesystemRootAccess, ResolvedAsset, ResolvedMediaManifest,
    RootAccess,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetAction {
    Inserted { asset_id: i64, stable_key: String },
    Retained { asset_id: i64, stable_key: String },
    Superseded { asset_id: i64, stable_key: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateDownloadJobRequest {
    pub selection: DownloadSelection,
    pub destination_root: String,
    pub requested_quality: DownloadQuality,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadJobOwner {
    pub account_id: i64,
    pub profile_key: String,
}

pub fn create_download_job(
    connection: &mut Connection,
    owner: &DownloadJobOwner,
    request: &CreateDownloadJobRequest,
) -> Result<i64, DownloadError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    validate_owner(&transaction, owner)?;
    let candidates = expand_candidates(&transaction, owner.account_id, &request.selection)?;
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let (selection_scope, filter_summary) = selection_metadata(&request.selection);
    let requested_quality = quality_name(request.requested_quality);

    transaction
        .execute(
            r#"
            INSERT INTO download_jobs (
              account_id, profile_key, created_at, selection_scope, filter_summary, destination_root,
              requested_quality, state, candidate_count
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'queued', ?8)
            "#,
            params![
                owner.account_id,
                owner.profile_key,
                now,
                selection_scope,
                filter_summary,
                request.destination_root,
                requested_quality,
                candidates.len() as i64
            ],
        )
        .map_err(storage_error)?;
    let job_id = transaction.last_insert_rowid();

    for candidate in candidates {
        let relative_work_directory = build_work_directory(&candidate);
        transaction
            .execute(
                r#"
                INSERT INTO download_items (
                  job_id, aweme_id, canonical_url, content_type, title, author_name,
                  published_at, filter_context, relative_work_directory, state,
                  created_at, updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'queued', ?10, ?10)
                "#,
                params![
                    job_id,
                    candidate.aweme_id,
                    candidate.canonical_url,
                    candidate.content_type,
                    candidate.title,
                    candidate.author_name,
                    candidate.published_at,
                    candidate.filter_context,
                    relative_work_directory.to_string_lossy(),
                    now
                ],
            )
            .map_err(storage_error)?;
    }

    transaction.commit().map_err(storage_error)?;
    Ok(job_id)
}

fn validate_owner(
    connection: &Connection,
    owner: &DownloadJobOwner,
) -> Result<(), DownloadError> {
    if owner.account_id <= 0
        || owner.profile_key.len() != 32
        || !owner
            .profile_key
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(DownloadError {
            code: "download_account_binding_invalid".to_string(),
            message: "下载任务的账号绑定无效".to_string(),
        });
    }
    let matches: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM accounts WHERE id = ?1 AND profile_key = ?2",
            params![owner.account_id, owner.profile_key],
            |row| row.get(0),
        )
        .map_err(storage_error)?;
    if matches != 1 {
        return Err(DownloadError {
            code: "download_account_binding_stale".to_string(),
            message: "下载任务的账号登录目录已失效".to_string(),
        });
    }
    Ok(())
}

pub fn reconcile_manifest(
    connection: &mut Connection,
    item_id: i64,
    manifest: &ResolvedMediaManifest,
) -> Result<Vec<AssetAction>, DownloadError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    let (aweme_id, generation, requested_profile, destination_root, work_directory): (
        String,
        i64,
        String,
        PathBuf,
        PathBuf,
    ) = transaction
        .query_row(
            "SELECT i.aweme_id, i.manifest_generation, j.requested_quality, j.destination_root, i.relative_work_directory FROM download_items i JOIN download_jobs j ON j.id=i.job_id WHERE i.id=?1",
            [item_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    PathBuf::from(row.get::<_, String>(3)?),
                    PathBuf::from(row.get::<_, String>(4)?),
                ))
            },
        )
        .map_err(storage_error)?;
    if aweme_id != manifest.aweme_id {
        return Err(DownloadError {
            code: "download_manifest_mismatch".into(),
            message: "媒体清单与下载项目不匹配".into(),
        });
    }
    let next_generation = generation + 1;
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let mut actions = Vec::new();

    let previous = load_previous_assets(&transaction, item_id, generation)?;
    let historical = load_historical_assets(
        &transaction,
        item_id,
        &aweme_id,
        &destination_root,
        &work_directory,
    )?;
    for old in &previous {
        if !manifest
            .assets
            .iter()
            .any(|asset| asset.stable_key == old.stable_key)
        {
            transaction.execute(
                "UPDATE download_assets SET required=0, state=CASE WHEN state IN ('completed','skipped') THEN state ELSE 'superseded' END, updated_at=?2 WHERE id=?1",
                params![old.id, now],
            ).map_err(storage_error)?;
            actions.push(AssetAction::Superseded {
                asset_id: old.id,
                stable_key: old.stable_key.clone(),
            });
        } else {
            transaction
                .execute(
                    "UPDATE download_assets SET required=0, updated_at=?2 WHERE id=?1",
                    params![old.id, now],
                )
                .map_err(storage_error)?;
        }
    }

    let mut completed = 0i64;
    let mut skipped = 0i64;
    for asset in &manifest.assets {
        let same_item_completed = previous.iter().find(|old| {
            old.stable_key == asset.stable_key
                && old.state == "completed"
                && safe_to_retain(old, asset, &requested_profile)
        });
        let paths = asset_paths(asset);
        let historical_completed = historical.iter().find(|old| {
            old.stable_key == asset.stable_key
                && old.relative_final_path == PathBuf::from(&paths.0)
                && safe_to_retain(old, asset, &requested_profile)
                && validate_completed_file(old, &destination_root, &work_directory)
        });
        let partial = previous.iter().find(|old| {
            old.stable_key == asset.stable_key
                && safe_partial_to_resume(old, asset, &requested_profile)
                && validate_partial_file(old, &destination_root, &work_directory)
        });

        if let Some(partial) = partial {
            transaction.execute(
                "UPDATE download_assets SET asset_type=?2, ordinal=?3, requested_profile=?4, actual_quality=?5, width=?6, height=?7, bitrate=?8, codec=?9, mime_type=?10, relative_final_path=?11, expected_size=?12, manifest_generation=?13, required=1, state='queued', retry_count=0, last_error_code=NULL, updated_at=?14, completed_at=NULL WHERE id=?1",
                params![partial.id, asset.asset_type, asset.ordinal, requested_profile, asset.actual_quality, asset.width, asset.height, asset.bitrate, asset.codec, asset.mime_type, paths.0, asset.expected_size, next_generation, now],
            ).map_err(storage_error)?;
            actions.push(AssetAction::Retained {
                asset_id: partial.id,
                stable_key: asset.stable_key.clone(),
            });
            continue;
        }

        let retained = same_item_completed.or(historical_completed);
        let retained_state = if historical_completed.is_some() {
            skipped += 1;
            "skipped"
        } else if same_item_completed.is_some() {
            completed += 1;
            "completed"
        } else {
            "queued"
        };
        transaction.execute(
            "INSERT INTO download_assets (item_id, stable_key, asset_type, ordinal, requested_profile, actual_quality, width, height, bitrate, codec, mime_type, relative_final_path, relative_part_path, expected_size, downloaded_bytes, validated_size, validated_sha256, validated_at, manifest_generation, required, state, created_at, updated_at, completed_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,'pending.part',?13,?14,?15,?16,?17,?18,1,?19,?20,?20,?21)",
            params![item_id, asset.stable_key, asset.asset_type, asset.ordinal, requested_profile, asset.actual_quality, asset.width, asset.height, asset.bitrate, asset.codec, asset.mime_type, paths.0, asset.expected_size, retained.map_or(0, |old| old.downloaded_bytes), retained.and_then(|old| old.validated_size), retained.and_then(|old| old.validated_sha256.as_deref()), retained.and_then(|old| old.validated_at.as_deref()), next_generation, retained_state, now, retained.and_then(|old| old.completed_at.as_deref()).or_else(|| retained.is_some().then_some(now.as_str()))],
        ).map_err(storage_error)?;
        let asset_id = transaction.last_insert_rowid();
        let part = partial_filename(asset_id, next_generation as u64, &paths.1);
        transaction
            .execute(
                "UPDATE download_assets SET relative_part_path=?2 WHERE id=?1",
                params![asset_id, part],
            )
            .map_err(storage_error)?;
        actions.push(if retained.is_some() {
            AssetAction::Retained {
                asset_id,
                stable_key: asset.stable_key.clone(),
            }
        } else {
            AssetAction::Inserted {
                asset_id,
                stable_key: asset.stable_key.clone(),
            }
        });
    }

    let terminal = completed + skipped;
    let item_state = if terminal == manifest.assets.len() as i64 {
        if skipped == terminal {
            "skipped"
        } else {
            "completed"
        }
    } else {
        "downloading"
    };
    transaction.execute(
        "UPDATE download_items SET content_type=?2, manifest_generation=?3, total_required_asset_count=?4, completed_asset_count=?5, state=?6, updated_at=?7, completed_at=CASE WHEN ?6 IN ('completed','skipped') THEN ?7 ELSE NULL END WHERE id=?1",
        params![item_id, manifest.content_type, next_generation, manifest.assets.len() as i64, terminal, item_state, now],
    ).map_err(storage_error)?;
    transaction.commit().map_err(storage_error)?;
    Ok(actions)
}

struct PreviousAsset {
    id: i64,
    stable_key: String,
    state: String,
    asset_type: String,
    ordinal: u32,
    requested_profile: String,
    actual_quality: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    bitrate: Option<u64>,
    codec: Option<String>,
    expected_size: Option<u64>,
    mime_type: Option<String>,
    relative_final_path: PathBuf,
    relative_part_path: PathBuf,
    downloaded_bytes: i64,
    etag: Option<String>,
    last_modified: Option<String>,
    validated_size: Option<i64>,
    validated_sha256: Option<String>,
    validated_at: Option<String>,
    completed_at: Option<String>,
}

fn load_previous_assets(
    transaction: &rusqlite::Transaction<'_>,
    item_id: i64,
    generation: i64,
) -> Result<Vec<PreviousAsset>, DownloadError> {
    let mut statement = transaction.prepare("SELECT id, stable_key, state, asset_type, ordinal, requested_profile, actual_quality, width, height, bitrate, codec, expected_size, mime_type, relative_final_path, relative_part_path, downloaded_bytes, etag, last_modified, validated_size, validated_sha256, validated_at, completed_at FROM download_assets WHERE item_id=?1 AND manifest_generation=?2 AND required=1").map_err(storage_error)?;
    let rows = statement
        .query_map(params![item_id, generation], |row| {
            Ok(PreviousAsset {
                id: row.get(0)?,
                stable_key: row.get(1)?,
                state: row.get(2)?,
                asset_type: row.get(3)?,
                ordinal: row.get(4)?,
                requested_profile: row.get(5)?,
                actual_quality: row.get(6)?,
                width: row.get(7)?,
                height: row.get(8)?,
                bitrate: row.get(9)?,
                codec: row.get(10)?,
                expected_size: row.get(11)?,
                mime_type: row.get(12)?,
                relative_final_path: PathBuf::from(row.get::<_, String>(13)?),
                relative_part_path: PathBuf::from(row.get::<_, String>(14)?),
                downloaded_bytes: row.get(15)?,
                etag: row.get(16)?,
                last_modified: row.get(17)?,
                validated_size: row.get(18)?,
                validated_sha256: row.get(19)?,
                validated_at: row.get(20)?,
                completed_at: row.get(21)?,
            })
        })
        .map_err(storage_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage_error)?;
    Ok(rows)
}

fn load_historical_assets(
    transaction: &rusqlite::Transaction<'_>,
    item_id: i64,
    aweme_id: &str,
    destination_root: &PathBuf,
    work_directory: &PathBuf,
) -> Result<Vec<PreviousAsset>, DownloadError> {
    let mut statement = transaction.prepare(
        "SELECT a.id, a.stable_key, a.state, a.asset_type, a.ordinal, a.requested_profile, a.actual_quality, a.width, a.height, a.bitrate, a.codec, a.expected_size, a.mime_type, a.relative_final_path, a.relative_part_path, a.downloaded_bytes, a.etag, a.last_modified, a.validated_size, a.validated_sha256, a.validated_at, a.completed_at FROM download_assets a JOIN download_items i ON i.id=a.item_id JOIN download_jobs j ON j.id=i.job_id WHERE i.id<>?1 AND i.aweme_id=?2 AND j.destination_root=?3 AND i.relative_work_directory=?4 AND a.required=1 AND a.state IN ('completed','skipped') ORDER BY a.completed_at DESC, a.id DESC",
    ).map_err(storage_error)?;
    let rows = statement
        .query_map(
            params![
                item_id,
                aweme_id,
                destination_root.to_string_lossy(),
                work_directory.to_string_lossy()
            ],
            |row| {
                Ok(PreviousAsset {
                    id: row.get(0)?,
                    stable_key: row.get(1)?,
                    state: row.get(2)?,
                    asset_type: row.get(3)?,
                    ordinal: row.get(4)?,
                    requested_profile: row.get(5)?,
                    actual_quality: row.get(6)?,
                    width: row.get(7)?,
                    height: row.get(8)?,
                    bitrate: row.get(9)?,
                    codec: row.get(10)?,
                    expected_size: row.get(11)?,
                    mime_type: row.get(12)?,
                    relative_final_path: PathBuf::from(row.get::<_, String>(13)?),
                    relative_part_path: PathBuf::from(row.get::<_, String>(14)?),
                    downloaded_bytes: row.get(15)?,
                    etag: row.get(16)?,
                    last_modified: row.get(17)?,
                    validated_size: row.get(18)?,
                    validated_sha256: row.get(19)?,
                    validated_at: row.get(20)?,
                    completed_at: row.get(21)?,
                })
            },
        )
        .map_err(storage_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage_error)?;
    Ok(rows)
}

fn safe_to_retain(
    previous: &PreviousAsset,
    asset: &ResolvedAsset,
    requested_profile: &str,
) -> bool {
    let Some(validated_size) = previous.validated_size else {
        return false;
    };
    previous.asset_type == asset.asset_type
        && previous.ordinal == asset.ordinal
        && previous.requested_profile == requested_profile
        && previous.actual_quality == asset.actual_quality
        && previous.width == asset.width
        && previous.height == asset.height
        && previous.bitrate == asset.bitrate
        && previous.codec == asset.codec
        && previous.expected_size == asset.expected_size
        && previous.mime_type == asset.mime_type
        && previous.downloaded_bytes == validated_size
        && asset
            .expected_size
            .is_none_or(|expected_size| expected_size == validated_size as u64)
        && previous
            .validated_sha256
            .as_deref()
            .is_some_and(|sha256| !sha256.is_empty())
        && previous.validated_at.is_some()
        && previous.completed_at.is_some()
}

fn safe_partial_to_resume(
    previous: &PreviousAsset,
    asset: &ResolvedAsset,
    requested_profile: &str,
) -> bool {
    matches!(
        previous.state.as_str(),
        "queued" | "paused" | "interrupted" | "downloading"
    ) && previous.downloaded_bytes > 0
        && (previous.etag.is_some() || previous.last_modified.is_some())
        && previous.asset_type == asset.asset_type
        && previous.ordinal == asset.ordinal
        && previous.requested_profile == requested_profile
        && previous.actual_quality == asset.actual_quality
        && previous.width == asset.width
        && previous.height == asset.height
        && previous.bitrate == asset.bitrate
        && previous.codec == asset.codec
        && previous.expected_size == asset.expected_size
        && previous.mime_type == asset.mime_type
        && asset
            .expected_size
            .is_none_or(|expected| previous.downloaded_bytes < expected as i64)
}

fn validate_completed_file(
    previous: &PreviousAsset,
    destination_root: &PathBuf,
    work_directory: &PathBuf,
) -> bool {
    let (Some(validated_size), Some(expected_digest)) = (
        previous.validated_size,
        previous.validated_sha256.as_deref(),
    ) else {
        return false;
    };
    if validated_size < 0 || expected_digest.len() != 64 {
        return false;
    }
    let relative = work_directory.join(&previous.relative_final_path);
    let Ok(guard) = FilesystemRootAccess.guard(destination_root, &relative) else {
        return false;
    };
    if guard.inspect().ok()
        != Some(FileObservation::File {
            len: validated_size as u64,
        })
    {
        return false;
    }
    let Ok(mut file) = guard.open_existing_for_resume() else {
        return false;
    };
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let Ok(read) = file.read(&mut buffer) else {
            return false;
        };
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    format!("{:x}", digest.finalize()).eq_ignore_ascii_case(expected_digest)
}

fn validate_partial_file(
    previous: &PreviousAsset,
    destination_root: &PathBuf,
    work_directory: &PathBuf,
) -> bool {
    let relative = work_directory.join(&previous.relative_part_path);
    let Ok(guard) = FilesystemRootAccess.guard(destination_root, &relative) else {
        return false;
    };
    guard.inspect().ok()
        == Some(FileObservation::File {
            len: previous.downloaded_bytes as u64,
        })
}

fn asset_paths(asset: &ResolvedAsset) -> (String, String) {
    let extension = match asset.mime_type.as_deref() {
        Some("video/mp4") => "mp4",
        Some("image/jpeg") => "jpg",
        Some("image/png") => "png",
        Some("audio/mpeg") => "mp3",
        _ => "bin",
    };
    let path = match asset.asset_type.as_str() {
        "video" => format!("video.{extension}"),
        "image" => format!("images/{:03}.{extension}", asset.ordinal),
        "live-image" | "live-video" => format!("live/{:03}.{extension}", asset.ordinal),
        "audio" => format!("audio/music.{extension}"),
        _ => format!("assets/{:03}.{extension}", asset.ordinal),
    };
    (path, extension.to_string())
}

fn selection_metadata(selection: &DownloadSelection) -> (&'static str, Option<String>) {
    match selection {
        DownloadSelection::Checked { .. } => ("selected", None),
        DownloadSelection::Filtered { query } => (
            "filtered",
            query
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
        ),
        DownloadSelection::All => ("all", None),
    }
}

fn quality_name(quality: DownloadQuality) -> &'static str {
    match quality {
        DownloadQuality::Highest => "highest",
        DownloadQuality::Balanced => "balanced",
        DownloadQuality::SpaceSaver => "space_saver",
    }
}

fn storage_error(_error: rusqlite::Error) -> DownloadError {
    DownloadError {
        code: "download_storage_failed".to_string(),
        message: "保存下载任务失败".to_string(),
    }
}
