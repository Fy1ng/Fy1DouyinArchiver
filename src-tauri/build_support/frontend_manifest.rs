use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path, PathBuf},
};
use walkdir::WalkDir;

const FIXED_INPUTS: [&str; 5] = [
    "app/index.html",
    "app/vite.config.ts",
    "package-lock.json",
    "package.json",
    "tsconfig.json",
];

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    build_command: String,
    tools: Tools,
    inputs: Vec<Input>,
    outputs: Vec<Output>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Tools {
    node: String,
    npm: String,
    vite: String,
    svelte: String,
    typescript: String,
    svelte_plugin: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    path: String,
    sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Output {
    path: String,
    bytes: u64,
    sha256: String,
}

pub fn validate_frontend(root: &Path, emit_cargo_directives: bool) -> Result<(), String> {
    let dist = root.join("dist");
    let manifest_path = dist.join("build-manifest.json");
    rerun(&manifest_path, emit_cargo_directives);
    reject_symlink(&manifest_path)?;

    let manifest: Manifest =
        serde_json::from_slice(&fs::read(&manifest_path).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    validate_header(&manifest)?;

    let expected_inputs = discover_inputs(root)?;
    let input_paths = manifest
        .inputs
        .iter()
        .map(|input| input.path.as_str())
        .collect::<Vec<_>>();
    ensure_sorted_unique(&input_paths, "inputs")?;
    let declared_inputs = manifest
        .inputs
        .iter()
        .map(|input| safe_relative(&input.path))
        .collect::<Result<BTreeSet<_>, _>>()?;
    if declared_inputs != expected_inputs {
        return Err("frontend manifest input set mismatch".into());
    }
    for input in &manifest.inputs {
        let absolute = root.join(safe_relative(&input.path)?);
        rerun(&absolute, emit_cargo_directives);
        validate_regular_file(&absolute)?;
        if sha(&absolute)? != input.sha256 {
            return Err(format!("stale frontend input: {}", input.path));
        }
    }

    reject_dist_directories(&dist)?;
    let output_paths = manifest
        .outputs
        .iter()
        .map(|output| output.path.as_str())
        .collect::<Vec<_>>();
    ensure_sorted_unique(&output_paths, "outputs")?;
    let mut declared_outputs = BTreeSet::new();
    for output in &manifest.outputs {
        let relative = safe_relative(&output.path)?;
        if prohibited_output(&relative) || !declared_outputs.insert(relative.clone()) {
            return Err(format!("invalid frontend output: {}", output.path));
        }
        let absolute = dist.join(&relative);
        rerun(&absolute, emit_cargo_directives);
        validate_regular_file(&absolute)?;
        let metadata = fs::metadata(&absolute).map_err(|error| error.to_string())?;
        if metadata.len() != output.bytes || sha(&absolute)? != output.sha256 {
            return Err(format!("stale frontend output: {}", output.path));
        }
    }

    let actual_outputs = discover_outputs(&dist)?;
    if actual_outputs != declared_outputs {
        return Err("unexpected or missing dist files".into());
    }
    Ok(())
}

fn validate_header(manifest: &Manifest) -> Result<(), String> {
    if manifest.schema_version != 1 || manifest.build_command != "npm run app:build" {
        return Err("unsupported manifest schema or command".into());
    }
    let expected = [
        (&manifest.tools.node, "24.15.0"),
        (&manifest.tools.npm, "11.12.1"),
        (&manifest.tools.vite, "7.3.6"),
        (&manifest.tools.svelte, "5.56.4"),
        (&manifest.tools.typescript, "5.9.3"),
        (&manifest.tools.svelte_plugin, "6.2.4"),
    ];
    if expected
        .iter()
        .any(|(actual, expected)| actual.as_str() != *expected)
    {
        return Err("unsupported frontend tool version".into());
    }
    Ok(())
}

fn discover_inputs(root: &Path) -> Result<BTreeSet<PathBuf>, String> {
    let mut inputs = FIXED_INPUTS
        .iter()
        .map(PathBuf::from)
        .collect::<BTreeSet<_>>();
    let source = root.join("app/src");
    for entry in WalkDir::new(&source) {
        let entry = entry.map_err(|error| error.to_string())?;
        if entry.path_is_symlink() {
            return Err(format!(
                "frontend input symlink: {}",
                entry.path().display()
            ));
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let extension = entry.path().extension().and_then(|value| value.to_str());
        if matches!(extension, Some("svelte" | "ts" | "css")) {
            inputs.insert(
                entry
                    .path()
                    .strip_prefix(root)
                    .map_err(|error| error.to_string())?
                    .to_path_buf(),
            );
        }
    }
    for input in &inputs {
        validate_regular_file(&root.join(input))?;
    }
    Ok(inputs)
}

fn discover_outputs(dist: &Path) -> Result<BTreeSet<PathBuf>, String> {
    let mut outputs = BTreeSet::new();
    for entry in WalkDir::new(dist) {
        let entry = entry.map_err(|error| error.to_string())?;
        if entry.path_is_symlink() {
            return Err(format!(
                "frontend output symlink: {}",
                entry.path().display()
            ));
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(dist)
            .map_err(|error| error.to_string())?
            .to_path_buf();
        if relative != Path::new("build-manifest.json") {
            if prohibited_output(&relative) {
                return Err(format!("invalid frontend output: {}", relative.display()));
            }
            outputs.insert(relative);
        }
    }
    Ok(outputs)
}

fn reject_dist_directories(dist: &Path) -> Result<(), String> {
    for name in ["runtime", "collector"] {
        if dist.join(name).exists() {
            return Err(format!("prohibited dist directory: {name}"));
        }
    }
    Ok(())
}

fn ensure_sorted_unique(values: &[&str], label: &str) -> Result<(), String> {
    if values.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(format!(
            "frontend manifest {label} must be sorted and unique"
        ));
    }
    Ok(())
}

fn safe_relative(value: &str) -> Result<PathBuf, String> {
    if value.contains('\\')
        || value.split('/').any(|component| {
            component.is_empty() || component == "." || component == ".."
        })
    {
        return Err("unsafe manifest path".into());
    }
    let path = PathBuf::from(value);
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("unsafe manifest path".into());
    }
    Ok(path)
}

fn prohibited_output(path: &Path) -> bool {
    if path == Path::new("index.html") {
        return false;
    }
    if path.components().next() != Some(Component::Normal("assets".as_ref())) {
        return true;
    }
    !matches!(
        path.extension().and_then(|value| value.to_str()),
        Some("js" | "css" | "png" | "jpg" | "jpeg" | "webp" | "avif" | "svg" | "ico" | "woff" | "woff2")
    )
}

fn validate_regular_file(path: &Path) -> Result<(), String> {
    reject_symlink(path)?;
    if !fs::metadata(path)
        .map_err(|error| error.to_string())?
        .is_file()
    {
        return Err(format!("not a regular file: {}", path.display()));
    }
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<(), String> {
    if fs::symlink_metadata(path)
        .map_err(|error| error.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err(format!("symbolic link is not allowed: {}", path.display()));
    }
    Ok(())
}

fn sha(path: &Path) -> Result<String, String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(fs::read(path).map_err(|error| error.to_string())?)
    ))
}

fn rerun(path: &Path, emit: bool) {
    if emit {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}
