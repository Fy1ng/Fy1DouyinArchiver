use crate::{
    collector::file_store::write_bytes_atomically,
    runtime_paths::RuntimePaths,
    schema::assert_schema_latest,
    settings_store::{load_portable_settings, CoverCacheCleanupModeV1},
};
use chrono::{DateTime, Duration};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::ErrorKind,
    path::{Component, Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::Duration as StdDuration,
};

const CACHE_WORKER_IDLE_WAIT: StdDuration = StdDuration::from_secs(30);
const CACHE_WORKER_MAX_DOWNLOAD_BYTES: usize = 32 * 1024 * 1024;

type CoverDownloader = dyn Fn(&str) -> Result<DownloadedCoverV1, String> + Send + Sync + 'static;

pub struct CoverCacheWorkerConfigV1 {
    runtime_paths: RuntimePaths,
    downloader: Arc<CoverDownloader>,
    idle_wait: StdDuration,
    max_download_bytes: usize,
}

impl CoverCacheWorkerConfigV1 {
    pub fn new(runtime_paths: RuntimePaths) -> Self {
        Self {
            runtime_paths,
            downloader: Arc::new(|url| download_cover(url, CACHE_WORKER_MAX_DOWNLOAD_BYTES)),
            idle_wait: CACHE_WORKER_IDLE_WAIT,
            max_download_bytes: CACHE_WORKER_MAX_DOWNLOAD_BYTES,
        }
    }

    pub fn with_downloader(mut self, downloader: Arc<CoverDownloader>) -> Self {
        self.downloader = downloader;
        self
    }

    pub fn with_idle_wait(mut self, idle_wait: StdDuration) -> Self {
        self.idle_wait = idle_wait.max(StdDuration::from_millis(1));
        self
    }
}

pub struct CoverCacheWorkerHandle {
    signal: Arc<CoverCacheWorkerSignal>,
    join: Mutex<Option<JoinHandle<Result<(), String>>>>,
}

struct CoverCacheWorkerSignal {
    state: Mutex<CoverCacheWorkerSignalState>,
    changed: Condvar,
}

#[derive(Default)]
struct CoverCacheWorkerSignalState {
    wake_generation: u64,
    stop: bool,
}

impl CoverCacheWorkerHandle {
    pub fn start(config: CoverCacheWorkerConfigV1) -> Result<Self, String> {
        let signal = Arc::new(CoverCacheWorkerSignal {
            state: Mutex::new(CoverCacheWorkerSignalState::default()),
            changed: Condvar::new(),
        });
        let worker_signal = Arc::clone(&signal);
        let join = thread::Builder::new()
            .name("emily-cover-cache-worker".to_string())
            .spawn(move || cover_cache_worker_loop(config, worker_signal))
            .map_err(|error| error.to_string())?;
        Ok(Self {
            signal,
            join: Mutex::new(Some(join)),
        })
    }

    pub fn wake(&self) {
        let Ok(mut state) = self.signal.state.lock() else {
            return;
        };
        state.wake_generation = state.wake_generation.wrapping_add(1);
        self.signal.changed.notify_all();
    }

    pub fn stop(&self) {
        let Ok(mut state) = self.signal.state.lock() else {
            return;
        };
        state.stop = true;
        self.signal.changed.notify_all();
    }

    pub fn join(&self) -> Result<(), String> {
        let join = self
            .join
            .lock()
            .map_err(|_| "cover_cache_worker_join_poisoned".to_string())?
            .take();
        match join {
            Some(join) => join
                .join()
                .map_err(|_| "cover_cache_worker_panicked".to_string())?,
            None => Ok(()),
        }
    }
}

impl Drop for CoverCacheWorkerHandle {
    fn drop(&mut self) {
        self.stop();
        let _ = self.join();
    }
}

fn cover_cache_worker_loop(
    config: CoverCacheWorkerConfigV1,
    signal: Arc<CoverCacheWorkerSignal>,
) -> Result<(), String> {
    let mut observed_generation = 0_u64;
    loop {
        let mut state = signal
            .state
            .lock()
            .map_err(|_| "cover_cache_worker_signal_poisoned".to_string())?;
        while !state.stop && state.wake_generation == observed_generation {
            let (next_state, _) = signal
                .changed
                .wait_timeout(state, config.idle_wait)
                .map_err(|_| "cover_cache_worker_signal_poisoned".to_string())?;
            state = next_state;
        }
        if state.stop {
            return Ok(());
        }
        observed_generation = state.wake_generation;
        drop(state);

        loop {
            if signal
                .state
                .lock()
                .map_err(|_| "cover_cache_worker_signal_poisoned".to_string())?
                .stop
            {
                return Ok(());
            }
            match process_background_cache_step(&config) {
                Ok(CacheTaskStepV1::Progressed) => continue,
                Ok(CacheTaskStepV1::Paused) => break,
                Ok(CacheTaskStepV1::Finished) => {
                    if let Err(error) = run_background_cache_cleanup(&config) {
                        eprintln!("cover_cache_worker_cleanup_failed:{error}");
                    }
                    break;
                }
                Err(error) if error == "data_root_migration_in_progress" => break,
                Err(error) => {
                    eprintln!("cover_cache_worker_step_failed:{error}");
                    break;
                }
            }
        }
    }
}

fn run_background_cache_cleanup(config: &CoverCacheWorkerConfigV1) -> Result<(), String> {
    let paths = config.runtime_paths.acquire()?;
    let settings = load_portable_settings(&paths.data_root)?.global.cover_cache;
    if !settings.enabled
        || (!settings.cleanup_by_capacity_enabled && !settings.cleanup_by_age_enabled)
    {
        return Ok(());
    }
    let mode = match settings.cleanup_list_mode {
        CoverCacheCleanupModeV1::KeepListed => CacheCleanupModeV1::KeepListed,
        CoverCacheCleanupModeV1::CleanListed => CacheCleanupModeV1::CleanListed,
    };
    let policy = CacheCleanupPolicyV1 {
        max_bytes: settings
            .cleanup_by_capacity_enabled
            .then(|| settings.capacity_limit_mib.saturating_mul(1024 * 1024)),
        max_age_days: settings
            .cleanup_by_age_enabled
            .then_some(settings.max_age_days),
    };
    let mut connection = Connection::open(&paths.database).map_err(|error| error.to_string())?;
    connection
        .execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")
        .map_err(|error| error.to_string())?;
    assert_schema_latest(&connection)?;
    execute_configured_cache_cleanup(
        &mut connection,
        &paths.covers,
        mode,
        &policy,
        &UtcNow::now_iso(),
    )?;
    Ok(())
}

fn process_background_cache_step(
    config: &CoverCacheWorkerConfigV1,
) -> Result<CacheTaskStepV1, String> {
    let paths = config.runtime_paths.acquire()?;
    let mut connection = Connection::open(&paths.database).map_err(|error| error.to_string())?;
    connection
        .execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")
        .map_err(|error| error.to_string())?;
    assert_schema_latest(&connection)?;
    let Some(task) = latest_cache_task(&connection)? else {
        return Ok(CacheTaskStepV1::Finished);
    };
    if task.status != CacheTaskStatusV1::Running {
        return Ok(if task.status == CacheTaskStatusV1::Paused {
            CacheTaskStepV1::Paused
        } else {
            CacheTaskStepV1::Finished
        });
    }
    let downloader = Arc::clone(&config.downloader);
    process_cache_task_step_with(
        &mut connection,
        &paths.covers,
        task.id,
        config.max_download_bytes,
        &UtcNow::now_iso(),
        move |url| downloader(url),
    )
}

struct UtcNow;

