use super::{mod_downloadable, DownloadData};
use crate::{
    config::{filters::Filter, structs::ModIdentifier},
    manifest::{parse_manifest, ModManifest},
    version_range::Requirement,
};
use futures_util::{stream, StreamExt as _};
use reqwest::Client;
use std::{collections::HashMap, io::Cursor};

const CONCURRENT_PEEKS: usize = 8;
const MAX_PROBES_PER_MOD: usize = 15;

async fn peek_manifest(client: &Client, download: &DownloadData) -> Option<ModManifest> {
    let bytes = client
        .get(download.download_url.clone())
        .send()
        .await
        .ok()?
        .bytes()
        .await
        .ok()?;
    parse_manifest(Cursor::new(bytes))
}

pub async fn apply_strict_deps(
    resolved: &mut [(ModIdentifier, DownloadData)],
    filters: &[Filter],
) -> Vec<String> {
    let client = Client::new();

    let peeks: Vec<Option<ModManifest>> = stream::iter(resolved.iter().map(|(_, dl)| dl))
        .map(|dl| peek_manifest(&client, dl))
        .buffered(CONCURRENT_PEEKS)
        .collect()
        .await;

    let mut id_to_index = HashMap::new();
    for (index, manifest) in peeks.iter().enumerate() {
        if let Some(manifest) = manifest {
            id_to_index.entry(manifest.id.clone()).or_insert(index);
        }
    }

    let mut requirements: HashMap<usize, Vec<Requirement>> = HashMap::new();
    for manifest in peeks.iter().flatten() {
        for (dep_id, requirement) in &manifest.depends {
            if let Some(&index) = id_to_index.get(dep_id) {
                requirements.entry(index).or_default().push(requirement.clone());
            }
        }
    }

    let mut notes = Vec::new();

    for (index, reqs) in requirements {
        let Some(current_version) = peeks[index].as_ref().and_then(|m| m.version.as_deref()) else {
            continue;
        };
        if reqs.iter().all(|r| r.satisfies(current_version)) {
            continue;
        }

        let identifier = resolved[index].0.clone();
        if identifier.pin().is_some() {
            continue;
        }

        let Ok(candidates) =
            mod_downloadable::fetch_ordered_candidates(&identifier, filters.to_vec()).await
        else {
            continue;
        };

        let mut replacement = None;
        for candidate in candidates.into_iter().take(MAX_PROBES_PER_MOD) {
            let Some(manifest) = peek_manifest(&client, &candidate).await else {
                continue;
            };
            let Some(version) = manifest.version.as_deref() else {
                continue;
            };
            if reqs.iter().all(|r| r.satisfies(version)) {
                replacement = Some(candidate);
                break;
            }
        }

        let old_filename = resolved[index].1.filename();
        if let Some(candidate) = replacement {
            let new_filename = candidate.filename();
            resolved[index].1 = candidate;
            notes.push(format!(
                "{old_filename} -> {new_filename} (to satisfy another mod's dependency requirement)"
            ));
        } else {
            notes.push(format!(
                "kept {old_filename} (a dependency requirement on it could not be satisfied by any available version)"
            ));
        }
    }

    notes
}
