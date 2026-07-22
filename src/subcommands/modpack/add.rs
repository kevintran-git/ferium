use super::refresh::refresh_group;
use anyhow::{anyhow, ensure, Result};
use colored::Colorize as _;
use libium::{
    config::structs::{ModpackGroup, ModpackSource, Profile},
    modpack::{add as modpack_add, group},
};
use reqwest::Url;
use std::path::Path;

pub async fn add(
    profile: &mut Profile,
    identifier: String,
    name: Option<String>,
    no_overrides: bool,
) -> Result<()> {
    let (source, default_name) = resolve_identifier(&profile.modpacks, &identifier).await?;

    let name = name.unwrap_or(default_name);
    ensure!(
        !profile
            .modpacks
            .iter()
            .any(|g| g.name.eq_ignore_ascii_case(&name)),
        "A modpack group named '{name}' is already tracked in this profile"
    );

    profile.modpacks.push(ModpackGroup {
        name: name.clone(),
        source,
        last_seen_version: None,
        install_overrides: !no_overrides,
        excluded: vec![],
    });
    let index = profile.modpacks.len() - 1;

    println!("{} {}", "Tracking modpack group".green(), name.bold());
    refresh_group(profile, index).await
}

/// Disambiguates `identifier` into a concrete [`ModpackSource`], along with a default name for
/// the group. `existing_groups` is used only to reject a project already tracked in this profile.
pub async fn resolve_identifier(
    existing_groups: &[ModpackGroup],
    identifier: &str,
) -> Result<(ModpackSource, String)> {
    if let Ok(url) = Url::parse(identifier) {
        resolve_url_source(url).await
    } else if Path::new(identifier).exists() {
        let path = Path::new(identifier).canonicalize()?;
        let url = Url::from_file_path(&path)
            .map_err(|()| anyhow!("Could not convert `{}` to a file URL", path.display()))?;
        resolve_url_source(url).await
    } else if let Ok(project_id) = identifier.parse::<i32>() {
        let project = modpack_add::curseforge(existing_groups, project_id).await?;
        Ok((ModpackSource::CurseForgeHosted(project.id), project.name))
    } else {
        match modpack_add::modrinth(existing_groups, identifier).await {
            Ok(project) => Ok((
                ModpackSource::ModrinthHosted(project.id.clone()),
                project.title,
            )),
            Err(modpack_add::Error::ModrinthError(ferinth::Error::InvalidIDorSlug)) => {
                Err(anyhow!("Invalid identifier"))
            }
            Err(err) => Err(err.into()),
        }
    }
}

async fn resolve_url_source(url: Url) -> Result<(ModpackSource, String)> {
    let format = group::sniff_format(&url).await?;
    let default_name = url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .filter(|s| !s.is_empty())
        .unwrap_or("modpack")
        .trim_end_matches(".mrpack")
        .trim_end_matches(".zip")
        .to_owned();
    Ok(match format {
        group::ArchiveFormat::Mrpack => (ModpackSource::MrpackFile(url), default_name),
        group::ArchiveFormat::CurseForgeZip => (ModpackSource::CurseForgeZipFile(url), default_name),
    })
}
