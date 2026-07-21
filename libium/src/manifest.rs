use regex::Regex;
use std::{
    collections::HashMap,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    time::SystemTime,
};
use zip::ZipArchive;

pub fn mod_manifest_id(jar_path: &Path) -> Option<String> {
    let file = File::open(jar_path).ok()?;
    let mut archive = ZipArchive::new(file).ok()?;

    if let Ok(mut entry) = archive.by_name("fabric.mod.json") {
        let mut contents = String::new();
        entry.read_to_string(&mut contents).ok()?;
        let json: serde_json::Value = serde_json::from_str(&contents).ok()?;
        return json.get("id")?.as_str().map(String::from);
    }

    if let Ok(mut entry) = archive.by_name("quilt.mod.json") {
        let mut contents = String::new();
        entry.read_to_string(&mut contents).ok()?;
        let json: serde_json::Value = serde_json::from_str(&contents).ok()?;
        return json
            .get("quilt_loader")?
            .get("id")?
            .as_str()
            .map(String::from);
    }

    for path in ["META-INF/mods.toml", "META-INF/neoforge.mods.toml"] {
        if let Ok(mut entry) = archive.by_name(path) {
            let mut contents = String::new();
            entry.read_to_string(&mut contents).ok()?;
            let id = Regex::new(r#"modId\s*=\s*"([^"]+)""#)
                .ok()?
                .captures(&contents)
                .map(|caps| caps[1].to_string());
            if id.is_some() {
                return id;
            }
        }
    }

    None
}

/// Group the jars in `directory` by the mod ID in their manifest, and move
/// every jar but the most recently modified one in each group to
/// `directory`/.old. Returns the filenames that were moved.
pub fn dedupe_by_manifest_id(directory: &Path) -> std::io::Result<Vec<String>> {
    let mut by_id: HashMap<String, Vec<(PathBuf, SystemTime)>> = HashMap::new();

    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("jar") {
            continue;
        }
        let Some(id) = mod_manifest_id(&path) else {
            continue;
        };
        let modified = entry.metadata()?.modified()?;
        by_id.entry(id).or_default().push((path, modified));
    }

    let old_dir = directory.join(".old");
    let mut moved = Vec::new();
    for mut files in by_id.into_values() {
        if files.len() < 2 {
            continue;
        }
        files.sort_by_key(|(_, modified)| *modified);
        files.pop();
        for (path, _) in files {
            let Some(filename) = path.file_name() else {
                continue;
            };
            let filename = filename.to_owned();
            fs::create_dir_all(&old_dir)?;
            if fs::rename(&path, old_dir.join(&filename)).is_err() {
                fs::remove_file(&path)?;
            }
            moved.push(filename.to_string_lossy().into_owned());
        }
    }

    Ok(moved)
}
