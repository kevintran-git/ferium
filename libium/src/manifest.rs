use crate::version_range::Requirement;
use regex::Regex;
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{Read, Seek},
    path::{Path, PathBuf},
    sync::OnceLock,
    time::SystemTime,
};
use zip::ZipArchive;

pub struct ModManifest {
    pub id: String,
    pub version: Option<String>,
    pub depends: HashMap<String, Requirement>,
}

pub fn parse_manifest<R: Read + Seek>(reader: R) -> Option<ModManifest> {
    let mut archive = ZipArchive::new(reader).ok()?;

    if let Ok(mut entry) = archive.by_name("fabric.mod.json") {
        let mut contents = String::new();
        entry.read_to_string(&mut contents).ok()?;
        let json: serde_json::Value = serde_json::from_str(&contents).ok()?;
        let id = json.get("id")?.as_str()?.to_string();
        let version = json
            .get("version")
            .and_then(serde_json::Value::as_str)
            .map(String::from);
        let depends = json
            .get("depends")
            .and_then(serde_json::Value::as_object)
            .map(|map| {
                map.iter()
                    .map(|(k, v)| (k.clone(), Requirement::parse_fabric(v)))
                    .collect()
            })
            .unwrap_or_default();
        return Some(ModManifest { id, version, depends });
    }

    if let Ok(mut entry) = archive.by_name("quilt.mod.json") {
        let mut contents = String::new();
        entry.read_to_string(&mut contents).ok()?;
        let json: serde_json::Value = serde_json::from_str(&contents).ok()?;
        let loader = json.get("quilt_loader")?;
        let id = loader.get("id")?.as_str()?.to_string();
        let version = loader
            .get("version")
            .and_then(serde_json::Value::as_str)
            .map(String::from);
        let mut depends = HashMap::new();
        for dep in loader
            .get("depends")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
        {
            let (dep_id, versions) = match dep {
                serde_json::Value::String(dep_id) => (dep_id.clone(), None),
                serde_json::Value::Object(_) => {
                    let Some(dep_id) = dep.get("id").and_then(serde_json::Value::as_str) else {
                        continue;
                    };
                    (dep_id.to_string(), dep.get("versions"))
                }
                _ => continue,
            };
            let requirement = versions.map_or_else(Requirement::any, Requirement::parse_fabric);
            depends.insert(dep_id, requirement);
        }
        return Some(ModManifest { id, version, depends });
    }

    for path in ["META-INF/mods.toml", "META-INF/neoforge.mods.toml"] {
        if let Ok(mut entry) = archive.by_name(path) {
            let mut contents = String::new();
            entry.read_to_string(&mut contents).ok()?;
            return parse_toml_manifest(&contents);
        }
    }

    None
}

