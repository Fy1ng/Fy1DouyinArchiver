use super::{
    browser_state::{
        browser_profile_path, ensure_account_profile_root, reject_link, validate_directory,
        validate_profile_filesystem_scope, BrowserId,
    },
    file_store::{read_json, write_json_atomically},
};
use serde_json::{json, Map, Value};
use std::{fs, path::{Path, PathBuf}};

pub fn apply_profile_constraints(
    account_profile_root: &Path,
    browser_id: BrowserId,
) -> Result<PathBuf, String> {
    ensure_account_profile_root(account_profile_root)?;
    let profile = browser_profile_path(account_profile_root, browser_id);
    fs::create_dir_all(&profile).map_err(|error| error.to_string())?;
    validate_profile_filesystem_scope(account_profile_root, &profile)?;
    let default_dir = profile.join("Default");
    if default_dir.exists() {
        validate_directory(&default_dir)?;
    } else {
        fs::create_dir(&default_dir).map_err(|error| error.to_string())?;
        validate_directory(&default_dir)?;
    }

    merge_json(
        &default_dir.join("Preferences"),
        &[
            (&["signin", "allowed_on_next_startup"], json!(false)),
            (&["signin", "allowed"], json!(false)),
            (&["sync", "requested"], json!(false)),
            (&["sync", "has_setup_completed"], json!(false)),
        ],
    )?;
    merge_json(
        &profile.join("Local State"),
        &[(&["background_mode", "enabled"], json!(false))],
    )?;
    Ok(profile)
}

fn merge_json(path: &Path, updates: &[(&[&str], Value)]) -> Result<(), String> {
    let mut root = if path.exists() {
        reject_link(path)?;
        if !fs::metadata(path)
            .map_err(|error| error.to_string())?
            .is_file()
        {
            return Err("browser_profile_not_regular_file".to_string());
        }
        read_json::<Value>(path)?
    } else {
        Value::Object(Map::new())
    };
    if !root.is_object() {
        return Err("browser_profile_invalid_json_root".to_string());
    }
    for (segments, value) in updates {
        insert_value(&mut root, segments, value.clone())?;
    }
    write_json_atomically(path, &root)
}

fn insert_value(root: &mut Value, segments: &[&str], value: Value) -> Result<(), String> {
    let (last, parents) = segments
        .split_last()
        .ok_or_else(|| "browser_profile_invalid_preference_path".to_string())?;
    let mut current = root;
    for segment in parents {
        let object = current
            .as_object_mut()
            .ok_or_else(|| "browser_profile_invalid_json_root".to_string())?;
        current = object
            .entry((*segment).to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if !current.is_object() {
            return Err("browser_profile_invalid_preference_parent".to_string());
        }
    }
    current
        .as_object_mut()
        .ok_or_else(|| "browser_profile_invalid_json_root".to_string())?
        .insert((*last).to_string(), value);
    Ok(())
}
