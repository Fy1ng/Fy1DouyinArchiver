use std::{
    fs,
    io::Cursor,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use douyin_history_archive::{
    download::{
        parse_detail_response, parse_manifest, partial_filename, read_frame, reconcile_manifest,
        write_frame, AssetAction, DownloadQuality, ResolverControl, ResolverRequest,
        ResolverResponse,
    },
    migrate_schema_to_latest_at_startup,
};
use rusqlite::Connection;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

static STORAGE_ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const TEST_PROFILE_KEY: &str = "0123456789abcdef0123456789abcdef";

fn seed_account(connection: &Connection) {
    connection.execute("INSERT INTO accounts (uid, profile_key, status, created_at, updated_at) VALUES ('download-resolver', ?1, 'active', 'now', 'now')", [TEST_PROFILE_KEY]).unwrap();
    let account_id = connection.last_insert_rowid();
    connection.execute("INSERT INTO remote_folder_registry_state(account_id, revision) VALUES (?1, 1)", [account_id]).unwrap();
}

fn fixture(name: &str) -> Value {
    let source = match name {
        "video" => include_str!("fixtures/download/video_variants.json"),
        "image" => include_str!("fixtures/download/image_music.json"),
        "live" => include_str!("fixtures/download/live_pairs.json"),
        "missing" => include_str!("fixtures/download/missing_fields.json"),
        "unsupported" => include_str!("fixtures/download/unsupported.json"),
        "expiring" => include_str!("fixtures/download/expiring_metadata.json"),
        "generation1" => include_str!("fixtures/download/generation1.json"),
        "generation2" => include_str!("fixtures/download/generation2.json"),
        _ => panic!("unknown fixture"),
    };
    serde_json::from_str(source).unwrap()
}

fn storage_root(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "emily-{label}-{}-{nonce}-{}",
        std::process::id(),
        STORAGE_ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ))
}

fn single_image_manifest(aweme_id: &str, size: usize) -> Value {
    json!({
        "awemeId": aweme_id,
        "contentType": "image",
        "images": [{
            "sourceUrl": "https://media.example/image.jpg",
            "mimeType": "image/jpeg",
            "width": 1080,
            "height": 1440,
            "expectedSize": size
        }]
    })
}

#[test]
fn upstream_detail_adapter_accepts_nested_realistic_video_response() {
    let value: Value =
        serde_json::from_str(include_str!("fixtures/download/douyin_detail_video.json")).unwrap();

    let manifest =
        parse_detail_response(value, "7600224486650121999", DownloadQuality::Balanced).unwrap();

    assert_eq!(manifest.aweme_id, "7600224486650121999");
    assert_eq!(manifest.content_type, "video");
    assert_eq!(manifest.assets.len(), 1);
    assert_eq!(manifest.assets[0].stable_key, "video:primary");
    assert_eq!(
        manifest.assets[0].actual_quality.as_deref(),
        Some("720p_h264")
    );
    assert_eq!(manifest.assets[0].expected_size, Some(111));
}

#[test]
fn upstream_detail_adapter_maps_gallery_live_pairs_and_music() {
    let value = json!({
        "status_code": 0,
        "aweme_detail": {
            "aweme_id": "7600224486650122000",
            "aweme_type": 68,
            "image_post_info": {
                "images": [
                    {
                        "display_image": {
                            "url_list": ["https://p.example.com/1.webp"],
                            "width": 1080,
                            "height": 1440,
                            "data_size": 51
                        },
                        "video": {
                            "play_addr": {
                                "url_list": ["https://v.example.com/live-1.mp4"],
                                "width": 1080,
                                "height": 1440,
                                "data_size": 61
                            }
                        }
                    },
                    {
                        "download_url_list": ["https://p.example.com/2.jpeg"],
                        "width": 720,
                        "height": 960
                    }
                ]
            },
            "music": {
                "play_url": {
                    "url_list": ["https://a.example.com/music.mp3"],
                    "data_size": 71
                }
            },
            "extra_unknown_upstream_field": { "safe": true }
        },
        "extra": { "logid": "ignored" }
    });

    let manifest =
        parse_detail_response(value, "7600224486650122000", DownloadQuality::Highest).unwrap();

    assert_eq!(manifest.content_type, "live_photo");
    assert_eq!(
        manifest
            .assets
            .iter()
            .map(|asset| asset.stable_key.as_str())
            .collect::<Vec<_>>(),
        [
            "live-image:0001",
            "live-video:0001",
            "live-image:0002",
            "audio:music"
        ]
    );
}

