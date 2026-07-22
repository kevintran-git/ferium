use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use std::{
    fs::{create_dir_all, read_to_string, write, File},
    path::Path,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("No stable Fabric loader is available for Minecraft {0}")]
    NoStableLoader(String),
}
type Result<T> = std::result::Result<T, Error>;

#[derive(Deserialize)]
struct LoaderEntry {
    loader: LoaderInfo,
}

#[derive(Deserialize)]
struct LoaderInfo {
    version: String,
    stable: bool,
}

/// Installs a Fabric loader version profile into `minecraft_dir` for `mc_version`, so the
/// vanilla launcher can run it. Idempotent — does nothing if the version is already installed.
/// Returns the installed version id.
pub async fn install_fabric_loader(minecraft_dir: &Path, mc_version: &str) -> Result<String> {
    let loaders: Vec<LoaderEntry> = Client::new()
        .get(format!(
            "https://meta.fabricmc.net/v2/versions/loader/{mc_version}"
        ))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let loader_version = loaders
        .into_iter()
        .find(|entry| entry.loader.stable)
        .map(|entry| entry.loader.version)
        .ok_or_else(|| Error::NoStableLoader(mc_version.to_owned()))?;

    let version_id = format!("fabric-loader-{loader_version}-{mc_version}");
    let version_dir = minecraft_dir.join("versions").join(&version_id);
    let version_file = version_dir.join(format!("{version_id}.json"));

    if !version_file.exists() {
        let profile_json = Client::new()
            .get(format!(
                "https://meta.fabricmc.net/v2/versions/loader/{mc_version}/{loader_version}/profile/json"
            ))
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        create_dir_all(&version_dir)?;
        write(&version_file, &profile_json)?;
    }

    Ok(version_id)
}

/// Adds or updates a `"custom"` profile entry keyed by `profile_id` in `minecraft_dir`'s
/// `launcher_profiles.json`, creating a minimal valid skeleton if the file doesn't exist yet.
/// Every other key in the file (and every other profile) is left untouched.
pub fn upsert_launcher_profile(
    minecraft_dir: &Path,
    profile_id: &str,
    name: &str,
    version_id: &str,
) -> Result<()> {
    let path = minecraft_dir.join("launcher_profiles.json");

    let mut root: serde_json::Value = if path.exists() {
        serde_json::from_str(&read_to_string(&path)?)?
    } else {
        json!({ "profiles": {}, "settings": {}, "version": 3 })
    };

    if !root.get("profiles").is_some_and(serde_json::Value::is_object) {
        root["profiles"] = json!({});
    }

    let now = chrono::Utc::now().to_rfc3339();
    root["profiles"][profile_id] = json!({
        "name": name,
        "type": "custom",
        "created": now,
        "lastUsed": now,
        "icon": "Furnace",
        "lastVersionId": version_id,
    });

    if let Some(parent) = path.parent() {
        create_dir_all(parent)?;
    }
    serde_json::to_writer_pretty(File::create(&path)?, &root)?;

    Ok(())
}
