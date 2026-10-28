use crate::settings_store::{validate_account_scope, AccountScopeV1};
use chrono::{Datelike, FixedOffset, NaiveDate, TimeZone, Utc};
use rusqlite::{params_from_iter, types::Value, Connection};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const SHANGHAI_OFFSET_SECONDS: i32 = 8 * 60 * 60;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnnualReviewQueryV1 {
    pub account_scope: AccountScopeV1,
    pub year: Option<i32>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AnnualReviewItemV1 {
    pub label: String,
    pub value: String,
    pub secondary: Option<String>,
    pub rate: Option<f64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AnnualReviewChapterV1 {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub omitted: bool,
    pub omitted_reason: Option<String>,
    pub items: Vec<AnnualReviewItemV1>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AnnualReviewViewV1 {
    pub year: i32,
    pub available_years: Vec<i32>,
    pub chapters: Vec<AnnualReviewChapterV1>,
}

#[derive(Debug, Clone)]
struct HistoryEvent {
    account_id: i64,
    video_id: i64,
    title: Option<String>,
    author_name: Option<String>,
    date: NaiveDate,
    progress: Option<f64>,
}

pub fn query_annual_review(
    connection: &Connection,
    query: AnnualReviewQueryV1,
) -> Result<AnnualReviewViewV1, String> {
    query_annual_review_at(connection, query, Utc::now().timestamp_millis())
}

pub fn query_annual_review_at(
    connection: &Connection,
    query: AnnualReviewQueryV1,
    reference_time_ms: i64,
) -> Result<AnnualReviewViewV1, String> {
    if reference_time_ms <= 0 {
        return Err("invalid_annual_review_reference_time".to_string());
    }
    let account_ids = resolve_account_ids(connection, &query.account_scope)?;
    let events = load_history_events(connection, &account_ids)?;
    let available_years = events
        .iter()
        .map(|event| event.date.year())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>();
    let current_year = shanghai_date(reference_time_ms)?.year();
    let year = query.year.unwrap_or_else(|| {
        if available_years.contains(&current_year) {
            current_year
        } else {
            available_years.first().copied().unwrap_or(current_year)
        }
    });
    let year_events = events
        .iter()
        .filter(|event| event.date.year() == year)
        .cloned()
        .collect::<Vec<_>>();
    let snapshot = load_archive_snapshot(connection, &account_ids)?;

    Ok(AnnualReviewViewV1 {
        year,
        available_years,
        chapters: build_chapters(&year_events, &snapshot),
    })
}

fn resolve_account_ids(
    connection: &Connection,
    scope: &AccountScopeV1,
) -> Result<Vec<i64>, String> {
    validate_account_scope(scope)?;
    let mut statement = connection
        .prepare(
            "SELECT id FROM accounts
             WHERE status IN ('active', 'login_expired', 'identity_conflict', 'archived')
             ORDER BY id",
        )
        .map_err(|error| error.to_string())?;
    let readable = statement
        .query_map([], |row| row.get::<_, i64>(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    match scope {
        AccountScopeV1::All => Ok(readable),
        AccountScopeV1::Selected { account_ids }
            if account_ids
                .iter()
                .all(|account_id| readable.binary_search(account_id).is_ok()) =>
        {
            Ok(account_ids.clone())
        }
        AccountScopeV1::Selected { .. } => Err("account_scope_invalid".to_string()),
    }
}

fn load_history_events(
    connection: &Connection,
    account_ids: &[i64],
) -> Result<Vec<HistoryEvent>, String> {
    if account_ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = question_marks(account_ids.len());
    let sql = format!(
        "SELECT wr.account_id, wr.video_id, v.title, v.author_name,
                wr.watched_date, wr.history_time_ms, wr.watch_progress_rate
         FROM watch_records wr
         JOIN videos v ON v.id = wr.video_id
         WHERE wr.account_id IN ({placeholders})
         ORDER BY wr.history_time_ms ASC, wr.id ASC"
    );
    let mut values = account_ids
        .iter()
        .copied()
        .map(Value::from)
        .collect::<Vec<_>>();
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params_from_iter(values.drain(..)), |row| {
            let watched_date: String = row.get(4)?;
            let history_time_ms: i64 = row.get(5)?;
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                watched_date,
                history_time_ms,
                row.get::<_, Option<f64>>(6)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    rows.into_iter()
        .map(
            |(
                account_id,
                video_id,
                title,
                author_name,
                watched_date,
                history_time_ms,
                progress,
            )| {
                Ok(HistoryEvent {
                    account_id,
                    video_id,
                    title,
                    author_name,
                    date: parse_local_date(&watched_date, history_time_ms)?,
                    progress,
                })
            },
        )
        .collect()
}

fn load_archive_snapshot(
    connection: &Connection,
    account_ids: &[i64],
) -> Result<BTreeMap<&'static str, i64>, String> {
    let mut snapshot = BTreeMap::new();
    if account_ids.is_empty() {
        return Ok(snapshot);
    }
    let placeholders = question_marks(account_ids.len());
    let sql = format!(
        "SELECT 'history', COUNT(*) FROM watch_records WHERE account_id IN ({p})
         UNION ALL
         SELECT 'likes', COUNT(*) FROM user_likes WHERE account_id IN ({p}) AND remote_removed_at IS NULL
         UNION ALL
         SELECT 'favorites', COUNT(*) FROM favorite_items WHERE account_id IN ({p}) AND remote_removed_at IS NULL",
        p = placeholders
    );
    let mut values = Vec::with_capacity(account_ids.len() * 3);
    for _ in 0..3 {
        values.extend(account_ids.iter().copied().map(Value::from));
    }
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params_from_iter(values), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    for (source, count) in rows {
        snapshot.insert(
            match source.as_str() {
                "history" => "history",
                "likes" => "likes",
                "favorites" => "favorites",
                _ => return Err("invalid_annual_review_source".to_string()),
            },
            count,
        );
    }
    Ok(snapshot)
}

fn build_chapters(
    events: &[HistoryEvent],
    snapshot: &BTreeMap<&'static str, i64>,
) -> Vec<AnnualReviewChapterV1> {
    vec![
        footprint_chapter(events),
        rhythm_chapter(events),
        streak_chapter(events),
        authors_chapter(events),
        rewatch_chapter(events),
        progress_chapter(events),
        archive_snapshot_chapter(snapshot),
        closing_chapter(events),
    ]
}

fn footprint_chapter(events: &[HistoryEvent]) -> AnnualReviewChapterV1 {
    let mut dates = BTreeSet::new();
    for event in events {
        dates.insert(event.date);
    }
    if events.is_empty() {
        return omitted_chapter(
            "footprint",
            "足迹",
            "这一年没有观看历史",
            "没有可用的观看历史",
        );
    }
    let first = dates.first().expect("non-empty dates");
    let last = dates.last().expect("non-empty dates");
    chapter(
        "footprint",
        "足迹",
        format!("在 {days} 个日子留下了观看记录", days = dates.len()),
        vec![
            item("活跃天数", dates.len().to_string()),
            item("观看事件", events.len().to_string()),
            item("起点", first.to_string()),
            item("最近", last.to_string()),
        ],
    )
}

fn rhythm_chapter(events: &[HistoryEvent]) -> AnnualReviewChapterV1 {
    if events.is_empty() {
        return omitted_chapter(
            "rhythm",
            "节奏",
            "没有可分析的观看节奏",
            "没有可用的观看历史",
        );
    }
    let mut counts = BTreeMap::<NaiveDate, i64>::new();
    for event in events {
        *counts.entry(event.date).or_default() += 1;
    }
    let (peak_date, peak_count) = counts
        .iter()
        .max_by_key(|(date, count)| (**count, std::cmp::Reverse(**date)))
        .map(|(date, count)| (*date, *count))
        .expect("non-empty counts");
    let average = events.len() as f64 / counts.len() as f64;
    chapter(
        "rhythm",
        "节奏",
        format!("最密集的一天是 {peak_date}"),
        vec![
            item("日均事件", format!("{average:.1}")),
            item("高峰日期", peak_date.to_string()),
            item("高峰事件", peak_count.to_string()),
        ],
    )
}

fn streak_chapter(events: &[HistoryEvent]) -> AnnualReviewChapterV1 {
    if events.is_empty() {
        return omitted_chapter(
            "streak",
            "连续活跃",
            "没有连续活跃记录",
            "没有可用的观看历史",
        );
    }
    let mut dates = events.iter().map(|event| event.date).collect::<Vec<_>>();
    dates.sort_unstable();
    dates.dedup();
    let mut longest = 1_usize;
    let mut current = 1_usize;
    for pair in dates.windows(2) {
        if pair[1] == pair[0].succ_opt().expect("valid date") {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 1;
        }
    }
    chapter(
        "streak",
        "连续活跃",
        format!("最长连续观看 {longest} 天"),
        vec![item("最长连续天数", longest.to_string())],
    )
}

fn authors_chapter(events: &[HistoryEvent]) -> AnnualReviewChapterV1 {
    let mut counts = BTreeMap::<String, i64>::new();
    for event in events {
        if let Some(author) = event
            .author_name
            .as_deref()
            .map(str::trim)
            .filter(|author| !author.is_empty())
        {
            *counts.entry(author.to_string()).or_default() += 1;
        }
    }
    let items = counts
        .into_iter()
        .map(|(author, count)| item(&author, count.to_string()))
        .collect::<Vec<_>>();
    if items.is_empty() {
        omitted_chapter("authors", "作者", "没有可用的作者信息", "作者字段为空")
    } else {
        chapter("authors", "作者", "观看最多的作者", items)
    }
}

fn rewatch_chapter(events: &[HistoryEvent]) -> AnnualReviewChapterV1 {
    let mut counts = BTreeMap::<(i64, i64), (String, Option<String>, i64)>::new();
    for event in events {
        let title = event
            .title
            .clone()
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| "未命名内容".to_string());
        let entry = counts.entry((event.account_id, event.video_id)).or_insert((
            title,
            event.author_name.clone(),
            0,
        ));
        entry.2 += 1;
    }
    let items = counts
        .into_values()
        .filter(|(_, _, count)| *count >= 2)
        .map(|(title, author, count)| item_with_secondary(&title, count.to_string(), author))
        .collect::<Vec<_>>();
    if items.is_empty() {
        omitted_chapter(
            "rewatch",
            "重看",
            "这一年没有明显重看内容",
            "同一账号同一内容没有至少两次观看",
        )
    } else {
        chapter("rewatch", "重看", "看过不止一次的内容", items)
    }
}

fn progress_chapter(events: &[HistoryEvent]) -> AnnualReviewChapterV1 {
    if events.is_empty() {
        return omitted_chapter(
            "progress",
            "观看进度",
            "没有可分析的观看进度",
            "没有可用的观看历史",
        );
    }
    let mut buckets = [0_i64; 6];
    for event in events {
        match event.progress {
            None => buckets[5] += 1,
            Some(rate) if rate < 0.2 => buckets[0] += 1,
            Some(rate) if rate < 0.4 => buckets[1] += 1,
            Some(rate) if rate < 0.6 => buckets[2] += 1,
            Some(rate) if rate < 0.8 => buckets[3] += 1,
            Some(_) => buckets[4] += 1,
        }
    }
    let labels = ["0-19%", "20-39%", "40-59%", "60-79%", "80-100%", "未记录"];
    let items = labels
        .into_iter()
        .zip(buckets)
        .filter(|(_, count)| *count > 0)
        .map(|(label, count)| item(label, count.to_string()))
        .collect::<Vec<_>>();
    chapter("progress", "观看进度", "按五档进度保留缺失值", items)
}

fn archive_snapshot_chapter(snapshot: &BTreeMap<&'static str, i64>) -> AnnualReviewChapterV1 {
    chapter(
        "archiveSnapshot",
        "当前归档",
        "当前账号范围内仍保留的归档关系",
        vec![
            item(
                "观看历史",
                snapshot.get("history").copied().unwrap_or(0).to_string(),
            ),
            item(
                "喜欢",
                snapshot.get("likes").copied().unwrap_or(0).to_string(),
            ),
            item(
                "收藏",
                snapshot.get("favorites").copied().unwrap_or(0).to_string(),
            ),
        ],
    )
}

fn closing_chapter(events: &[HistoryEvent]) -> AnnualReviewChapterV1 {
    chapter(
        "closing",
        "规则结语",
        if events.is_empty() {
            "本年度没有观看历史，已保留空章节说明".to_string()
        } else {
            format!("本年度共记录 {} 次观看事件", events.len())
        },
        Vec::new(),
    )
}

fn chapter(
    id: &str,
    title: &str,
    summary: impl Into<String>,
    items: Vec<AnnualReviewItemV1>,
) -> AnnualReviewChapterV1 {
    AnnualReviewChapterV1 {
        id: id.to_string(),
        title: title.to_string(),
        summary: summary.into(),
        omitted: false,
        omitted_reason: None,
        items,
    }
}

fn omitted_chapter(id: &str, title: &str, summary: &str, reason: &str) -> AnnualReviewChapterV1 {
    AnnualReviewChapterV1 {
        id: id.to_string(),
        title: title.to_string(),
        summary: summary.to_string(),
        omitted: true,
        omitted_reason: Some(reason.to_string()),
        items: Vec::new(),
    }
}

fn item(label: &str, value: String) -> AnnualReviewItemV1 {
    AnnualReviewItemV1 {
        label: label.to_string(),
        value,
        secondary: None,
        rate: None,
    }
}

fn item_with_secondary(
    label: &str,
    value: String,
    secondary: Option<String>,
) -> AnnualReviewItemV1 {
    AnnualReviewItemV1 {
        label: label.to_string(),
        value,
        secondary,
        rate: None,
    }
}

fn question_marks(length: usize) -> String {
    std::iter::repeat("?")
        .take(length)
        .collect::<Vec<_>>()
        .join(", ")
}

fn parse_local_date(watched_date: &str, history_time_ms: i64) -> Result<NaiveDate, String> {
    if let Ok(date) = NaiveDate::parse_from_str(watched_date, "%Y-%m-%d") {
        return Ok(date);
    }
    shanghai_date(history_time_ms)
}

fn shanghai_date(timestamp_ms: i64) -> Result<NaiveDate, String> {
    FixedOffset::east_opt(SHANGHAI_OFFSET_SECONDS)
        .and_then(|offset| offset.timestamp_millis_opt(timestamp_ms).single())
        .map(|date| date.date_naive())
        .ok_or_else(|| "invalid_history_time_ms".to_string())
}
