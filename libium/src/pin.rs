use crate::{config::structs::ModIdentifier, CURSEFORGE_API, GITHUB_API, MODRINTH_API};
use std::cmp::Reverse;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Modrinth: {0}")]
    ModrinthError(#[from] ferinth::Error),
    #[error("CurseForge: {0}")]
    CurseForgeError(#[from] furse::Error),
    #[error("GitHub: {0:#?}")]
    GitHubError(#[from] octocrab::Error),
}
pub type Result<T> = std::result::Result<T, Error>;

/// A specific version/file/release that a mod's project could be pinned to
#[derive(Debug, Clone)]
pub struct PinOption {
    /// The value to store as the mod's pin
    pub pin: String,
    /// A human-readable description of this version, for use in an interactive picker
    pub label: String,
}

/// Fetches every version/file/release of `identifier`'s project that it could be pinned to,
/// newest first
pub async fn available_pins(identifier: &ModIdentifier) -> Result<Vec<PinOption>> {
    Ok(match identifier {
        ModIdentifier::CurseForgeProject(id, _) => {
            let mut files = CURSEFORGE_API.get_mod_files(*id).await?;
            files.sort_unstable_by_key(|f| Reverse(f.file_date));
            files
                .into_iter()
                .map(|f| PinOption {
                    pin: f.id.to_string(),
                    label: format!(
                        "{}  ({})  [{}]",
                        f.display_name,
                        f.file_date.date_naive(),
                        f.game_versions.join(", ")
                    ),
                })
                .collect()
        }
        ModIdentifier::ModrinthProject(id, _) => {
            let mut versions = MODRINTH_API.version_list(id).await?;
            versions.sort_unstable_by_key(|v| Reverse(v.date_published));
            versions
                .into_iter()
                .map(|v| PinOption {
                    pin: v.id.clone(),
                    label: format!(
                        "{}  ({})  [{}]",
                        if v.version_number.is_empty() {
                            v.name.clone()
                        } else {
                            v.version_number.clone()
                        },
                        v.date_published.date_naive(),
                        v.game_versions.join(", ")
                    ),
                })
                .collect()
        }
        ModIdentifier::GitHubRepository((owner, repo), _) => GITHUB_API
            .repos(owner, repo)
            .releases()
            .list()
            .send()
            .await?
            .items
            .into_iter()
            .map(|r| PinOption {
                pin: r.tag_name.clone(),
                label: format!(
                    "{}  ({}){}",
                    r.name.unwrap_or_else(|| r.tag_name.clone()),
                    r.published_at
                        .map_or_else(|| "unpublished".to_owned(), |d| d.date_naive().to_string()),
                    if r.prerelease { "  [prerelease]" } else { "" },
                ),
            })
            .collect(),
    })
}