fn parse_toml_manifest(contents: &str) -> Option<ModManifest> {
    static MOD_ID: OnceLock<Regex> = OnceLock::new();
    static VERSION: OnceLock<Regex> = OnceLock::new();
    static DEP_BLOCK: OnceLock<Regex> = OnceLock::new();
    static DEP_RANGE: OnceLock<Regex> = OnceLock::new();

    let mod_id_re = MOD_ID.get_or_init(|| Regex::new(r#"modId\s*=\s*"([^"]+)""#).unwrap());
    let id = mod_id_re.captures(contents)?[1].to_string();

    let version_re =
        VERSION.get_or_init(|| Regex::new(r#"(?m)^\s*version\s*=\s*"([^"]+)""#).unwrap());
    let version = version_re
        .captures(contents)
        .map(|c| c[1].to_string())
        .filter(|v| !v.contains("${"));

    let dep_header_re =
        DEP_BLOCK.get_or_init(|| Regex::new(r"\[\[dependencies\.[^\]]+\]\]").unwrap());
    let dep_range_re =
        DEP_RANGE.get_or_init(|| Regex::new(r#"versionRange\s*=\s*"([^"]*)""#).unwrap());

    let headers = dep_header_re.find_iter(contents).collect::<Vec<_>>();
    let mut depends = HashMap::new();
    for (i, header) in headers.iter().enumerate() {
        let end = headers.get(i + 1).map_or(contents.len(), |next| next.start());
        let block = &contents[header.end()..end];
        let Some(dep_id) = mod_id_re.captures(block).map(|c| c[1].to_string()) else {
            continue;
        };
        let range = dep_range_re
            .captures(block)
            .map(|c| c[1].to_string())
            .unwrap_or_default();
        depends.insert(dep_id, Requirement::parse_maven(&range));
    }

    Some(ModManifest { id, version, depends })
}

pub fn parse_manifest_file(jar_path: &Path) -> Option<ModManifest> {
    parse_manifest(File::open(jar_path).ok()?)
}

pub fn mod_manifest_id(jar_path: &Path) -> Option<String> {
    parse_manifest_file(jar_path).map(|m| m.id)
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

#[cfg(test)]
mod tests {
    use super::{parse_manifest, parse_toml_manifest};
    use std::io::{Cursor, Write as _};
    use zip::write::SimpleFileOptions;

    fn make_jar(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(Cursor::new(&mut buf));
            for (name, contents) in entries {
                writer
                    .start_file(*name, SimpleFileOptions::default())
                    .unwrap();
                writer.write_all(contents.as_bytes()).unwrap();
            }
            writer.finish().unwrap();
        }
        buf
    }

    #[test]
    fn fabric_manifest_extracts_id_version_depends() {
        let jar = make_jar(&[(
            "fabric.mod.json",
            r#"{"id":"examplemod","version":"1.2.3","depends":{"fabricloader":">=0.14.0","fabric-api":"*"}}"#,
        )]);
        let manifest = parse_manifest(Cursor::new(jar)).unwrap();
        assert_eq!(manifest.id, "examplemod");
        assert_eq!(manifest.version.as_deref(), Some("1.2.3"));
        assert!(manifest.depends["fabricloader"].satisfies("0.15.0"));
        assert!(!manifest.depends["fabricloader"].satisfies("0.13.0"));
        assert!(manifest.depends["fabric-api"].satisfies("anything"));
    }

    #[test]
    fn quilt_manifest_extracts_shorthand_and_object_depends() {
        let jar = make_jar(&[(
            "quilt.mod.json",
            r#"{"quilt_loader":{"id":"examplemod","version":"2.0.0","depends":["shorthand_dep",{"id":"other_dep","versions":">=1.0.0"}]}}"#,
        )]);
        let manifest = parse_manifest(Cursor::new(jar)).unwrap();
        assert_eq!(manifest.id, "examplemod");
        assert_eq!(manifest.version.as_deref(), Some("2.0.0"));
        assert!(manifest.depends["shorthand_dep"].satisfies("anything"));
        assert!(manifest.depends["other_dep"].satisfies("1.5.0"));
        assert!(!manifest.depends["other_dep"].satisfies("0.5.0"));
    }

    #[test]
    fn forge_toml_manifest_extracts_version_ranges() {
        let toml = r#"
[[mods]]
modId="examplemod"
version="3.1.0"

[[dependencies.examplemod]]
    modId="forge"
    mandatory=true
    versionRange="[47,)"
[[dependencies.examplemod]]
    modId="other_mod"
    mandatory=false
    versionRange="[1.0,2.0)"
"#;
        let manifest = parse_toml_manifest(toml).unwrap();
        assert_eq!(manifest.id, "examplemod");
        assert_eq!(manifest.version.as_deref(), Some("3.1.0"));
        assert!(manifest.depends["forge"].satisfies("50"));
        assert!(!manifest.depends["forge"].satisfies("40"));
        assert!(manifest.depends["other_mod"].satisfies("1.5"));
        assert!(!manifest.depends["other_mod"].satisfies("2.0"));
    }

    #[test]
    fn toml_manifest_ignores_template_version() {
        let toml = r#"
[[mods]]
modId="examplemod"
version="${file.jarVersion}"
"#;
        let manifest = parse_toml_manifest(toml).unwrap();
        assert_eq!(manifest.version, None);
    }
}
