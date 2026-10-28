#[path = "../build_support/frontend_manifest.rs"]
mod frontend_manifest;

use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[test]
fn accepts_a_complete_sorted_frontend_manifest() {
    let root = fixture_root();
    write_fixture(&root, false);
    assert_eq!(frontend_manifest::validate_frontend(&root, false), Ok(()));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_a_manifest_that_omits_a_required_input() {
    let root = fixture_root();
    write_fixture(&root, true);
    assert_eq!(
        frontend_manifest::validate_frontend(&root, false),
        Err("frontend manifest input set mismatch".into())
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_tampered_and_unexpected_dist_outputs() {
    let root = fixture_root();
    write_fixture(&root, false);
    fs::write(root.join("dist/index.html"), "tampered").unwrap();
    assert!(frontend_manifest::validate_frontend(&root, false)
        .unwrap_err()
        .starts_with("stale frontend output"));

    write_fixture(&root, false);
    fs::write(root.join("dist/unexpected.txt"), "unexpected").unwrap();
    assert_eq!(
        frontend_manifest::validate_frontend(&root, false),
        Err("invalid frontend output: unexpected.txt".into())
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_missing_manifest_changed_input_and_deleted_output() {
    let root = fixture_root();
    write_fixture(&root, false);
    fs::remove_file(root.join("dist/build-manifest.json")).unwrap();
    assert!(frontend_manifest::validate_frontend(&root, false).is_err());

    write_fixture(&root, false);
    fs::write(root.join("app/src/App.svelte"), "changed").unwrap();
    assert!(frontend_manifest::validate_frontend(&root, false)
        .unwrap_err()
        .starts_with("stale frontend input"));

    write_fixture(&root, false);
    fs::remove_file(root.join("dist/index.html")).unwrap();
    assert!(frontend_manifest::validate_frontend(&root, false).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_prohibited_dist_directories_and_source_maps() {
    let root = fixture_root();
    write_fixture(&root, false);
    fs::create_dir(root.join("dist/runtime")).unwrap();
    assert_eq!(
        frontend_manifest::validate_frontend(&root, false),
        Err("prohibited dist directory: runtime".into())
    );

    fs::remove_dir(root.join("dist/runtime")).unwrap();
    fs::create_dir(root.join("dist/collector")).unwrap();
    assert_eq!(
        frontend_manifest::validate_frontend(&root, false),
        Err("prohibited dist directory: collector".into())
    );

    fs::remove_dir(root.join("dist/collector")).unwrap();
    fs::write(root.join("dist/app.js.map"), "{}").unwrap();
    assert!(frontend_manifest::validate_frontend(&root, false)
        .unwrap_err()
        .starts_with("invalid frontend output"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_declared_node_browser_and_non_vite_outputs() {
    for output in [
        "bin/node.exe",
        "node_modules/runtime.js",
        "assets/playwright.exe",
        "assets/chromium.dll",
        "unexpected.txt",
    ] {
        let root = fixture_root();
        write_fixture(&root, false);
        declare_output(&root, output, b"binary");
        assert!(
            frontend_manifest::validate_frontend(&root, false)
                .unwrap_err()
                .starts_with("invalid frontend output"),
            "output {output}"
        );
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn rejects_noncanonical_manifest_output_paths() {
    let root = fixture_root();
    write_fixture(&root, false);
    declare_output(&root, "assets\\payload.js", b"script");

    assert_eq!(
        frontend_manifest::validate_frontend(&root, false),
        Err("unsafe manifest path".into())
    );

    fs::remove_dir_all(root).unwrap();
}

fn fixture_root() -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "emily-frontend-manifest-{}-{timestamp}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ))
}

fn write_fixture(root: &Path, omit_source: bool) {
    let files = [
        ("app/index.html", "<main></main>"),
        ("app/vite.config.ts", "export default {}"),
        ("app/src/App.svelte", "<main />"),
        ("package-lock.json", "{}"),
        ("package.json", "{}"),
        ("tsconfig.json", "{}"),
        ("dist/index.html", "<main></main>"),
    ];
    for (relative, contents) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    let mut inputs = files[..6]
        .iter()
        .map(|(path, _)| json!({"path": path, "sha256": hash(&root.join(path))}))
        .collect::<Vec<_>>();
    inputs.sort_by_key(|value| value["path"].as_str().unwrap().to_string());
    if omit_source {
        inputs.retain(|value| value["path"] != "app/src/App.svelte");
    }
    let output = root.join("dist/index.html");
    let manifest = json!({
        "schemaVersion": 1,
        "buildCommand": "npm run app:build",
        "tools": {
            "node": "24.15.0",
            "npm": "11.12.1",
            "vite": "7.3.7",
            "svelte": "5.56.4",
            "typescript": "5.9.3",
            "sveltePlugin": "6.2.4"
        },
        "inputs": inputs,
        "outputs": [{
            "path": "index.html",
            "bytes": fs::metadata(&output).unwrap().len(),
            "sha256": hash(&output)
        }]
    });
    fs::write(
        root.join("dist/build-manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
}

fn declare_output(root: &Path, relative: &str, contents: &[u8]) {
    let output = root.join("dist").join(relative);
    fs::create_dir_all(output.parent().unwrap()).unwrap();
    fs::write(&output, contents).unwrap();
    let manifest_path = root.join("dist/build-manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["outputs"].as_array_mut().unwrap().push(json!({
        "path": relative,
        "bytes": contents.len(),
        "sha256": hash(&output)
    }));
    manifest["outputs"]
        .as_array_mut()
        .unwrap()
        .sort_by_key(|value| value["path"].as_str().unwrap().to_string());
    fs::write(manifest_path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
}

fn hash(path: &Path) -> String {
    format!("{:x}", Sha256::digest(fs::read(path).unwrap()))
}
