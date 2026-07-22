use super::{
    curseforge::structs::Manifest as CFManifest,
    modrinth::structs::{
        DependencyID, Metadata as MRMetadata, ModpackFile as MRModpackFile,
    },
    read_file_from_zip,
};
use crate::config::structs::{Mod, ModIdentifier};
use crate::{
    config::structs::{ModLoader, ModpackSource, ProjectKind},
    CURSEFORGE_API, MODRINTH_API, PROJECT_DIRS,
};
use ferinth::structures::project::SideType;
use reqwest::{Client, Url};
use sha1_smol::Sha1;
use std::{
    collections::HashMap,
    fs::{create_dir_all, read, write, File},
    io::BufReader,
    path::{Path, PathBuf},
    str::FromStr as _,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Modrinth: {0}")]
    Modrinth(#[from] ferinth::Error),
    #[error("CurseForge: {0}")]
    CurseForge(#[from] furse::Error),
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Zip(#[from] zip::result::ZipError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("The modpack has no files available")]
    NoFilesAvailable,
    #[error(
        "The developer of this project has denied third party applications from downloading it"
    )]
    DistributionDenied,
    #[error("The archive did not contain a modrinth.index.json or manifest.json file")]
    UnrecognisedFormat,
    #[error("`{0}` is not a valid local file URL")]
    InvalidFileUrl(Url),
}
type Result<T> = std::result::Result<T, Error>;

pub struct ResolvedEntry {
    pub identifier: ModIdentifier,
    pub kind: ProjectKind,
}

pub struct ResolvedModpack {
    pub entries: Vec<ResolvedEntry>,
    pub version_marker: String,
    pub archive_path: PathBuf,
    pub override_subdirs: Vec<String>,
    pub game_version: Option<String>,
    pub mod_loader: Option<ModLoader>,
}

pub enum Resolution {
    Unchanged,
    Changed(ResolvedModpack),
}

pub enum ArchiveFormat {
    Mrpack,
    CurseForgeZip,
}

pub async fn sniff_format(url: &Url) -> Result<ArchiveFormat> {
    let (archive_path, _) = download_to_cache(url).await?;
    let mut zip = zip::ZipArchive::new(File::open(&archive_path)?)?;
    if zip.by_name("modrinth.index.json").is_ok() {
        Ok(ArchiveFormat::Mrpack)
    } else if zip.by_name("manifest.json").is_ok() {
        Ok(ArchiveFormat::CurseForgeZip)
    } else {
        Err(Error::UnrecognisedFormat)
    }
}

pub async fn resolve(source: &ModpackSource, last_seen_version: Option<&str>) -> Result<Resolution> {
    match source {
        ModpackSource::ModrinthHosted(project_id) => {
            let version = MODRINTH_API
                .version_list(project_id)
                .await?
                .into_iter()
                .next()
                .ok_or(Error::NoFilesAvailable)?;
            if last_seen_version == Some(version.id.as_str()) {
                return Ok(Resolution::Unchanged);
            }
            let file = crate::version_ext::VersionExt::get_version_file(&version)
                .ok_or(Error::NoFilesAvailable)?;
            let (archive_path, _) = download_to_cache(&file.url).await?;
            resolve_mrpack(archive_path, version.id).await.map(Resolution::Changed)
        }
        ModpackSource::MrpackFile(url) => {
            let (archive_path, content_hash) = download_to_cache(url).await?;
            if last_seen_version == Some(content_hash.as_str()) {
                return Ok(Resolution::Unchanged);
            }
            resolve_mrpack(archive_path, content_hash).await.map(Resolution::Changed)
        }
        ModpackSource::CurseForgeHosted(project_id) => {
            let file = CURSEFORGE_API
                .get_mod_files(*project_id)
                .await?
                .into_iter()
                .next()
                .ok_or(Error::NoFilesAvailable)?;
            let marker = file.id.to_string();
            if last_seen_version.is_some_and(|last| last == marker) {
                return Ok(Resolution::Unchanged);
            }
            let download_url = file.download_url.ok_or(Error::DistributionDenied)?;
            let (archive_path, _) = download_to_cache(&download_url).await?;
            resolve_cf_manifest(archive_path, marker).map(Resolution::Changed)
        }
        ModpackSource::CurseForgeZipFile(url) => {
            let (archive_path, content_hash) = download_to_cache(url).await?;
            if last_seen_version == Some(content_hash.as_str()) {
                return Ok(Resolution::Unchanged);
            }
            resolve_cf_manifest(archive_path, content_hash).map(Resolution::Changed)
        }
    }
}

struct TrackedFile {
    hash: String,
    path: PathBuf,
    kind: ProjectKind,
}

fn filter_mrpack_files(files: Vec<MRModpackFile>) -> Vec<TrackedFile> {
    let mut tracked = Vec::new();
    for file in files {
        if let Some(env) = &file.env {
            if env.client == SideType::Unsupported {
                continue;
            }
        }
        let Some(kind) = classify_path(&file.path) else {
            eprintln!(
                "WARNING: {} is outside a folder this client tracks, skipping",
                file.path.display()
            );
            continue;
        };
        tracked.push(TrackedFile {
            hash: file.hashes.sha1,
            path: file.path,
            kind,
        });
    }
    tracked
}

async fn resolve_mrpack(archive_path: PathBuf, version_marker: String) -> Result<ResolvedModpack> {
    let metadata: MRMetadata = serde_json::from_str(
        &read_file_from_zip(BufReader::new(File::open(&archive_path)?), "modrinth.index.json")?
            .ok_or(Error::UnrecognisedFormat)?,
    )?;

    let game_version = metadata.dependencies.get(&DependencyID::Minecraft).cloned();
    let mod_loader = metadata.dependencies.keys().find_map(|id| match id {
        DependencyID::FabricLoader => Some(ModLoader::Fabric),
        DependencyID::QuiltLoader => Some(ModLoader::Quilt),
        DependencyID::Forge => Some(ModLoader::Forge),
        DependencyID::Neoforge => Some(ModLoader::NeoForge),
        DependencyID::Minecraft => None,
    });

    let tracked = filter_mrpack_files(metadata.files);
    let hashes: Vec<String> = tracked.iter().map(|t| t.hash.clone()).collect();

    let mut versions = if hashes.is_empty() {
        HashMap::new()
    } else {
        MODRINTH_API
            .version_get_from_multiple_hashes(hashes)
            .await?
    };

    let mut entries = Vec::new();
    for file in tracked {
        if let Some(version) = versions.remove(&file.hash) {
            entries.push(ResolvedEntry {
                identifier: ModIdentifier::ModrinthProject(version.project_id, Some(version.id)),
                kind: file.kind,
            });
        } else {
            eprintln!(
                "WARNING: {} could not be resolved to a Modrinth project, skipping",
                file.path.display()
            );
        }
    }

    Ok(ResolvedModpack {
        entries,
        version_marker,
        archive_path,
        override_subdirs: vec!["overrides".to_owned(), "client-overrides".to_owned()],
        game_version,
        mod_loader,
    })
}

fn resolve_cf_manifest(archive_path: PathBuf, version_marker: String) -> Result<ResolvedModpack> {
    let manifest: CFManifest = serde_json::from_str(
        &read_file_from_zip(BufReader::new(File::open(&archive_path)?), "manifest.json")?
            .ok_or(Error::UnrecognisedFormat)?,
    )?;

    let entries = manifest
        .files
        .iter()
        .map(|file| ResolvedEntry {
            identifier: ModIdentifier::CurseForgeProject(
                file.project_id,
                Some(file.file_id.to_string()),
            ),
            kind: ProjectKind::Mod,
        })
        .collect();

    let mod_loader = manifest
        .minecraft
        .mod_loaders
        .iter()
        .find(|loader| loader.primary)
        .or_else(|| manifest.minecraft.mod_loaders.first())
        .and_then(|loader| loader.id.split('-').next())
        .and_then(|name| ModLoader::from_str(name).ok());

    Ok(ResolvedModpack {
        entries,
        version_marker,
        archive_path,
        override_subdirs: vec![manifest.overrides],
        game_version: Some(manifest.minecraft.version),
        mod_loader,
    })
}

fn classify_path(path: &Path) -> Option<ProjectKind> {
    match path.components().next()?.as_os_str().to_str()? {
        "mods" => Some(ProjectKind::Mod),
        "resourcepacks" => Some(ProjectKind::ResourcePack),
        "shaderpacks" => Some(ProjectKind::ShaderPack),
        _ => None,
    }
}

async fn download_to_cache(url: &Url) -> Result<(PathBuf, String)> {
    let bytes = if url.scheme() == "file" {
        let path = url
            .to_file_path()
            .map_err(|()| Error::InvalidFileUrl(url.clone()))?;
        read(path)?
    } else {
        Client::new()
            .get(url.clone())
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?
            .to_vec()
    };

    let content_hash = format!("{}", Sha1::from(&bytes).digest());

    let cache_dir = PROJECT_DIRS.cache_dir().join("modpack-groups");
    create_dir_all(&cache_dir)?;
    let archive_path = cache_dir.join(format!("{content_hash}.zip"));
    write(&archive_path, &bytes)?;

    Ok((archive_path, content_hash))
}

/// The result of reconciling a group's resolved upstream entries against the mods a profile
/// already tracks for that kind. Indices refer back into the `existing` slice passed to
/// [`diff_entries`].
#[derive(Debug, Default, PartialEq, Eq)]
pub struct DiffOutcome {
    /// Resolved identifiers not tracked anywhere yet; the caller should add and tag these
    pub new_identifiers: Vec<ModIdentifier>,
    /// `(existing index, new pin)` for mods this group already tracks whose upstream pin changed
    pub updated: Vec<(usize, ModIdentifier)>,
    /// Indices of mods this group tracks that are no longer provided upstream
    pub removed: Vec<usize>,
    /// Indices of mods that match a resolved entry but are tracked by something else (another
    /// group, or freestanding) — left untouched, reported to the user
    pub conflicts: Vec<usize>,
}

/// Reconciles `resolved` (the group's current upstream entries, already filtered to one
/// [`ProjectKind`]) against `existing` (`profile.mods`/`shaderpacks`/`resourcepacks` for that
/// same kind), without performing any I/O
pub fn diff_entries(
    group_name: &str,
    excluded: &[ModIdentifier],
    resolved: &[ModIdentifier],
    existing: &[Mod],
) -> DiffOutcome {
    let mut outcome = DiffOutcome::default();

    for (i, existing_mod) in existing.iter().enumerate() {
        if existing_mod.source_modpack.as_deref() == Some(group_name)
            && !resolved
                .iter()
                .any(|id| id.is_same_as(&existing_mod.identifier))
        {
            outcome.removed.push(i);
        }
    }

    for id in resolved {
        if excluded.iter().any(|ex| ex.is_same_as(id)) {
            continue;
        }

        match existing
            .iter()
            .enumerate()
            .find(|(_, m)| m.identifier.is_same_as(id))
        {
            Some((i, existing_mod)) if existing_mod.source_modpack.as_deref() == Some(group_name) => {
                if &existing_mod.identifier != id {
                    outcome.updated.push((i, id.clone()));
                }
            }
            Some((i, _)) => outcome.conflicts.push(i),
            None => outcome.new_identifiers.push(id.clone()),
        }
    }

    outcome
}

#[cfg(test)]
mod tests {
    use super::{classify_path, diff_entries, filter_mrpack_files, MRModpackFile};
    use crate::config::structs::{Mod, ModIdentifier, ProjectKind};
    use crate::modpack::modrinth::structs::ModpackFileEnvironment;
    use ferinth::structures::{project::SideType, version::Hash};

    fn hash(sha1: &str) -> Hash {
        Hash {
            sha1: sha1.to_owned(),
            sha512: String::new(),
            others: Default::default(),
        }
    }

    fn mrpack_file(path: &str, sha1: &str, client: SideType) -> MRModpackFile {
        MRModpackFile {
            path: path.into(),
            hashes: hash(sha1),
            env: Some(ModpackFileEnvironment {
                client,
                server: SideType::Required,
            }),
            downloads: vec![],
            file_size: 0,
        }
    }

    fn tracked_mod(name: &str, id: &str, pin: Option<&str>, source_modpack: Option<&str>) -> Mod {
        let mut m = Mod::new(
            name.to_owned(),
            ModIdentifier::ModrinthProject(id.to_owned(), pin.map(ToOwned::to_owned)),
            vec![],
            false,
        );
        m.source_modpack = source_modpack.map(ToOwned::to_owned);
        m
    }

    #[test]
    fn classify_path_maps_known_folders() {
        assert_eq!(classify_path("mods/a.jar".as_ref()), Some(ProjectKind::Mod));
        assert_eq!(
            classify_path("resourcepacks/a.zip".as_ref()),
            Some(ProjectKind::ResourcePack)
        );
        assert_eq!(
            classify_path("shaderpacks/a.zip".as_ref()),
            Some(ProjectKind::ShaderPack)
        );
        assert_eq!(classify_path("config/a.toml".as_ref()), None);
    }

    #[test]
    fn filter_mrpack_files_drops_server_only_and_unknown_folders() {
        let files = vec![
            mrpack_file("mods/client.jar", "a", SideType::Required),
            mrpack_file("mods/server-only.jar", "b", SideType::Unsupported),
            mrpack_file("config/settings.toml", "c", SideType::Required),
        ];
        let tracked = filter_mrpack_files(files);
        assert_eq!(tracked.len(), 1);
        assert_eq!(tracked[0].hash, "a");
        assert_eq!(tracked[0].kind, ProjectKind::Mod);
    }

    #[test]
    fn diff_entries_adds_new_mods() {
        let resolved = vec![ModIdentifier::ModrinthProject("AANobbMI".to_owned(), Some("v1".to_owned()))];
        let outcome = diff_entries("pack", &[], &resolved, &[]);
        assert_eq!(outcome.new_identifiers, resolved);
        assert!(outcome.updated.is_empty());
        assert!(outcome.removed.is_empty());
        assert!(outcome.conflicts.is_empty());
    }

    #[test]
    fn diff_entries_updates_changed_pin_for_own_mod() {
        let existing = vec![tracked_mod("Sodium", "AANobbMI", Some("v1"), Some("pack"))];
        let resolved = vec![ModIdentifier::ModrinthProject("AANobbMI".to_owned(), Some("v2".to_owned()))];
        let outcome = diff_entries("pack", &[], &resolved, &existing);
        assert_eq!(outcome.updated, vec![(0, resolved[0].clone())]);
        assert!(outcome.new_identifiers.is_empty());
    }

    #[test]
    fn diff_entries_removes_mods_no_longer_upstream() {
        let existing = vec![tracked_mod("Sodium", "AANobbMI", Some("v1"), Some("pack"))];
        let outcome = diff_entries("pack", &[], &[], &existing);
        assert_eq!(outcome.removed, vec![0]);
    }

    #[test]
    fn diff_entries_does_not_touch_other_profiles_mods_on_removal() {
        let existing = vec![tracked_mod("Sodium", "AANobbMI", Some("v1"), None)];
        let outcome = diff_entries("pack", &[], &[], &existing);
        assert!(outcome.removed.is_empty());
    }

    #[test]
    fn diff_entries_flags_conflict_instead_of_stealing_pin() {
        let existing = vec![tracked_mod("Sodium", "AANobbMI", Some("v1"), None)];
        let resolved = vec![ModIdentifier::ModrinthProject("AANobbMI".to_owned(), Some("v2".to_owned()))];
        let outcome = diff_entries("pack", &[], &resolved, &existing);
        assert_eq!(outcome.conflicts, vec![0]);
        assert!(outcome.updated.is_empty());
        assert!(outcome.new_identifiers.is_empty());
    }

    #[test]
    fn diff_entries_respects_excluded() {
        let resolved = vec![ModIdentifier::ModrinthProject("AANobbMI".to_owned(), Some("v2".to_owned()))];
        let outcome = diff_entries("pack", &resolved, &resolved, &[]);
        assert!(outcome.new_identifiers.is_empty());
    }
}