#[test]
fn upstream_detail_adapter_rejects_a_different_aweme_identity() {
    let value: Value =
        serde_json::from_str(include_str!("fixtures/download/douyin_detail_video.json")).unwrap();
    assert_eq!(
        parse_detail_response(value, "7600224486650122999", DownloadQuality::Highest)
            .unwrap_err()
            .code,
        "download_manifest_identity_mismatch"
    );
}

#[test]
fn parser_selects_video_quality_and_stable_key() {
    let manifest = parse_manifest(fixture("video"), DownloadQuality::Balanced).unwrap();
    assert_eq!(manifest.content_type, "video");
    assert_eq!(manifest.assets.len(), 1);
    assert_eq!(manifest.assets[0].stable_key, "video:primary");
    assert_eq!(
        manifest.assets[0].actual_quality.as_deref(),
        Some("1080-h264")
    );
}

#[test]
fn parser_emits_images_music_and_paired_live_assets_in_order() {
    let image = parse_manifest(fixture("image"), DownloadQuality::Highest).unwrap();
    assert_eq!(
        image
            .assets
            .iter()
            .map(|a| a.stable_key.as_str())
            .collect::<Vec<_>>(),
        ["image:0001", "image:0002", "audio:music"]
    );

    let live = parse_manifest(fixture("live"), DownloadQuality::Highest).unwrap();
    assert_eq!(
        live.assets
            .iter()
            .map(|a| a.stable_key.as_str())
            .collect::<Vec<_>>(),
        [
            "live-image:0001",
            "live-video:0001",
            "live-image:0002",
            "live-video:0002"
        ]
    );
}

#[test]
fn parser_accepts_live_photo_with_optional_music() {
    let mut value = fixture("live");
    value["music"] = json!({
        "sourceUrl": "https://media.example/live-music.mp3",
        "mimeType": "audio/mpeg",
        "expectedSize": 55
    });

    let manifest = parse_manifest(value, DownloadQuality::Highest).unwrap();
    assert_eq!(
        manifest
            .assets
            .iter()
            .map(|asset| asset.stable_key.as_str())
            .collect::<Vec<_>>(),
        [
            "live-image:0001",
            "live-video:0001",
            "live-image:0002",
            "live-video:0002",
            "audio:music"
        ]
    );
}

#[test]
fn parser_rejects_missing_and_unsupported_media() {
    assert_eq!(
        parse_manifest(fixture("missing"), DownloadQuality::Highest)
            .unwrap_err()
            .code,
        "download_manifest_invalid"
    );
    assert_eq!(
        parse_manifest(fixture("unsupported"), DownloadQuality::Highest)
            .unwrap_err()
            .code,
        "download_unsupported_content"
    );
}

