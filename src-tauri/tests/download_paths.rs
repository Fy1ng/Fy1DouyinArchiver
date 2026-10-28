use std::path::{Path, PathBuf};

use douyin_history_archive::download::{
    asset_ownership_key, build_work_directory, contained_path, partial_filename,
    sanitize_component, DownloadCandidate, FileObservation, FilesystemRootAccess, RootAccess,
};

#[test]
fn guarded_partial_can_be_reopened_for_resume_without_path_fallbacks() {
    use std::{
        fs,
        io::{Read, Seek, SeekFrom, Write},
        time::{SystemTime, UNIX_EPOCH},
    };

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("emily-resume-path-{nonce}"));
    let relative = Path::new("author/work/.asset-7-42.mp4.part");
    fs::create_dir_all(root.join("author/work")).unwrap();
    let access = FilesystemRootAccess;

    let create_guard = access.guard_for_create(&root, relative).unwrap();
    let mut created = create_guard.create_new().unwrap();
    created.write_all(b"media-").unwrap();
    drop(created);
    drop(create_guard);

    let resume_guard = access.guard(&root, relative).unwrap();
    assert_eq!(
        resume_guard.inspect().unwrap(),
        FileObservation::File { len: 6 }
    );
    let mut resumed = resume_guard.open_existing_for_resume().unwrap();
    let mut prefix = Vec::new();
    resumed.read_to_end(&mut prefix).unwrap();
    assert_eq!(prefix, b"media-");
    resumed.seek(SeekFrom::End(0)).unwrap();
    resumed.write_all(b"tail").unwrap();
    resumed.flush().unwrap();
    drop(resumed);
    assert_eq!(
        resume_guard.inspect().unwrap(),
        FileObservation::File { len: 10 }
    );
    drop(resume_guard);

    assert_eq!(fs::read(root.join(relative)).unwrap(), b"media-tail");
    fs::remove_dir_all(root).unwrap();
}

fn candidate(
    author: Option<&str>,
    published: Option<&str>,
    title: Option<&str>,
) -> DownloadCandidate {
    DownloadCandidate {
        aweme_id: "1234567890".to_string(),
        canonical_url: "https://www.douyin.com/video/1234567890".to_string(),
        content_type: "video".to_string(),
        title: title.map(str::to_string),
        author_name: author.map(str::to_string),
        published_at: published.map(str::to_string),
        filter_context: None,
    }
}

#[test]
fn sanitizes_windows_reserved_names_invalid_characters_and_trailing_dots() {
    assert_eq!(sanitize_component("CON. "), "_CON");
    assert_eq!(
        sanitize_component("a<b>:c\"d/e\\f|g?h*"),
        "a_b_c_d_e_f_g_h_"
    );
    assert_eq!(sanitize_component("  hello\t\r\nworld...  "), "hello world");
    assert_eq!(sanitize_component("nul.txt"), "_nul.txt");
}

#[test]
fn work_directory_is_deterministic_bounded_and_always_contains_aweme_id() {
    let long = "标题😀".repeat(100);
    let long_author = "作者😀".repeat(100);
    let long_published = "2026-07-12时间".repeat(100);
    let path = build_work_directory(&candidate(
        Some(&long_author),
        Some(&long_published),
        Some(&long),
    ));
    let components: Vec<_> = path.components().collect();
    assert_eq!(components.len(), 2);
    assert!(path.to_string_lossy().contains("1234567890"));
    assert!(components.iter().all(|component| component
        .as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .count()
        <= 120));
    assert!(path.to_string_lossy().encode_utf16().count() <= 220);

    assert_eq!(
        build_work_directory(&candidate(None, None, None)),
        PathBuf::from("未知作者").join("未知时间_无标题_1234567890")
    );
}

#[test]
fn contained_paths_reject_absolute_and_parent_traversal() {
    let root = Path::new("C:/archive");
    assert!(contained_path(root, Path::new("author/work/video.mp4")).is_ok());
    assert!(contained_path(root, Path::new("../outside.mp4")).is_err());
    assert!(contained_path(root, Path::new("author/../../outside.mp4")).is_err());
    assert!(contained_path(root, Path::new("D:/outside.mp4")).is_err());
}

#[test]
fn ownership_keys_include_normalized_root_identity_and_partial_names_are_unique() {
    let key = asset_ownership_key(
        Path::new("C:\\Archive\\."),
        "123",
        "video:primary",
        "highest",
    );
    assert_eq!(
        key,
        asset_ownership_key(Path::new("c:/archive"), "123", "video:primary", "highest")
    );
    assert_ne!(
        key,
        asset_ownership_key(Path::new("c:/archive"), "124", "video:primary", "highest")
    );
    assert_ne!(
        key,
        asset_ownership_key(Path::new("c:/archive"), "123", "video:primary", "balanced")
    );
    assert_ne!(
        partial_filename(1, 10, "mp4"),
        partial_filename(2, 10, "mp4")
    );
    assert_ne!(
        partial_filename(1, 10, "mp4"),
        partial_filename(1, 11, "mp4")
    );
    assert_eq!(partial_filename(7, 42, ".MP4"), ".asset-7-42.mp4.part");
}

#[cfg(windows)]
#[test]
fn windows_native_guards_create_inspect_rename_and_delete_without_path_fallbacks() {
    use std::{
        fs,
        io::Write,
        time::{SystemTime, UNIX_EPOCH},
    };

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("emily-guarded-path-{nonce}"));
    fs::create_dir(&root).unwrap();
    let access = FilesystemRootAccess;
    let partial = Path::new("author/work/.asset-7-42.mp4.part");
    let final_path = Path::new("author/work/video.mp4");

    let guard = access.guard_for_create(&root, partial).unwrap();
    let mut file = guard.create_new().unwrap();
    file.write_all(b"media").unwrap();
    drop(file);
    assert_eq!(guard.inspect().unwrap(), FileObservation::File { len: 5 });
    drop(guard);

    access.atomic_rename(&root, partial, final_path).unwrap();
    let final_guard = access.guard(&root, final_path).unwrap();
    assert_eq!(
        final_guard.inspect().unwrap(),
        FileObservation::File { len: 5 }
    );
    final_guard.remove().unwrap();
    drop(final_guard);
    assert!(!root.join(final_path).exists());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn windows_guard_lifetime_blocks_parent_and_partial_file_moves() {
    use std::{
        fs,
        io::Write,
        time::{SystemTime, UNIX_EPOCH},
    };

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("emily-guarded-move-{nonce}"));
    fs::create_dir(&root).unwrap();
    let access = FilesystemRootAccess;
    let partial = Path::new("author/work/.asset-7-42.mp4.part");
    let guard = access.guard_for_create(&root, partial).unwrap();

    assert!(
        fs::rename(root.join("author"), root.join("moved-author")).is_err(),
        "a guarded ancestor directory remained movable"
    );

    let mut file = guard.create_new().unwrap();
    file.write_all(b"media").unwrap();
    assert!(
        fs::rename(root.join(partial), root.join("author/work/moved.part")).is_err(),
        "an open guarded partial remained movable"
    );

    drop(file);
    drop(guard);
    fs::remove_dir_all(root).unwrap();
}
