use crate::download::{install_files, read_overrides};
use anyhow::{Context as _, Result};
use colored::Colorize as _;
use libium::{
    config::structs::{ModIdentifier, Profile, ProjectKind},
    modpack::{
        group::{self, Resolution},
        zip_extract,
    },
    PROJECT_DIRS,
};

const KINDS: [ProjectKind; 3] = [
    ProjectKind::Mod,
    ProjectKind::ResourcePack,
    ProjectKind::ShaderPack,
];

/// Resolves the modpack group at `profile.modpacks[index]`, reconciles it against
/// `profile.mods`/`shaderpacks`/`resourcepacks` in place, and installs its overrides if the
/// upstream source has changed since it was last applied.
pub async fn refresh_group(profile: &mut Profile, index: usize) -> Result<()> {
    let group = profile.modpacks[index].clone();

    let resolved = match group::resolve(&group.source, group.last_seen_version.as_deref())
        .await
        .with_context(|| format!("Failed to resolve modpack group '{}'", group.name))?
    {
        Resolution::Unchanged => {
            println!("{}: {}", group.name.bold(), "unchanged, skipping".dimmed());
            return Ok(());
        }
        Resolution::Changed(resolved) => resolved,
    };

    let mut added = 0usize;
    let mut removed = 0usize;
    let mut updated = 0usize;

    for kind in KINDS {
        let resolved_ids: Vec<ModIdentifier> = resolved
            .entries
            .iter()
            .filter(|entry| entry.kind == kind)
            .map(|entry| entry.identifier.clone())
            .collect();

        let diff = group::diff_entries(&group.name, &group.excluded, &resolved_ids, profile.mods(kind));

        for &i in &diff.conflicts {
            let existing = &profile.mods(kind)[i];
            let owner = existing
                .source_modpack
                .clone()
                .unwrap_or_else(|| "your own additions".to_owned());
            println!(
                "{} is already tracked (via {owner}) — leaving its current version",
                existing.name
            );
        }

        for (i, new_id) in diff.updated {
            profile.mods_mut(kind)[i].identifier = new_id;
            updated += 1;
        }

        let mut removed_indices = diff.removed;
        removed_indices.sort_unstable_by(|a, b| b.cmp(a));
        for i in removed_indices {
            profile.mods_mut(kind).remove(i);
            removed += 1;
        }

        if !diff.new_identifiers.is_empty() {
            let (successes, failures) =
                libium::add(profile, kind, diff.new_identifiers, false, false, vec![]).await?;
            for (_, id) in &successes {
                if let Some(m) = profile
                    .mods_mut(kind)
                    .iter_mut()
                    .find(|m| m.identifier.is_same_as(id))
                {
                    m.source_modpack = Some(group.name.clone());
                }
            }
            added += successes.len();
            for (name, err) in failures {
                eprintln!(
                    "{}",
                    format!("WARNING: couldn't add {name} from '{}': {err}", group.name).yellow()
                );
            }
        }
    }

    println!(
        "{}: +{added} -{removed} mods, {updated} version(s) bumped",
        group.name.bold(),
    );

    if group.install_overrides {
        let extract_dir = PROJECT_DIRS
            .cache_dir()
            .join("extracted-overrides")
            .join(&group.name);
        zip_extract(&resolved.archive_path, &extract_dir)?;

        let mut to_install = Vec::new();
        for subdir in &resolved.override_subdirs {
            to_install.extend(read_overrides(&extract_dir.join(subdir))?);
        }
        if !to_install.is_empty() {
            install_files(&profile.output_dir, to_install)?;
        }
    }

    profile.modpacks[index].last_seen_version = Some(resolved.version_marker);

    Ok(())
}