#[test]
fn parser_requires_exact_content_type_and_its_exclusive_media_shape() {
    let unknown_with_images = json!({
        "awemeId": "7010",
        "contentType": "story",
        "images": [{ "sourceUrl": "https://media.example/1.jpg", "mimeType": "image/jpeg" }]
    });
    assert_eq!(
        parse_manifest(unknown_with_images, DownloadQuality::Highest)
            .unwrap_err()
            .code,
        "download_unsupported_content"
    );

    let mixed_cases = [
        json!({
            "awemeId": "7011", "contentType": "video",
            "video": { "formats": [{ "id": "v", "width": 1920, "height": 1080, "bitrate": 1, "codec": "h264", "mimeType": "video/mp4", "expectedSize": 1, "sourceUrl": "https://media.example/v", "requestHeaders": {}, "sourceExpiresAt": null }] },
            "music": { "sourceUrl": "https://media.example/m.mp3", "mimeType": "audio/mpeg" }
        }),
        json!({
            "awemeId": "7012", "contentType": "image",
            "images": [{ "sourceUrl": "https://media.example/1.jpg", "mimeType": "image/jpeg" }],
            "video": { "formats": [{ "id": "v", "width": 1920, "height": 1080, "bitrate": 1, "codec": "h264", "mimeType": "video/mp4", "expectedSize": 1, "sourceUrl": "https://media.example/v", "requestHeaders": {}, "sourceExpiresAt": null }] }
        }),
        json!({
            "awemeId": "7013", "contentType": "live_photo",
            "livePhotos": [{
                "image": { "sourceUrl": "https://media.example/live.jpg", "mimeType": "image/jpeg" },
                "video": { "formats": [{ "id": "v", "width": 1080, "height": 1440, "bitrate": 1, "codec": "h264", "mimeType": "video/mp4", "expectedSize": 1, "sourceUrl": "https://media.example/live.mp4", "requestHeaders": {}, "sourceExpiresAt": null }] }
            }],
            "images": [{ "sourceUrl": "https://media.example/extra.jpg", "mimeType": "image/jpeg" }]
        }),
    ];
    for mixed in mixed_cases {
        assert_eq!(
            parse_manifest(mixed, DownloadQuality::Highest)
                .unwrap_err()
                .code,
            "download_manifest_invalid"
        );
    }
}

#[test]
fn expiring_request_metadata_stays_in_manifest_memory() {
    let manifest = parse_manifest(fixture("expiring"), DownloadQuality::Highest).unwrap();
    let asset = &manifest.assets[0];
    assert!(asset.source_url.contains("token=secret"));
    assert_eq!(asset.request_headers["Cookie"], "session=secret");
    assert_eq!(
        asset.source_expires_at.as_deref(),
        Some("2026-07-12T12:00:00Z")
    );
}

fn seeded_item() -> (Connection, i64) {
    let mut connection = Connection::open_in_memory().unwrap();
    migrate_schema_to_latest_at_startup(&mut connection).unwrap();
    seed_account(&connection);
    connection.execute("INSERT INTO download_jobs (account_id, profile_key, created_at, selection_scope, destination_root, requested_quality, state, candidate_count) VALUES (1, ?1, 'now','selected','C:/downloads','balanced','running',1)", [TEST_PROFILE_KEY]).unwrap();
    let job_id = connection.last_insert_rowid();
    connection.execute("INSERT INTO download_items (job_id, aweme_id, canonical_url, content_type, relative_work_directory, state, created_at, updated_at) VALUES (?1,'7007','https://www.douyin.com/video/7007','image','work/7007','resolving','now','now')", [job_id]).unwrap();
    let item_id = connection.last_insert_rowid();
    (connection, item_id)
}