impl UtcNow {
    fn now_iso() -> String {
        chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CacheCleanupModeV1 {
    KeepListed,
    CleanListed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheCleanupRuleV1 {
    pub mode: CacheCleanupModeV1,
    pub video_ids: BTreeSet<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CacheCleanupPolicyV1 {
    pub max_bytes: Option<u64>,
    pub max_age_days: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheEntryCandidateV1 {
    pub video_id: i64,
    pub byte_size: u64,
    pub last_accessed_at: String,
}

impl CacheEntryCandidateV1 {
    pub fn new(video_id: i64, byte_size: u64, last_accessed_at: &str) -> Self {
        Self {
            video_id,
            byte_size,
            last_accessed_at: last_accessed_at.to_string(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CacheCleanupPlanV1 {
    pub video_ids: Vec<i64>,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CoverCacheEntryV1 {
    pub video_id: i64,
    pub remote_url: String,
    pub relative_path: String,
    pub byte_size: u64,
    pub mime_type: String,
    pub sha256: String,
    pub downloaded_at: String,
    pub last_accessed_at: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CoverCandidateV1 {
    pub video_id: i64,
    pub aweme_id: String,
    pub remote_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadedCoverV1 {
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedCoverAssetV1 {
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheTaskStepV1 {
    Progressed,
    Paused,
    Finished,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CacheInspectionV1 {
    pub healthy_files: u64,
    pub missing_files: u64,
    pub orphan_files: u64,
    pub invalid_indexes: u64,
    pub indexed_bytes: u64,
    pub orphan_bytes: u64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CacheCleanupResultV1 {
    pub deleted_files: u64,
    pub deleted_indexes: u64,
    pub released_bytes: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CacheRuleItemV1 {
    pub video_id: i64,
    pub aweme_id: String,
    pub title: Option<String>,
    pub author_name: Option<String>,
    pub cover_url: Option<String>,
    pub cached: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CacheCleanupPreviewV1 {
    pub files: u64,
    pub videos: u64,
    pub estimated_bytes: u64,
    pub mode: CacheCleanupModeV1,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CacheTaskKindV1 {
    Backfill,
    NewRecords,
    Cleanup,
}

impl CacheTaskKindV1 {
    fn as_str(self) -> &'static str {
        match self {
            Self::Backfill => "backfill",
            Self::NewRecords => "new_records",
            Self::Cleanup => "cleanup",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "backfill" => Ok(Self::Backfill),
            "new_records" => Ok(Self::NewRecords),
            "cleanup" => Ok(Self::Cleanup),
            _ => Err("invalid_cover_cache_task_kind".to_string()),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CacheTaskStatusV1 {
    Queued,
    Running,
    Paused,
    Success,
    Failed,
    Cancelled,
}

impl CacheTaskStatusV1 {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "paused" => Ok(Self::Paused),
            "success" => Ok(Self::Success),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err("invalid_cover_cache_task_status".to_string()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CacheTaskViewV1 {
    pub id: i64,
    pub kind: CacheTaskKindV1,
    pub status: CacheTaskStatusV1,
    pub total_items: u64,
    pub completed_items: u64,
    pub failed_items: u64,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub error_code: Option<String>,
}

pub fn cache_relative_path(content_id: &str, mime_type: &str) -> Result<String, String> {
    if content_id.is_empty() {
        return Err("invalid_cover_content_id".to_string());
    }
    let extension = match mime_type {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => return Err("unsupported_cover_mime".to_string()),
    };
    let stem = if content_id.bytes().all(|byte| byte.is_ascii_digit()) {
        content_id.to_string()
    } else {
        format!("{:x}", Sha256::digest(content_id.as_bytes()))
    };
    let prefix = stem
        .get(0..2)
        .ok_or_else(|| "invalid_cover_content_id".to_string())?;
    Ok(format!("{prefix}/{stem}.{extension}"))
}

pub fn plan_cache_cleanup(
    entries: &[CacheEntryCandidateV1],
    rule: &CacheCleanupRuleV1,
    policy: &CacheCleanupPolicyV1,
    now: &str,
) -> Result<CacheCleanupPlanV1, String> {
    let now = DateTime::parse_from_rfc3339(now).map_err(|_| "invalid_cache_cleanup_time")?;
    let cutoff = policy
        .max_age_days
        .map(|days| now - Duration::days(i64::from(days)));
    let mut ordered = entries
        .iter()
        .map(|entry| {
            let accessed = DateTime::parse_from_rfc3339(&entry.last_accessed_at)
                .map_err(|_| "invalid_cache_entry_time".to_string())?;
            Ok((entry, accessed))
        })
        .collect::<Result<Vec<_>, String>>()?;
    ordered.sort_by(|left, right| {
        left.1
            .cmp(&right.1)
            .then(left.0.video_id.cmp(&right.0.video_id))
    });

    let mut selected = BTreeSet::new();
    match rule.mode {
        CacheCleanupModeV1::KeepListed
            if policy.max_bytes.is_none() && policy.max_age_days.is_none() =>
        {
            selected.extend(
                entries
                    .iter()
                    .filter(|entry| !rule.video_ids.contains(&entry.video_id))
                    .map(|entry| entry.video_id),
            );
        }
        CacheCleanupModeV1::CleanListed
            if policy.max_bytes.is_none() && policy.max_age_days.is_none() =>
        {
            selected.extend(rule.video_ids.iter().copied());
        }
        CacheCleanupModeV1::CleanListed => {}
        CacheCleanupModeV1::KeepListed => {}
    }
    if let Some(cutoff) = cutoff {
        for (entry, accessed) in &ordered {
            let eligible = match rule.mode {
                CacheCleanupModeV1::KeepListed => !rule.video_ids.contains(&entry.video_id),
                CacheCleanupModeV1::CleanListed => rule.video_ids.contains(&entry.video_id),
            };
            if eligible && *accessed < cutoff {
                selected.insert(entry.video_id);
            }
        }
    }
    if let Some(max_bytes) = policy.max_bytes {
        let mut retained = entries
            .iter()
            .filter(|entry| !selected.contains(&entry.video_id))
            .map(|entry| entry.byte_size)
            .sum::<u64>();
        for (entry, _) in &ordered {
            if retained <= max_bytes {
                break;
            }
            if selected.contains(&entry.video_id)
                || (rule.mode == CacheCleanupModeV1::KeepListed
                    && rule.video_ids.contains(&entry.video_id))
                || (rule.mode == CacheCleanupModeV1::CleanListed
                    && !rule.video_ids.contains(&entry.video_id))
            {
                continue;
            }
            selected.insert(entry.video_id);
            retained = retained.saturating_sub(entry.byte_size);
        }
    }

    let video_ids = ordered
        .iter()
        .filter_map(|(entry, _)| selected.contains(&entry.video_id).then_some(entry.video_id))
        .collect::<Vec<_>>();
    let bytes = entries
        .iter()
        .filter(|entry| selected.contains(&entry.video_id))
        .map(|entry| entry.byte_size)
        .sum();
    Ok(CacheCleanupPlanV1 { video_ids, bytes })
}

pub fn list_cache_rule_items(connection: &Connection) -> Result<Vec<CacheRuleItemV1>, String> {
    cache_rule_items_query(
        connection,
        "WHERE EXISTS(SELECT 1 FROM cover_cache_rules rules WHERE rules.video_id = videos.id)",
        None,
        5_000,
    )
}

pub fn search_cache_rule_candidates(
    connection: &Connection,
    query: &str,
    limit: usize,
) -> Result<Vec<CacheRuleItemV1>, String> {
    let trimmed = query.trim();
    let like = (!trimmed.is_empty()).then(|| format!("%{trimmed}%"));
    cache_rule_items_query(
        connection,
        "WHERE NOT EXISTS(SELECT 1 FROM cover_cache_rules rules WHERE rules.video_id = videos.id)
           AND (?1 IS NULL OR videos.aweme_id LIKE ?1 OR videos.title LIKE ?1 OR videos.author_name LIKE ?1)
           AND (EXISTS(SELECT 1 FROM cover_cache WHERE video_id = videos.id)
             OR EXISTS(SELECT 1 FROM watch_records WHERE video_id = videos.id)
             OR EXISTS(SELECT 1 FROM user_likes WHERE video_id = videos.id)
             OR EXISTS(SELECT 1 FROM favorite_items WHERE video_id = videos.id)
             OR EXISTS(SELECT 1 FROM remote_folder_items WHERE video_id = videos.id)
             OR EXISTS(SELECT 1 FROM local_folder_items WHERE video_id = videos.id))",
        like.as_deref(),
        limit.clamp(1, 200),
    )
}

fn cache_rule_items_query(
    connection: &Connection,
    where_clause: &str,
    query: Option<&str>,
    limit: usize,
) -> Result<Vec<CacheRuleItemV1>, String> {
    let sql = format!(
        "SELECT videos.id, videos.aweme_id, videos.title, videos.author_name, videos.cover_url,
                EXISTS(SELECT 1 FROM cover_cache WHERE video_id = videos.id)
         FROM videos {where_clause}
         ORDER BY videos.last_seen_at DESC, videos.id DESC LIMIT ?2"
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| error.to_string())?;
    let records = statement
        .query_map(params![query, limit as i64], |row| {
            Ok(CacheRuleItemV1 {
                video_id: row.get(0)?,
                aweme_id: row.get(1)?,
                title: row.get(2)?,
                author_name: row.get(3)?,
                cover_url: row.get(4)?,
                cached: row.get(5)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(records)
}

pub fn add_cache_rule_item(
    connection: &Connection,
    video_id: i64,
    created_at: &str,
) -> Result<(), String> {
    if video_id <= 0 {
        return Err("invalid_cover_video_id".to_string());
    }
    validate_task_time(created_at)?;
    let inserted = connection
        .execute(
            "INSERT OR IGNORE INTO cover_cache_rules(video_id, created_at)
             SELECT ?1, ?2 WHERE EXISTS(SELECT 1 FROM videos WHERE id = ?1)",
            params![video_id, created_at],
        )
        .map_err(|error| error.to_string())?;
    if inserted == 0 {
        let exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM cover_cache_rules WHERE video_id = ?1)",
                [video_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if !exists {
            return Err("cover_cache_video_missing".to_string());
        }
    }
    Ok(())
}

pub fn remove_cache_rule_item(connection: &Connection, video_id: i64) -> Result<bool, String> {
    connection
        .execute(
            "DELETE FROM cover_cache_rules WHERE video_id = ?1",
            [video_id],
        )
        .map(|count| count > 0)
        .map_err(|error| error.to_string())
}

pub fn clear_cache_rule_items(connection: &Connection) -> Result<u64, String> {
    connection
        .execute("DELETE FROM cover_cache_rules", [])
        .map(|count| count as u64)
        .map_err(|error| error.to_string())
}

pub fn preview_configured_cache_cleanup(
    connection: &Connection,
    mode: CacheCleanupModeV1,
    policy: &CacheCleanupPolicyV1,
    now: &str,
) -> Result<CacheCleanupPreviewV1, String> {
    let (entries, rule) = load_cleanup_inputs(connection, mode)?;
    let plan = plan_cache_cleanup(&entries, &rule, policy, now)?;
    Ok(CacheCleanupPreviewV1 {
        files: plan.video_ids.len() as u64,
        videos: plan.video_ids.len() as u64,
        estimated_bytes: plan.bytes,
        mode,
    })
}

pub fn execute_configured_cache_cleanup(
    connection: &mut Connection,
    covers_root: &Path,
    mode: CacheCleanupModeV1,
    policy: &CacheCleanupPolicyV1,
    now: &str,
) -> Result<CacheCleanupResultV1, String> {
    let (entries, rule) = load_cleanup_inputs(connection, mode)?;
    let plan = plan_cache_cleanup(&entries, &rule, policy, now)?;
    execute_cache_cleanup(connection, covers_root, &plan.video_ids)
}

fn load_cleanup_inputs(
    connection: &Connection,
    mode: CacheCleanupModeV1,
) -> Result<(Vec<CacheEntryCandidateV1>, CacheCleanupRuleV1), String> {
    let mut statement = connection
        .prepare(
            "SELECT video_id, byte_size, last_accessed_at
             FROM cover_cache ORDER BY last_accessed_at, video_id",
        )
        .map_err(|error| error.to_string())?;
    let entries = statement
        .query_map([], |row| {
            Ok(CacheEntryCandidateV1 {
                video_id: row.get(0)?,
                byte_size: u64::try_from(row.get::<_, i64>(1)?).unwrap_or(0),
                last_accessed_at: row.get(2)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let mut rules = connection
        .prepare("SELECT video_id FROM cover_cache_rules ORDER BY video_id")
        .map_err(|error| error.to_string())?;
    let video_ids = rules
        .query_map([], |row| row.get::<_, i64>(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<BTreeSet<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok((entries, CacheCleanupRuleV1 { mode, video_ids }))
}

#[allow(clippy::too_many_arguments)]
pub fn store_cover_bytes(
    connection: &Connection,
    covers_root: &Path,
    video_id: i64,
    content_id: &str,
    remote_url: &str,
    mime_type: &str,
    bytes: &[u8],
    downloaded_at: &str,
) -> Result<CoverCacheEntryV1, String> {
    if video_id <= 0 || bytes.is_empty() || remote_url.trim() != remote_url {
        return Err("invalid_cover_cache_entry".to_string());
    }
    url::Url::parse(remote_url)
        .ok()
        .filter(|url| matches!(url.scheme(), "http" | "https"))
        .ok_or_else(|| "invalid_cover_remote_url".to_string())?;
    DateTime::parse_from_rfc3339(downloaded_at)
        .map_err(|_| "invalid_cover_cache_time".to_string())?;
    let relative_path = cache_relative_path(content_id, mime_type)?;
    let target = controlled_cache_path(covers_root, &relative_path, true)?;
    let previous_path = connection
        .query_row(
            "SELECT relative_path FROM cover_cache WHERE video_id = ?1",
            [video_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let previous_bytes = if previous_path.as_deref() == Some(relative_path.as_str()) {
        fs::read(&target).ok()
    } else {
        None
    };
    write_bytes_atomically(&target, bytes)?;
    let sha256 = format!("{:x}", Sha256::digest(bytes));
    let byte_size = u64::try_from(bytes.len()).map_err(|_| "cover_cache_size_overflow")?;
    let updated = connection.execute(
        "INSERT INTO cover_cache
           (video_id, remote_url, relative_path, byte_size, mime_type, sha256,
            downloaded_at, last_accessed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
         ON CONFLICT(video_id) DO UPDATE SET
           remote_url = excluded.remote_url,
           relative_path = excluded.relative_path,
           byte_size = excluded.byte_size,
           mime_type = excluded.mime_type,
           sha256 = excluded.sha256,
           downloaded_at = excluded.downloaded_at,
           last_accessed_at = excluded.last_accessed_at",
        params![
            video_id,
            remote_url,
            relative_path,
            i64::try_from(byte_size).map_err(|_| "cover_cache_size_overflow")?,
            mime_type,
            sha256,
            downloaded_at,
        ],
    );
    if let Err(error) = updated {
        if let Some(previous_bytes) = previous_bytes {
            let _ = write_bytes_atomically(&target, &previous_bytes);
        } else {
            let _ = fs::remove_file(&target);
        }
        return Err(error.to_string());
    }
    if let Some(previous_path) = previous_path.filter(|path| path != &relative_path) {
        if let Ok(previous) = controlled_cache_path(covers_root, &previous_path, false) {
            let _ = fs::remove_file(previous);
        }
    }
    Ok(CoverCacheEntryV1 {
        video_id,
        remote_url: remote_url.to_string(),
        relative_path,
        byte_size,
        mime_type: mime_type.to_string(),
        sha256,
        downloaded_at: downloaded_at.to_string(),
        last_accessed_at: downloaded_at.to_string(),
    })
}

pub fn select_cover_candidates(
    connection: &Connection,
    sync_job_id: Option<i64>,
) -> Result<Vec<CoverCandidateV1>, String> {
    let query = if sync_job_id.is_some() {
        "SELECT DISTINCT videos.id, videos.aweme_id, videos.cover_url
         FROM videos
         JOIN watch_records ON watch_records.video_id = videos.id
         LEFT JOIN cover_cache ON cover_cache.video_id = videos.id
         WHERE watch_records.sync_job_id = ?1
           AND videos.cover_url IS NOT NULL AND length(trim(videos.cover_url)) > 0
           AND cover_cache.video_id IS NULL
         ORDER BY videos.id ASC"
    } else {
        "SELECT videos.id, videos.aweme_id, videos.cover_url
         FROM videos
         LEFT JOIN cover_cache ON cover_cache.video_id = videos.id
         WHERE videos.cover_url IS NOT NULL AND length(trim(videos.cover_url)) > 0
           AND cover_cache.video_id IS NULL
           AND (EXISTS(SELECT 1 FROM watch_records WHERE video_id = videos.id)
             OR EXISTS(SELECT 1 FROM user_likes WHERE video_id = videos.id)
             OR EXISTS(SELECT 1 FROM favorite_items WHERE video_id = videos.id)
             OR EXISTS(SELECT 1 FROM remote_folder_items WHERE video_id = videos.id)
             OR EXISTS(SELECT 1 FROM local_folder_items WHERE video_id = videos.id))
         ORDER BY videos.id ASC"
    };
    let mut statement = connection
        .prepare(query)
        .map_err(|error| error.to_string())?;
    let map_row = |row: &rusqlite::Row<'_>| {
        Ok(CoverCandidateV1 {
            video_id: row.get(0)?,
            aweme_id: row.get(1)?,
            remote_url: row.get(2)?,
        })
    };
    let records = match sync_job_id {
        Some(job_id) => statement
            .query_map([job_id], map_row)
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>(),
        None => statement
            .query_map([], map_row)
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>(),
    }
    .map_err(|error| error.to_string())?;
    Ok(records)
}

#[cfg(target_os = "windows")]
pub fn download_cover(url: &str, max_bytes: usize) -> Result<DownloadedCoverV1, String> {
    use std::{ffi::c_void, os::windows::ffi::OsStrExt, ptr};
    use windows_sys::Win32::Networking::WinHttp::{
        WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest,
        WinHttpQueryDataAvailable, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse,
        WinHttpSendRequest, WinHttpSetOption, WinHttpSetTimeouts,
        WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_OPTION_REDIRECT_POLICY,
        WINHTTP_OPTION_REDIRECT_POLICY_DISALLOW_HTTPS_TO_HTTP, WINHTTP_QUERY_CONTENT_TYPE,
        WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
    };

    struct Handle(*mut c_void);
    impl Drop for Handle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { WinHttpCloseHandle(self.0) };
            }
        }
    }
    fn wide(value: &str) -> Vec<u16> {
        std::ffi::OsStr::new(value)
            .encode_wide()
            .chain(Some(0))
            .collect()
    }
    fn winhttp_ok(value: i32, code: &str) -> Result<(), String> {
        if value == 0 {
            Err(format!("{code}:{}", std::io::Error::last_os_error()))
        } else {
            Ok(())
        }
    }

    if max_bytes == 0 {
        return Err("invalid_cover_download_limit".to_string());
    }
    let parsed = url::Url::parse(url).map_err(|_| "invalid_cover_remote_url")?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.username() != ""
        || parsed.password().is_some()
    {
        return Err("invalid_cover_remote_url".to_string());
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "invalid_cover_remote_url".to_string())?;
    let port = parsed
        .port_or_known_default()
        .ok_or_else(|| "invalid_cover_remote_url".to_string())?;
    let mut object_name = parsed.path().to_string();
    if object_name.is_empty() {
        object_name.push('/');
    }
    if let Some(query) = parsed.query() {
        object_name.push('?');
        object_name.push_str(query);
    }
    let agent = wide("Fy1DouyinArchiver/0.1 cover-cache");
    let session = Handle(unsafe {
        WinHttpOpen(
            agent.as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            ptr::null(),
            ptr::null(),
            0,
        )
    });
    if session.0.is_null() {
        return Err(format!(
            "cover_download_open_failed:{}",
            std::io::Error::last_os_error()
        ));
    }
    winhttp_ok(
        unsafe { WinHttpSetTimeouts(session.0, 10_000, 15_000, 15_000, 30_000) },
        "cover_download_timeout_setup_failed",
    )?;
    let redirect_policy = WINHTTP_OPTION_REDIRECT_POLICY_DISALLOW_HTTPS_TO_HTTP;
    winhttp_ok(
        unsafe {
            WinHttpSetOption(
                session.0,
                WINHTTP_OPTION_REDIRECT_POLICY,
                (&redirect_policy as *const u32).cast(),
                size_of::<u32>() as u32,
            )
        },
        "cover_download_redirect_setup_failed",
    )?;
    let host = wide(host);
    let connection = Handle(unsafe { WinHttpConnect(session.0, host.as_ptr(), port, 0) });
    if connection.0.is_null() {
        return Err(format!(
            "cover_download_connect_failed:{}",
            std::io::Error::last_os_error()
        ));
    }
    let verb = wide("GET");
    let object_name = wide(&object_name);
    let request = Handle(unsafe {
        WinHttpOpenRequest(
            connection.0,
            verb.as_ptr(),
            object_name.as_ptr(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            if parsed.scheme() == "https" {
                WINHTTP_FLAG_SECURE
            } else {
                0
            },
        )
    });
    if request.0.is_null() {
        return Err(format!(
            "cover_download_request_failed:{}",
            std::io::Error::last_os_error()
        ));
    }
    winhttp_ok(
        unsafe { WinHttpSendRequest(request.0, ptr::null(), 0, ptr::null(), 0, 0, 0) },
        "cover_download_send_failed",
    )?;
    winhttp_ok(
        unsafe { WinHttpReceiveResponse(request.0, ptr::null_mut()) },
        "cover_download_receive_failed",
    )?;
    let mut status = 0_u32;
    let mut status_size = size_of::<u32>() as u32;
    winhttp_ok(
        unsafe {
            WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                ptr::null(),
                (&mut status as *mut u32).cast(),
                &mut status_size,
                ptr::null_mut(),
            )
        },
        "cover_download_status_failed",
    )?;
    if !(200..300).contains(&status) {
        return Err(format!("cover_download_http_status:{status}"));
    }
    let mut content_type_size = 0_u32;
    unsafe {
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_CONTENT_TYPE,
            ptr::null(),
            ptr::null_mut(),
            &mut content_type_size,
            ptr::null_mut(),
        )
    };
    if content_type_size < 2 {
        return Err("unsupported_cover_mime".to_string());
    }
    let mut content_type = vec![0_u16; (content_type_size as usize).div_ceil(2)];
    winhttp_ok(
        unsafe {
            WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_CONTENT_TYPE,
                ptr::null(),
                content_type.as_mut_ptr().cast(),
                &mut content_type_size,
                ptr::null_mut(),
            )
        },
        "cover_download_content_type_failed",
    )?;
    let content_type = String::from_utf16_lossy(
        &content_type[..content_type
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(content_type.len())],
    );
    let mime_type = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if !matches!(
        mime_type.as_str(),
        "image/jpeg" | "image/png" | "image/webp" | "image/gif"
    ) {
        return Err("unsupported_cover_mime".to_string());
    }
    let mut bytes = Vec::new();
    loop {
        let mut available = 0_u32;
        winhttp_ok(
            unsafe { WinHttpQueryDataAvailable(request.0, &mut available) },
            "cover_download_query_failed",
        )?;
        if available == 0 {
            break;
        }
        let available = usize::try_from(available).map_err(|_| "cover_download_too_large")?;
        if bytes.len().saturating_add(available) > max_bytes {
            return Err("cover_download_too_large".to_string());
        }
        let start = bytes.len();
        bytes.resize(start + available, 0);
        let mut read = 0_u32;
        winhttp_ok(
            unsafe {
                WinHttpReadData(
                    request.0,
                    bytes[start..].as_mut_ptr().cast(),
                    available as u32,
                    &mut read,
                )
            },
            "cover_download_read_failed",
        )?;
        bytes.truncate(start + read as usize);
        if read == 0 {
            break;
        }
    }
    if bytes.is_empty() {
        return Err("cover_download_empty".to_string());
    }
    Ok(DownloadedCoverV1 { mime_type, bytes })
}

#[cfg(not(target_os = "windows"))]
pub fn download_cover(_url: &str, _max_bytes: usize) -> Result<DownloadedCoverV1, String> {
    Err("cover_download_unsupported_platform".to_string())
}

pub fn enqueue_cover_candidate_best_effort(
    transaction: &rusqlite::Transaction<'_>,
    video_id: i64,
    sync_job_id: i64,
    queued_at: &str,
) {
    let table_exists = transaction
        .query_row(
            "SELECT EXISTS(
               SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'cover_cache_queue'
             )",
            [],
            |row| row.get::<_, bool>(0),
        )
        .unwrap_or(false);
    if table_exists {
        let inserted = transaction
            .execute(
                "INSERT OR IGNORE INTO cover_cache_queue
                   (video_id, sync_job_id, queued_at, attempts, last_error)
                 SELECT ?1, ?2, ?3, 0, NULL
                 WHERE NOT EXISTS(SELECT 1 FROM cover_cache WHERE video_id = ?1)",
                params![video_id, sync_job_id, queued_at],
            )
            .unwrap_or(0);
        if inserted == 0 {
            let _ = transaction.execute(
                "UPDATE cover_cache_queue
                 SET sync_job_id = ?2, queued_at = ?3, last_error = NULL
                 WHERE video_id = ?1",
                params![video_id, sync_job_id, queued_at],
            );
        } else {
            let _ = transaction.execute(
                "UPDATE cover_cache_tasks
                 SET total_items = total_items + 1
                 WHERE status IN ('running', 'paused')",
                [],
            );
        }
    }
}

pub fn clear_cover_candidate_queue(connection: &Connection) -> Result<u64, String> {
    connection
        .execute("DELETE FROM cover_cache_queue", [])
        .map(|count| count as u64)
        .map_err(|error| error.to_string())
}

pub fn disable_cover_cache_queue(connection: &mut Connection, now: &str) -> Result<(), String> {
    validate_task_time(now)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    transaction
        .execute("DELETE FROM cover_cache_queue", [])
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "UPDATE cover_cache_tasks
             SET status = 'cancelled', finished_at = ?1, error_code = NULL
             WHERE status IN ('queued', 'running', 'paused')",
            [now],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())
}

pub fn enqueue_all_missing_cover_candidates(
    connection: &mut Connection,
    queued_at: &str,
) -> Result<u64, String> {
    validate_task_time(queued_at)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let inserted = transaction
        .execute(
            "INSERT OR IGNORE INTO cover_cache_queue
               (video_id, sync_job_id, queued_at, attempts, last_error)
             SELECT videos.id, NULL, ?1, 0, NULL
             FROM videos
             LEFT JOIN cover_cache ON cover_cache.video_id = videos.id
             WHERE videos.cover_url IS NOT NULL AND length(trim(videos.cover_url)) > 0
               AND cover_cache.video_id IS NULL
               AND (EXISTS(SELECT 1 FROM watch_records WHERE video_id = videos.id)
                 OR EXISTS(SELECT 1 FROM user_likes WHERE video_id = videos.id)
                 OR EXISTS(SELECT 1 FROM favorite_items WHERE video_id = videos.id)
                 OR EXISTS(SELECT 1 FROM remote_folder_items WHERE video_id = videos.id)
                 OR EXISTS(SELECT 1 FROM local_folder_items WHERE video_id = videos.id))",
            [queued_at],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    u64::try_from(inserted).map_err(|_| "cover_cache_task_limit".to_string())
}

pub fn ensure_pending_cache_task(
    connection: &mut Connection,
    kind: CacheTaskKindV1,
    now: &str,
) -> Result<Option<CacheTaskViewV1>, String> {
    if let Some(task) = latest_cache_task(connection)?.filter(|task| {
        matches!(
            task.status,
            CacheTaskStatusV1::Queued | CacheTaskStatusV1::Running | CacheTaskStatusV1::Paused
        )
    }) {
        return Ok(Some(task));
    }
    let total_items = connection
        .query_row(
            "SELECT COUNT(*) FROM cover_cache_queue
             WHERE attempts < 3
               AND EXISTS(
                 SELECT 1 FROM videos
                 WHERE videos.id = cover_cache_queue.video_id
                   AND videos.cover_url IS NOT NULL
                   AND length(trim(videos.cover_url)) > 0
               )",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| error.to_string())?;
    if total_items == 0 {
        return Ok(None);
    }
    start_cache_task(
        connection,
        kind,
        u64::try_from(total_items).map_err(|_| "cover_cache_task_limit")?,
        now,
    )
    .map(Some)
}

pub fn select_queued_cover_candidates(
    connection: &Connection,
    limit: usize,
) -> Result<Vec<CoverCandidateV1>, String> {
    let mut statement = connection
        .prepare(
            "SELECT videos.id, videos.aweme_id, videos.cover_url
             FROM cover_cache_queue queue
             JOIN videos ON videos.id = queue.video_id
             LEFT JOIN cover_cache ON cover_cache.video_id = videos.id
             WHERE videos.cover_url IS NOT NULL AND length(trim(videos.cover_url)) > 0
               AND cover_cache.video_id IS NULL
             ORDER BY queue.queued_at ASC, queue.video_id ASC
             LIMIT ?1",
        )
        .map_err(|error| error.to_string())?;
    let records = statement
        .query_map([limit.clamp(1, 1_000) as i64], |row| {
            Ok(CoverCandidateV1 {
                video_id: row.get(0)?,
                aweme_id: row.get(1)?,
                remote_url: row.get(2)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(records)
}

pub fn remove_cover_candidate(connection: &Connection, video_id: i64) -> Result<(), String> {
    connection
        .execute(
            "DELETE FROM cover_cache_queue WHERE video_id = ?1",
            [video_id],
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
}

pub fn record_cover_candidate_failure(
    connection: &Connection,
    video_id: i64,
    error_code: &str,
) -> Result<(), String> {
    connection
        .execute(
            "UPDATE cover_cache_queue
             SET attempts = attempts + 1, last_error = ?2 WHERE video_id = ?1",
            params![video_id, error_code],
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
}

pub fn read_cached_cover(
    connection: &Connection,
    covers_root: &Path,
    video_id: i64,
    accessed_at: &str,
) -> Result<CachedCoverAssetV1, String> {
    if video_id <= 0 {
        return Err("invalid_cover_video_id".to_string());
    }
    validate_task_time(accessed_at)?;
    let entry = connection
        .query_row(
            "SELECT relative_path, byte_size, mime_type, sha256
             FROM cover_cache WHERE video_id = ?1",
            [video_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "cover_cache_missing".to_string())?;
    let path = controlled_cache_path(covers_root, &entry.0, false)?;
    let metadata = fs::symlink_metadata(&path).map_err(|error| {
        if error.kind() == ErrorKind::NotFound {
            "cover_cache_missing".to_string()
        } else {
            error.to_string()
        }
    })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata_is_reparse(&metadata) {
        return Err("cover_cache_corrupt".to_string());
    }
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    if metadata.len() != u64::try_from(entry.1).unwrap_or(0)
        || format!("{:x}", Sha256::digest(&bytes)) != entry.3
    {
        return Err("cover_cache_corrupt".to_string());
    }
    connection
        .execute(
            "UPDATE cover_cache SET last_accessed_at = ?2 WHERE video_id = ?1",
            params![video_id, accessed_at],
        )
        .map_err(|error| error.to_string())?;
    Ok(CachedCoverAssetV1 {
        mime_type: entry.2,
        bytes,
    })
}

pub fn cached_cover_url_for_video_id(
    connection: &Connection,
    video_id: i64,
) -> Result<Option<String>, String> {
    if video_id <= 0 || !schema_table_exists(connection, "cover_cache")? {
        return Ok(None);
    }
    let cached: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM cover_cache WHERE video_id = ?1)",
            [video_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    Ok(cached.then(|| format!("http://cover-cache.localhost/{video_id}")))
}

pub fn cached_cover_url_for_aweme_id(
    connection: &Connection,
    aweme_id: &str,
) -> Result<Option<String>, String> {
    if !schema_table_exists(connection, "cover_cache")? {
        return Ok(None);
    }
    let video_id = connection
        .query_row(
            "SELECT videos.id FROM videos
             JOIN cover_cache ON cover_cache.video_id = videos.id
             WHERE videos.aweme_id = ?1",
            [aweme_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    Ok(video_id.map(|video_id| format!("http://cover-cache.localhost/{video_id}")))
}

pub fn process_cache_task_step_with<F>(
    connection: &mut Connection,
    covers_root: &Path,
    task_id: i64,
    max_download_bytes: usize,
    now: &str,
    mut downloader: F,
) -> Result<CacheTaskStepV1, String>
where
    F: FnMut(&str) -> Result<DownloadedCoverV1, String>,
{
    const MAX_ATTEMPTS: i64 = 3;
    let task = get_cache_task(connection, task_id)?
        .ok_or_else(|| "cover_cache_task_missing".to_string())?;
    match task.status {
        CacheTaskStatusV1::Paused => return Ok(CacheTaskStepV1::Paused),
        CacheTaskStatusV1::Success | CacheTaskStatusV1::Failed | CacheTaskStatusV1::Cancelled => {
            return Ok(CacheTaskStepV1::Finished)
        }
        CacheTaskStatusV1::Queued | CacheTaskStatusV1::Running => {}
    }
    let candidate = connection
        .query_row(
            "SELECT videos.id, videos.aweme_id, videos.cover_url, queue.attempts
             FROM cover_cache_queue queue
             JOIN videos ON videos.id = queue.video_id
             LEFT JOIN cover_cache ON cover_cache.video_id = videos.id
             WHERE videos.cover_url IS NOT NULL AND length(trim(videos.cover_url)) > 0
               AND cover_cache.video_id IS NULL AND queue.attempts < ?1
             ORDER BY queue.queued_at ASC, queue.video_id ASC LIMIT 1",
            [MAX_ATTEMPTS],
            |row| {
                Ok((
                    CoverCandidateV1 {
                        video_id: row.get(0)?,
                        aweme_id: row.get(1)?,
                        remote_url: row.get(2)?,
                    },
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((candidate, attempts)) = candidate else {
        connection
            .execute(
                "DELETE FROM cover_cache_queue
                 WHERE video_id IN (
                   SELECT queue.video_id FROM cover_cache_queue queue
                   LEFT JOIN videos ON videos.id = queue.video_id
                   LEFT JOIN cover_cache ON cover_cache.video_id = queue.video_id
                   WHERE videos.id IS NULL OR videos.cover_url IS NULL
                      OR length(trim(videos.cover_url)) = 0 OR cover_cache.video_id IS NOT NULL
                 )",
                [],
            )
            .map_err(|error| error.to_string())?;
        finish_cache_task(
            connection,
            task_id,
            if task.failed_items == 0 {
                CacheTaskStatusV1::Success
            } else {
                CacheTaskStatusV1::Failed
            },
            now,
            (task.failed_items > 0).then_some("cover_download_failed"),
        )?;
        return Ok(CacheTaskStepV1::Finished);
    };

    let download = downloader(&candidate.remote_url);
    let current = get_cache_task(connection, task_id)?
        .ok_or_else(|| "cover_cache_task_missing".to_string())?;
    match current.status {
        CacheTaskStatusV1::Paused => return Ok(CacheTaskStepV1::Paused),
        CacheTaskStatusV1::Success | CacheTaskStatusV1::Failed | CacheTaskStatusV1::Cancelled => {
            return Ok(CacheTaskStepV1::Finished)
        }
        CacheTaskStatusV1::Queued | CacheTaskStatusV1::Running => {}
    }
    match download.and_then(|downloaded| {
        if downloaded.bytes.len() > max_download_bytes {
            return Err("cover_download_too_large".to_string());
        }
        store_cover_bytes(
            connection,
            covers_root,
            candidate.video_id,
            &candidate.aweme_id,
            &candidate.remote_url,
            &downloaded.mime_type,
            &downloaded.bytes,
            now,
        )
        .map(|_| ())
    }) {
        Ok(()) => {
            remove_cover_candidate(connection, candidate.video_id)?;
            update_cache_task_progress(
                connection,
                task_id,
                task.completed_items + 1,
                task.failed_items,
            )?;
        }
        Err(error) if attempts + 1 < MAX_ATTEMPTS => {
            record_cover_candidate_failure(connection, candidate.video_id, &error)?;
            return Ok(CacheTaskStepV1::Progressed);
        }
        Err(error) => {
            record_cover_candidate_failure(connection, candidate.video_id, &error)?;
            update_cache_task_progress(
                connection,
                task_id,
                task.completed_items + 1,
                task.failed_items + 1,
            )?;
        }
    }

    let remaining: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM cover_cache_queue WHERE attempts < ?1)",
            [MAX_ATTEMPTS],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if remaining {
        Ok(CacheTaskStepV1::Progressed)
    } else {
        let task = get_cache_task(connection, task_id)?
            .ok_or_else(|| "cover_cache_task_missing".to_string())?;
        finish_cache_task(
            connection,
            task_id,
            if task.failed_items == 0 {
                CacheTaskStatusV1::Success
            } else {
                CacheTaskStatusV1::Failed
            },
            now,
            (task.failed_items > 0).then_some("cover_download_failed"),
        )?;
        Ok(CacheTaskStepV1::Finished)
    }
}

pub fn inspect_cache(
    connection: &Connection,
    covers_root: &Path,
) -> Result<CacheInspectionV1, String> {
    let mut statement = connection
        .prepare(
            "SELECT video_id, relative_path, byte_size, sha256
             FROM cover_cache ORDER BY video_id ASC",
        )
        .map_err(|error| error.to_string())?;
    let indexed = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let mut inspection = CacheInspectionV1::default();
    let mut known = BTreeSet::new();
    for (_, relative_path, byte_size, expected_hash) in indexed {
        let Ok(path) = controlled_cache_path(covers_root, &relative_path, false) else {
            inspection.invalid_indexes += 1;
            continue;
        };
        known.insert(normalized_relative_key(&relative_path)?);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                inspection.missing_files += 1;
                continue;
            }
            Err(error) => return Err(error.to_string()),
        };
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata_is_reparse(&metadata)
        {
            inspection.invalid_indexes += 1;
            continue;
        }
        let bytes = fs::read(&path).map_err(|error| error.to_string())?;
        let actual_hash = format!("{:x}", Sha256::digest(&bytes));
        if actual_hash != expected_hash || metadata.len() != u64::try_from(byte_size).unwrap_or(0) {
            inspection.invalid_indexes += 1;
            continue;
        }
        inspection.healthy_files += 1;
        inspection.indexed_bytes = inspection.indexed_bytes.saturating_add(metadata.len());
    }
    for (relative, size) in scan_cache_files(covers_root)? {
        if !known.contains(&relative) {
            inspection.orphan_files += 1;
            inspection.orphan_bytes = inspection.orphan_bytes.saturating_add(size);
        }
    }
    Ok(inspection)
}

pub fn cleanup_broken_cache_entries(
    connection: &mut Connection,
    covers_root: &Path,
) -> Result<CacheCleanupResultV1, String> {
    let mut statement = connection
        .prepare(
            "SELECT video_id, relative_path, byte_size, sha256
             FROM cover_cache ORDER BY video_id ASC",
        )
        .map_err(|error| error.to_string())?;
    let indexed = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    drop(statement);

    let mut known = BTreeSet::new();
    let mut invalid_video_ids = Vec::new();
    let mut invalid_files = Vec::new();
    for (video_id, relative_path, byte_size, expected_hash) in &indexed {
        let normalized = match normalized_relative_key(relative_path) {
            Ok(normalized) => normalized,
            Err(_) => {
                invalid_video_ids.push(*video_id);
                continue;
            }
        };
        known.insert(normalized);
        let path = match controlled_cache_path(covers_root, relative_path, false) {
            Ok(path) => path,
            Err(_) => {
                invalid_video_ids.push(*video_id);
                continue;
            }
        };
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                invalid_video_ids.push(*video_id);
                continue;
            }
            Err(error) => return Err(error.to_string()),
        };
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata_is_reparse(&metadata)
        {
            invalid_video_ids.push(*video_id);
            continue;
        }
        let bytes = fs::read(&path).map_err(|error| error.to_string())?;
        if metadata.len() != u64::try_from(*byte_size).unwrap_or(0)
            || format!("{:x}", Sha256::digest(&bytes)) != *expected_hash
        {
            invalid_video_ids.push(*video_id);
            invalid_files.push((path, metadata.len()));
        }
    }

    let orphan_files = scan_cache_files(covers_root)?
        .into_iter()
        .filter(|(relative, _)| !known.contains(relative))
        .map(|(relative, size)| {
            controlled_cache_path(covers_root, &relative, false).map(|path| (path, size))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let mut result = CacheCleanupResultV1::default();
    for video_id in invalid_video_ids {
        result.deleted_indexes += transaction
            .execute("DELETE FROM cover_cache WHERE video_id = ?1", [video_id])
            .map_err(|error| error.to_string())? as u64;
    }
    transaction.commit().map_err(|error| error.to_string())?;

    invalid_files.extend(orphan_files);
    for (path, size) in invalid_files {
        match fs::remove_file(path) {
            Ok(()) => {
                result.deleted_files += 1;
                result.released_bytes = result.released_bytes.saturating_add(size);
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(result)
}

pub fn execute_cache_cleanup(
    connection: &mut Connection,
    covers_root: &Path,
    video_ids: &[i64],
) -> Result<CacheCleanupResultV1, String> {
    let unique = video_ids
        .iter()
        .copied()
        .filter(|video_id| *video_id > 0)
        .collect::<BTreeSet<_>>();
    let mut entries = Vec::new();
    for video_id in unique {
        let entry = connection
            .query_row(
                "SELECT relative_path, byte_size FROM cover_cache WHERE video_id = ?1",
                [video_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if let Some((relative_path, byte_size)) = entry {
            let path = controlled_cache_path(covers_root, &relative_path, false)?;
            entries.push((video_id, path, u64::try_from(byte_size).unwrap_or(0)));
        }
    }
    for (_, path, _) in &entries {
        match fs::symlink_metadata(path) {
            Ok(metadata)
                if !metadata.is_file()
                    || metadata.file_type().is_symlink()
                    || metadata_is_reparse(&metadata) =>
            {
                return Err("cover_cache_path_not_regular".to_string())
            }
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let mut result = CacheCleanupResultV1::default();
    for (video_id, path, indexed_size) in &entries {
        match fs::remove_file(path) {
            Ok(()) => {
                result.deleted_files += 1;
                result.released_bytes = result.released_bytes.saturating_add(*indexed_size);
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
        result.deleted_indexes += transaction
            .execute("DELETE FROM cover_cache WHERE video_id = ?1", [video_id])
            .map_err(|error| error.to_string())? as u64;
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(result)
}

pub fn start_cache_task(
    connection: &mut Connection,
    kind: CacheTaskKindV1,
    total_items: u64,
    now: &str,
) -> Result<CacheTaskViewV1, String> {
    validate_task_time(now)?;
    let total_items = i64::try_from(total_items).map_err(|_| "cover_cache_task_limit")?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let active: bool = transaction
        .query_row(
            "SELECT EXISTS(
               SELECT 1 FROM cover_cache_tasks WHERE status IN ('queued', 'running', 'paused')
             )",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if active {
        return Err("cover_cache_task_active".to_string());
    }
    transaction
        .execute(
            "INSERT INTO cover_cache_tasks
               (kind, status, total_items, completed_items, failed_items,
                created_at, started_at)
             VALUES (?1, 'running', ?2, 0, 0, ?3, ?3)",
            params![kind.as_str(), total_items, now],
        )
        .map_err(|error| error.to_string())?;
    let id = transaction.last_insert_rowid();
    transaction.commit().map_err(|error| error.to_string())?;
    get_cache_task(connection, id)?.ok_or_else(|| "cover_cache_task_missing".to_string())
}

pub fn get_cache_task(
    connection: &Connection,
    task_id: i64,
) -> Result<Option<CacheTaskViewV1>, String> {
    connection
        .query_row(
            "SELECT id, kind, status, total_items, completed_items, failed_items,
                    created_at, started_at, finished_at, error_code
             FROM cover_cache_tasks WHERE id = ?1",
            [task_id],
            cache_task_from_row,
        )
        .optional()
        .map_err(|error| error.to_string())?
        .map(parse_cache_task)
        .transpose()
}

pub fn latest_cache_task(connection: &Connection) -> Result<Option<CacheTaskViewV1>, String> {
    connection
        .query_row(
            "SELECT id, kind, status, total_items, completed_items, failed_items,
                    created_at, started_at, finished_at, error_code
             FROM cover_cache_tasks ORDER BY id DESC LIMIT 1",
            [],
            cache_task_from_row,
        )
        .optional()
        .map_err(|error| error.to_string())?
        .map(parse_cache_task)
        .transpose()
}

pub fn update_cache_task_progress(
    connection: &Connection,
    task_id: i64,
    completed_items: u64,
    failed_items: u64,
) -> Result<CacheTaskViewV1, String> {
    let completed = i64::try_from(completed_items).map_err(|_| "cover_cache_task_limit")?;
    let failed = i64::try_from(failed_items).map_err(|_| "cover_cache_task_limit")?;
    if failed > completed {
        return Err("invalid_cover_cache_task_progress".to_string());
    }
    let updated = connection
        .execute(
            "UPDATE cover_cache_tasks
             SET completed_items = ?2, failed_items = ?3
             WHERE id = ?1 AND status = 'running' AND ?2 >= completed_items
               AND ?2 <= total_items AND ?3 >= failed_items AND ?3 <= ?2",
            params![task_id, completed, failed],
        )
        .map_err(|error| error.to_string())?;
    if updated != 1 {
        return Err("cover_cache_task_state_changed".to_string());
    }
    get_cache_task(connection, task_id)?.ok_or_else(|| "cover_cache_task_missing".to_string())
}

pub fn pause_cache_task(connection: &Connection, task_id: i64) -> Result<CacheTaskViewV1, String> {
    transition_cache_task(connection, task_id, "running", "paused", None, None)
}

pub fn resume_cache_task(connection: &Connection, task_id: i64) -> Result<CacheTaskViewV1, String> {
    transition_cache_task(connection, task_id, "paused", "running", None, None)
}

pub fn request_cache_task_cancel(
    connection: &Connection,
    task_id: i64,
    now: &str,
) -> Result<CacheTaskViewV1, String> {
    validate_task_time(now)?;
    let updated = connection
        .execute(
            "UPDATE cover_cache_tasks
             SET status = 'cancelled', finished_at = ?2, error_code = NULL
             WHERE id = ?1 AND status IN ('queued', 'running', 'paused')",
            params![task_id, now],
        )
        .map_err(|error| error.to_string())?;
    if updated != 1 {
        return Err("cover_cache_task_state_changed".to_string());
    }
    get_cache_task(connection, task_id)?.ok_or_else(|| "cover_cache_task_missing".to_string())
}

pub fn finish_cache_task(
    connection: &Connection,
    task_id: i64,
    status: CacheTaskStatusV1,
    now: &str,
    error_code: Option<&str>,
) -> Result<CacheTaskViewV1, String> {
    validate_task_time(now)?;
    let status_text = match status {
        CacheTaskStatusV1::Success => "success",
        CacheTaskStatusV1::Failed => "failed",
        CacheTaskStatusV1::Cancelled => "cancelled",
        _ => return Err("invalid_cover_cache_terminal_status".to_string()),
    };
    let updated = connection
        .execute(
            "UPDATE cover_cache_tasks
             SET status = ?2, finished_at = ?3, error_code = ?4
             WHERE id = ?1 AND status IN ('queued', 'running', 'paused')",
            params![task_id, status_text, now, error_code],
        )
        .map_err(|error| error.to_string())?;
    if updated != 1 {
        return Err("cover_cache_task_state_changed".to_string());
    }
    get_cache_task(connection, task_id)?.ok_or_else(|| "cover_cache_task_missing".to_string())
}

pub fn retry_cache_task(
    connection: &mut Connection,
    task_id: i64,
    now: &str,
) -> Result<CacheTaskViewV1, String> {
    let previous = get_cache_task(connection, task_id)?
        .ok_or_else(|| "cover_cache_task_missing".to_string())?;
    if !matches!(
        previous.status,
        CacheTaskStatusV1::Failed | CacheTaskStatusV1::Cancelled
    ) {
        return Err("cover_cache_task_not_retryable".to_string());
    }
    let queued_items = connection
        .query_row("SELECT COUNT(*) FROM cover_cache_queue", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|error| error.to_string())?;
    connection
        .execute(
            "UPDATE cover_cache_queue SET attempts = 0, last_error = NULL",
            [],
        )
        .map_err(|error| error.to_string())?;
    let total_items = if queued_items > 0 {
        u64::try_from(queued_items).map_err(|_| "cover_cache_task_limit")?
    } else {
        previous.total_items
    };
    start_cache_task(connection, previous.kind, total_items, now)
}

pub fn recover_interrupted_cache_tasks(connection: &Connection, now: &str) -> Result<u64, String> {
    validate_task_time(now)?;
    connection
        .execute(
            "UPDATE cover_cache_tasks
             SET status = 'failed', finished_at = ?1, error_code = 'cache_task_interrupted'
             WHERE status IN ('queued', 'running', 'paused')",
            [now],
        )
        .map(|count| count as u64)
        .map_err(|error| error.to_string())
}

fn transition_cache_task(
    connection: &Connection,
    task_id: i64,
    from: &str,
    to: &str,
    finished_at: Option<&str>,
    error_code: Option<&str>,
) -> Result<CacheTaskViewV1, String> {
    let updated = connection
        .execute(
            "UPDATE cover_cache_tasks
             SET status = ?3, finished_at = ?4, error_code = ?5
             WHERE id = ?1 AND status = ?2",
            params![task_id, from, to, finished_at, error_code],
        )
        .map_err(|error| error.to_string())?;
    if updated != 1 {
        return Err("cover_cache_task_state_changed".to_string());
    }
    get_cache_task(connection, task_id)?.ok_or_else(|| "cover_cache_task_missing".to_string())
}

type RawCacheTask = (
    i64,
    String,
    String,
    i64,
    i64,
    i64,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

fn cache_task_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawCacheTask> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
    ))
}

fn parse_cache_task(raw: RawCacheTask) -> Result<CacheTaskViewV1, String> {
    Ok(CacheTaskViewV1 {
        id: raw.0,
        kind: CacheTaskKindV1::parse(&raw.1)?,
        status: CacheTaskStatusV1::parse(&raw.2)?,
        total_items: u64::try_from(raw.3).map_err(|_| "invalid_cover_cache_task")?,
        completed_items: u64::try_from(raw.4).map_err(|_| "invalid_cover_cache_task")?,
        failed_items: u64::try_from(raw.5).map_err(|_| "invalid_cover_cache_task")?,
        created_at: raw.6,
        started_at: raw.7,
        finished_at: raw.8,
        error_code: raw.9,
    })
}

fn validate_task_time(value: &str) -> Result<(), String> {
    DateTime::parse_from_rfc3339(value)
        .map(|_| ())
        .map_err(|_| "invalid_cover_cache_task_time".to_string())
}

fn schema_table_exists(connection: &Connection, table: &str) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT EXISTS(
               SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1
             )",
            [table],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
}

fn controlled_cache_path(
    covers_root: &Path,
    relative_path: &str,
    create_parent: bool,
) -> Result<PathBuf, String> {
    let relative = Path::new(relative_path);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("cover_cache_path_escape".to_string());
    }
    let normalized = normalized_relative_key(relative_path)?;
    if normalized != relative_path.replace('\\', "/") {
        return Err("cover_cache_path_escape".to_string());
    }
    if covers_root.exists() {
        assert_regular_directory(covers_root)?;
    } else if create_parent {
        fs::create_dir_all(covers_root).map_err(|error| error.to_string())?;
        assert_regular_directory(covers_root)?;
    }
    let target = covers_root.join(relative);
    let parent = target
        .parent()
        .ok_or_else(|| "cover_cache_path_escape".to_string())?;
    if parent.exists() {
        assert_regular_directory(parent)?;
    } else if create_parent {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        assert_regular_directory(parent)?;
    }
    Ok(target)
}

fn normalized_relative_key(relative_path: &str) -> Result<String, String> {
    let components = Path::new(relative_path)
        .components()
        .map(|component| match component {
            Component::Normal(value) => value
                .to_str()
                .map(str::to_string)
                .ok_or_else(|| "cover_cache_path_escape".to_string()),
            _ => Err("cover_cache_path_escape".to_string()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if components.is_empty() {
        return Err("cover_cache_path_escape".to_string());
    }
    Ok(components.join("/"))
}

fn assert_regular_directory(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() || metadata_is_reparse(&metadata) {
        return Err("cover_cache_path_not_regular".to_string());
    }
    Ok(())
}

fn scan_cache_files(root: &Path) -> Result<BTreeMap<String, u64>, String> {
    if !root.exists() {
        return Ok(BTreeMap::new());
    }
    assert_regular_directory(root)?;
    let mut files = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
            if metadata.file_type().is_symlink() || metadata_is_reparse(&metadata) {
                return Err("cover_cache_path_not_regular".to_string());
            }
            if metadata.is_dir() {
                pending.push(path);
            } else if metadata.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .map_err(|_| "cover_cache_path_escape".to_string())?
                    .to_string_lossy()
                    .replace('\\', "/");
                files.insert(relative, metadata.len());
            } else {
                return Err("cover_cache_path_not_regular".to_string());
            }
        }
    }
    Ok(files)
}

#[cfg(target_os = "windows")]
fn metadata_is_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(target_os = "windows"))]
fn metadata_is_reparse(_metadata: &fs::Metadata) -> bool {
    false
}
