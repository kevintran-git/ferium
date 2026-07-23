use super::Metadata;
use crate::{
    config::filters::{Filter, ReleaseChannel},
    iter_ext::{IterExt, IterExtPositions},
    MODRINTH_API,
};
use ferinth::structures::tag::GameVersionType;
use regex::Regex;
use std::{collections::HashSet, sync::OnceLock};

#[derive(thiserror::Error, Debug)]
#[error(transparent)]
pub enum Error {
    VersionGrouping(#[from] ferinth::Error),
    FilenameRegex(#[from] regex::Error),
    #[error("The following filter(s) were empty: {}", _0.iter().display(", "))]
    FilterEmpty(Vec<String>),
    #[error("Failed to find a compatible combination")]
    IntersectFailure,
    #[error("{0} is not a known game version")]
    UnknownGameVersion(String),
}
pub type Result<T> = std::result::Result<T, Error>;

static VERSION_GROUPS: OnceLock<Vec<Vec<String>>> = OnceLock::new();

/// Gets groups of versions that are considered minor updates in terms of mod compatibility
///
/// This is determined by Modrinth's `major` parameter for game versions.
pub async fn get_version_groups() -> Result<&'static Vec<Vec<String>>> {
    if let Some(v) = VERSION_GROUPS.get() {
        Ok(v)
    } else {
        let versions = MODRINTH_API.tag_list_game_versions().await?;
        let mut v = vec![vec![]];
        for version in versions {
            if version.version_type == GameVersionType::Release {
                v.last_mut().unwrap().push(version.version);
                if version.major {
                    v.push(vec![]);
                }
            }
        }
        let _ = VERSION_GROUPS.set(v);

        Ok(VERSION_GROUPS.get().unwrap())
    }
}

impl Filter {
    /// Returns the indices of `download_files` that have successfully filtered through `self`
    ///
    /// This function fails if getting version groups fails, or the regex files to parse.
    pub async fn filter(
        &self,
        download_files: impl Iterator<Item = (usize, &Metadata)> + Clone,
    ) -> Result<HashSet<usize>> {
        Ok(match self {
            Filter::ModLoaderPrefer(loaders) => loaders
                .iter()
                .map(move |l| {
                    download_files
                        .clone()
                        .positions(|f| f.loaders.contains(l))
                        .collect_hashset()
                })
                .find(|v| !v.is_empty())
                .unwrap_or_default(),

            Filter::ModLoaderAny(loaders) => download_files
                .positions(|f| loaders.iter().any(|l| f.loaders.contains(l)))
                .collect_hashset(),

            Filter::GameVersionStrict(versions) => download_files
                .positions(|f| versions.iter().any(|vc| f.game_versions.contains(vc)))
                .collect_hashset(),

            Filter::GameVersionMinor(versions) => {
                let mut final_versions = vec![];
                for group in get_version_groups().await? {
                    if group.iter().any(|v| versions.contains(v)) {
                        final_versions.extend(group.clone());
                    }
                }

                download_files
                    .positions(|f| final_versions.iter().any(|vc| f.game_versions.contains(vc)))
                    .collect_hashset()
            }

            Filter::GameVersionRange { from, to } => {
                let ordered = get_version_groups().await?.iter().flatten().collect_vec();

                let from_idx = match from {
                    Some(v) => ordered
                        .iter()
                        .position(|o| *o == v)
                        .ok_or_else(|| Error::UnknownGameVersion(v.clone()))?,
                    None => ordered.len().saturating_sub(1),
                };
                let to_idx = match to {
                    Some(v) => ordered
                        .iter()
                        .position(|o| *o == v)
                        .ok_or_else(|| Error::UnknownGameVersion(v.clone()))?,
                    None => 0,
                };
                let (newest_idx, oldest_idx) = (to_idx.min(from_idx), to_idx.max(from_idx));
                let final_versions = &ordered[newest_idx..=oldest_idx];

                download_files
                    .positions(|f| final_versions.iter().any(|vc| f.game_versions.contains(*vc)))
                    .collect_hashset()
            }

            Filter::ReleaseChannel(channel) => download_files
                .positions(|f| match channel {
                    ReleaseChannel::Alpha => true,
                    ReleaseChannel::Beta => {
                        f.channel == ReleaseChannel::Beta || f.channel == ReleaseChannel::Release
                    }
                    ReleaseChannel::Release => f.channel == ReleaseChannel::Release,
                })
                .collect_hashset(),

            Filter::Filename(regex) => {
                let regex = Regex::new(regex)?;
                download_files
                    .positions(|f| regex.is_match(&f.filename))
                    .collect_hashset()
            }

            Filter::Title(regex) => {
                let regex = Regex::new(regex)?;
                download_files
                    .positions(|f| regex.is_match(&f.title))
                    .collect_hashset()
            }

            Filter::Description(regex) => {
                let regex = Regex::new(regex)?;
                download_files
                    .positions(|f| regex.is_match(&f.description))
                    .collect_hashset()
            }
        })
    }
}

/// Assumes that the provided `download_files` are sorted in the order of preference (e.g. chronological)
pub async fn select_latest(
    download_files: impl Iterator<Item = &Metadata> + Clone,
    filters: Vec<Filter>,
) -> Result<usize> {
    Ok(select_ordered(download_files, filters).await?[0])
}

/// Like [`select_latest`], but returns every index that passes every filter, ordered from most
/// to least preferred, instead of just the single most preferred one.
pub async fn select_ordered(
    download_files: impl Iterator<Item = &Metadata> + Clone,
    filters: Vec<Filter>,
) -> Result<Vec<usize>> {
    let mut filter_results = vec![];
    let mut run_last = vec![];

    for filter in &filters {
        if let Filter::ModLoaderPrefer(_) = filter {
            run_last.push((
                filter,
                filter.filter(download_files.clone().enumerate()).await?,
            ));
        } else {
            filter_results.push((
                filter,
                filter.filter(download_files.clone().enumerate()).await?,
            ));
        }
    }

    let empty_filtrations = filter_results
        .iter()
        .chain(run_last.iter())
        .filter_map(|(filter, indices)| {
            if indices.is_empty() {
                Some(filter.to_string())
            } else {
                None
            }
        })
        .collect_vec();
    if !empty_filtrations.is_empty() {
        return Err(Error::FilterEmpty(empty_filtrations));
    }

    let mut filter_results = filter_results.into_iter().map(|(_, set)| set);

    let final_indices = filter_results
        .next()
        .map(|set_1| {
            filter_results.fold(set_1, |set_a, set_b| {
                set_a.intersection(&set_b).copied().collect_hashset()
            })
        })
        .unwrap_or_default();

    let mut ordered = if run_last.is_empty() {
        final_indices.into_iter().collect_vec()
    } else {
        let download_files = download_files.into_iter().enumerate().filter_map(|(i, f)| {
            if final_indices.contains(&i) {
                Some((i, f))
            } else {
                None
            }
        });

        let mut filter_results = vec![];
        for (filter, _) in run_last {
            filter_results.push(filter.filter(download_files.clone()).await?)
        }
        let mut filter_results = filter_results.into_iter();

        filter_results
            .next()
            .map(|set_1| {
                filter_results
                    .fold(set_1, |set_a, set_b| {
                        set_a.intersection(&set_b).copied().collect_hashset()
                    })
                    .into_iter()
                    .collect_vec()
            })
            .unwrap_or_default()
    };

    if ordered.is_empty() {
        return Err(Error::IntersectFailure);
    }
    ordered.sort_unstable();
    Ok(ordered)
}