#[test]
fn reconciliation_advances_generation_without_persisting_request_secrets() {
    let (mut connection, item_id) = seeded_item();
    let manifest = parse_manifest(fixture("expiring"), DownloadQuality::Balanced).unwrap();
    connection
        .execute(
            "UPDATE download_items SET aweme_id='7006' WHERE id=?1",
            [item_id],
        )
        .unwrap();
    reconcile_manifest(&mut connection, item_id, &manifest).unwrap();

    let columns = connection
        .prepare("PRAGMA table_info(download_assets)")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(!columns.iter().any(|name| name == "source_url"
        || name == "request_headers"
        || name == "source_expires_at"));
    let generation: i64 = connection
        .query_row(
            "SELECT manifest_generation FROM download_items WHERE id=?1",
            [item_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(generation, 1);
}

#[test]
fn reconciliation_retains_safe_completed_progress_and_supersedes_disappeared_assets() {
    let (mut connection, item_id) = seeded_item();
    let first = parse_manifest(fixture("generation1"), DownloadQuality::Balanced).unwrap();
    reconcile_manifest(&mut connection, item_id, &first).unwrap();
    connection.execute("UPDATE download_assets SET state='completed', downloaded_bytes=expected_size, validated_size=expected_size, validated_sha256='abc', validated_at='now', completed_at='now' WHERE item_id=?1 AND stable_key='image:0001'", [item_id]).unwrap();
    connection.execute("UPDATE download_assets SET state='downloading', downloaded_bytes=77, etag='old' WHERE item_id=?1 AND stable_key='audio:music'", [item_id]).unwrap();

    let mut second = parse_manifest(fixture("generation2"), DownloadQuality::Balanced).unwrap();
    second.assets[0]
        .request_headers
        .insert("Authorization".into(), "Bearer refreshed".into());
    second.assets[0].source_expires_at = Some("2026-07-13T12:00:00Z".into());
    let actions = reconcile_manifest(&mut connection, item_id, &second).unwrap();

    assert!(actions.iter().any(|action| matches!(action, AssetAction::Retained { stable_key, .. } if stable_key == "image:0001")));
    assert!(actions.iter().any(|action| matches!(action, AssetAction::Superseded { stable_key, .. } if stable_key == "audio:music")));
    assert_eq!(connection.query_row("SELECT state FROM download_assets WHERE item_id=?1 AND stable_key='image:0001' AND manifest_generation=2", [item_id], |r| r.get::<_, String>(0)).unwrap(), "completed");
    assert_eq!(connection.query_row("SELECT downloaded_bytes FROM download_assets WHERE item_id=?1 AND stable_key='image:0001' AND manifest_generation=2", [item_id], |r| r.get::<_, i64>(0)).unwrap(), 100);
    assert_eq!(connection.query_row("SELECT state FROM download_assets WHERE item_id=?1 AND stable_key='audio:music' AND manifest_generation=1", [item_id], |r| r.get::<_, String>(0)).unwrap(), "superseded");
    assert_eq!(connection.query_row("SELECT COUNT(*) FROM download_assets WHERE item_id=?1 AND manifest_generation=2 AND required=1", [item_id], |r| r.get::<_, i64>(0)).unwrap(), 3);
    assert_eq!(
        connection
            .query_row(
                "SELECT total_required_asset_count FROM download_items WHERE id=?1",
                [item_id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        3
    );
}

#[test]
fn reconciliation_queues_completed_assets_when_any_persisted_identity_field_changes() {
    type Mutation = fn(&mut douyin_history_archive::download::ResolvedAsset, &Connection, i64);
    let cases: [(&str, Mutation); 10] = [
        ("asset_type", |asset, _, _| {
            asset.asset_type = "live-image".into()
        }),
        ("ordinal", |asset, _, _| asset.ordinal = 9),
        ("actual_quality", |asset, _, _| {
            asset.actual_quality = Some("other".into())
        }),
        ("width", |asset, _, _| asset.width = Some(720)),
        ("height", |asset, _, _| asset.height = Some(720)),
        ("bitrate", |asset, _, _| asset.bitrate = Some(99)),
        ("codec", |asset, _, _| asset.codec = Some("h265".into())),
        ("mime_type", |asset, _, _| {
            asset.mime_type = Some("image/png".into())
        }),
        ("expected_size", |asset, _, _| {
            asset.expected_size = Some(101)
        }),
        ("requested_profile", |_, connection, item_id| {
            connection.execute("UPDATE download_jobs SET requested_quality='highest' WHERE id=(SELECT job_id FROM download_items WHERE id=?1)", [item_id]).unwrap();
        }),
    ];

    for (name, mutate) in cases {
        let (mut connection, item_id) = seeded_item();
        let first = parse_manifest(fixture("generation1"), DownloadQuality::Balanced).unwrap();
        reconcile_manifest(&mut connection, item_id, &first).unwrap();
        connection.execute("UPDATE download_assets SET state='completed', downloaded_bytes=expected_size, validated_size=expected_size, validated_sha256='abc', validated_at='now', completed_at='now' WHERE item_id=?1 AND stable_key='image:0001'", [item_id]).unwrap();

        let mut second = first.clone();
        mutate(&mut second.assets[0], &connection, item_id);
        reconcile_manifest(&mut connection, item_id, &second).unwrap();

        let (state, downloaded, validated): (String, i64, Option<String>) = connection.query_row(
            "SELECT state, downloaded_bytes, validated_sha256 FROM download_assets WHERE item_id=?1 AND stable_key='image:0001' AND manifest_generation=2",
            [item_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap();
        assert_eq!(state, "queued", "{name}");
        assert_eq!(downloaded, 0, "{name}");
        assert_eq!(validated, None, "{name}");
    }
}

#[test]
fn reconciliation_rejects_completed_progress_beyond_validated_size_when_size_is_unknown() {
    let (mut connection, item_id) = seeded_item();
    let mut manifest = parse_manifest(fixture("generation1"), DownloadQuality::Balanced).unwrap();
    manifest.assets[0].expected_size = None;
    reconcile_manifest(&mut connection, item_id, &manifest).unwrap();
    connection.execute(
        "UPDATE download_assets SET state='completed', downloaded_bytes=101, validated_size=100, validated_sha256='abc', validated_at='now', completed_at='now' WHERE item_id=?1 AND stable_key='image:0001'",
        [item_id],
    ).unwrap();

    reconcile_manifest(&mut connection, item_id, &manifest).unwrap();

    let (state, downloaded, validated): (String, i64, Option<String>) = connection
        .query_row(
            "SELECT state, downloaded_bytes, validated_sha256 FROM download_assets WHERE item_id=?1 AND stable_key='image:0001' AND manifest_generation=2",
            [item_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(state, "queued");
    assert_eq!(downloaded, 0);
    assert_eq!(validated, None);
}

#[test]
fn reconciliation_skips_a_valid_cross_job_file_and_redownloads_it_after_corruption() {
    let root = storage_root("historical-reuse");
    let work = PathBuf::from("author/work");
    fs::create_dir_all(root.join(&work).join("images")).unwrap();
    let bytes = b"valid-image-bytes";
    fs::write(root.join(&work).join("images/001.jpg"), bytes).unwrap();
    let digest = format!("{:x}", Sha256::digest(bytes));
    let manifest = parse_manifest(
        single_image_manifest("7008", bytes.len()),
        DownloadQuality::Balanced,
    )
    .unwrap();

    let mut connection = Connection::open_in_memory().unwrap();
    migrate_schema_to_latest_at_startup(&mut connection).unwrap();
    seed_account(&connection);
    connection.execute("INSERT INTO download_jobs (account_id,profile_key,created_at,selection_scope,destination_root,requested_quality,state,candidate_count) VALUES (1,?1,'now','selected',?2,'balanced','completed',1)", rusqlite::params![TEST_PROFILE_KEY, root.to_string_lossy().as_ref()]).unwrap();
    let first_job = connection.last_insert_rowid();
    connection.execute("INSERT INTO download_items (job_id,aweme_id,canonical_url,content_type,relative_work_directory,state,manifest_generation,completed_asset_count,total_required_asset_count,created_at,updated_at,completed_at) VALUES (?1,'7008','https://www.douyin.com/note/7008','image',?2,'completed',1,1,1,'now','now','now')", rusqlite::params![first_job, work.to_string_lossy()]).unwrap();
    let first_item = connection.last_insert_rowid();
    let asset = &manifest.assets[0];
    connection.execute("INSERT INTO download_assets (item_id,stable_key,asset_type,ordinal,requested_profile,actual_quality,width,height,bitrate,codec,mime_type,relative_final_path,relative_part_path,expected_size,downloaded_bytes,validated_size,validated_sha256,validated_at,manifest_generation,required,state,created_at,updated_at,completed_at) VALUES (?1,?2,?3,?4,'balanced',?5,?6,?7,?8,?9,?10,'images/001.jpg','.old.part',?11,?11,?11,?12,'now',1,1,'completed','now','now','now')", rusqlite::params![first_item, asset.stable_key, asset.asset_type, asset.ordinal, asset.actual_quality, asset.width, asset.height, asset.bitrate, asset.codec, asset.mime_type, bytes.len() as i64, digest]).unwrap();

    let new_item = |connection: &Connection| {
        connection.execute("INSERT INTO download_jobs (account_id,profile_key,created_at,selection_scope,destination_root,requested_quality,state,candidate_count) VALUES (1,?1,'later','selected',?2,'balanced','running',1)", rusqlite::params![TEST_PROFILE_KEY, root.to_string_lossy().as_ref()]).unwrap();
        let job = connection.last_insert_rowid();
        connection.execute("INSERT INTO download_items (job_id,aweme_id,canonical_url,content_type,relative_work_directory,state,created_at,updated_at) VALUES (?1,'7008','https://www.douyin.com/note/7008','image',?2,'resolving','now','now')", rusqlite::params![job, work.to_string_lossy()]).unwrap();
        connection.last_insert_rowid()
    };

    let second_item = new_item(&connection);
    reconcile_manifest(&mut connection, second_item, &manifest).unwrap();
    let (asset_state, item_state): (String, String) = connection.query_row("SELECT a.state,i.state FROM download_assets a JOIN download_items i ON i.id=a.item_id WHERE a.item_id=?1 AND a.required=1", [second_item], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
    assert_eq!(
        (asset_state.as_str(), item_state.as_str()),
        ("skipped", "skipped")
    );

    fs::write(
        root.join(&work).join("images/001.jpg"),
        b"corrupt-image-data",
    )
    .unwrap();
    let third_item = new_item(&connection);
    reconcile_manifest(&mut connection, third_item, &manifest).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT state FROM download_assets WHERE item_id=?1 AND required=1",
                [third_item],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "queued"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn reconciliation_keeps_a_compatible_persisted_partial_on_the_same_asset_row() {
    let root = storage_root("partial-generation");
    let work = PathBuf::from("author/work");
    fs::create_dir_all(root.join(&work).join("images")).unwrap();
    let manifest =
        parse_manifest(single_image_manifest("7009", 10), DownloadQuality::Balanced).unwrap();
    let asset = &manifest.assets[0];
    let mut connection = Connection::open_in_memory().unwrap();
    migrate_schema_to_latest_at_startup(&mut connection).unwrap();
    seed_account(&connection);
    connection.execute("INSERT INTO download_jobs (account_id,profile_key,created_at,selection_scope,destination_root,requested_quality,state,candidate_count) VALUES (1,?1,'now','selected',?2,'balanced','running',1)", rusqlite::params![TEST_PROFILE_KEY, root.to_string_lossy().as_ref()]).unwrap();
    let job = connection.last_insert_rowid();
    connection.execute("INSERT INTO download_items (job_id,aweme_id,canonical_url,content_type,relative_work_directory,state,manifest_generation,created_at,updated_at) VALUES (?1,'7009','https://www.douyin.com/note/7009','image',?2,'resolving',1,'now','now')", rusqlite::params![job, work.to_string_lossy()]).unwrap();
    let item = connection.last_insert_rowid();
    connection.execute("INSERT INTO download_assets (item_id,stable_key,asset_type,ordinal,requested_profile,actual_quality,width,height,bitrate,codec,mime_type,relative_final_path,relative_part_path,expected_size,downloaded_bytes,etag,manifest_generation,required,state,created_at,updated_at) VALUES (?1,?2,?3,?4,'balanced',?5,?6,?7,?8,?9,?10,'images/001.jpg','pending.part',10,4,'\"v1\"',1,1,'queued','now','now')", rusqlite::params![item, asset.stable_key, asset.asset_type, asset.ordinal, asset.actual_quality, asset.width, asset.height, asset.bitrate, asset.codec, asset.mime_type]).unwrap();
    let asset_id = connection.last_insert_rowid();
    let part = partial_filename(asset_id, 1, "jpg");
    connection
        .execute(
            "UPDATE download_assets SET relative_part_path=?2 WHERE id=?1",
            rusqlite::params![asset_id, part],
        )
        .unwrap();
    fs::write(root.join(&work).join(&part), b"part").unwrap();

    reconcile_manifest(&mut connection, item, &manifest).unwrap();
    let row: (i64, i64, String, i64, String) = connection.query_row("SELECT id,manifest_generation,state,downloaded_bytes,relative_part_path FROM download_assets WHERE item_id=?1 AND required=1", [item], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))).unwrap();
    assert_eq!(row, (asset_id, 2, "queued".into(), 4, part));
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM download_assets WHERE item_id=?1",
                [item],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn ipc_round_trips_strict_length_delimited_json() {
    let request = ResolverRequest {
        request_id: 9,
        account_id: 1,
        profile_key: "0123456789abcdef0123456789abcdef".into(),
        aweme_id: "7001".into(),
        canonical_url: "https://www.douyin.com/video/7001".into(),
        quality: DownloadQuality::Balanced,
    };
    let mut bytes = Vec::new();
    write_frame(&mut bytes, &request).unwrap();
    assert_eq!(
        u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize,
        bytes.len() - 4
    );
    assert_eq!(
        read_frame::<_, ResolverRequest>(&mut Cursor::new(bytes)).unwrap(),
        request
    );

    let response = ResolverResponse::Error {
        request_id: 9,
        code: "download_unsupported_content".into(),
        message: "不支持".into(),
    };
    let mut bytes = Vec::new();
    write_frame(&mut bytes, &response).unwrap();
    assert_eq!(
        read_frame::<_, ResolverResponse>(&mut Cursor::new(bytes)).unwrap(),
        response
    );

    let mut bytes = Vec::new();
    write_frame(&mut bytes, &ResolverControl::Cancel).unwrap();
    assert_eq!(
        read_frame::<_, ResolverControl>(&mut Cursor::new(bytes)).unwrap(),
        ResolverControl::Cancel
    );
}

#[test]
fn ipc_rejects_oversized_invalid_utf8_and_unknown_fields() {
    let mut oversized = Cursor::new(((8 * 1024 * 1024 + 1) as u32).to_le_bytes().to_vec());
    assert_eq!(
        read_frame::<_, ResolverRequest>(&mut oversized)
            .unwrap_err()
            .code,
        "resolver_frame_too_large"
    );

    let mut invalid = Vec::from((2_u32).to_le_bytes());
    invalid.extend_from_slice(&[0xff, 0xff]);
    assert_eq!(
        read_frame::<_, ResolverRequest>(&mut Cursor::new(invalid))
            .unwrap_err()
            .code,
        "resolver_frame_invalid_utf8"
    );

    let payload = serde_json::to_vec(&json!({"requestId":1,"awemeId":"1","canonicalUrl":"https://example/1","quality":"highest","secret":"no"})).unwrap();
    let mut unknown = Vec::from((payload.len() as u32).to_le_bytes());
    unknown.extend(payload);
    assert_eq!(
        read_frame::<_, ResolverRequest>(&mut Cursor::new(unknown))
            .unwrap_err()
            .code,
        "resolver_frame_invalid_json"
    );
}
