use crate::{
    config::structs::{ModpackGroup, ModpackSource},
    CURSEFORGE_API, MODRINTH_API,
};
use ferinth::structures::project::{Project, ProjectType};
use furse::structures::mod_structs::Mod;
use reqwest::StatusCode;

type Result<T> = std::result::Result<T, Error>;
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Modpack is already added to profile")]
    AlreadyAdded,
    #[error("The provided modpack does not exist")]
    DoesNotExist,
    #[error("The project is not a modpack")]
    NotAModpack,
    #[error("Modrinth: {0}")]
    ModrinthError(ferinth::Error),
    #[error("CurseForge: {0}")]
    CurseForgeError(furse::Error),
}

impl From<furse::Error> for Error {
    fn from(err: furse::Error) -> Self {
        if let furse::Error::ReqwestError(source) = &err {
            if Some(StatusCode::NOT_FOUND) == source.status() {
                Self::DoesNotExist
            } else {
                Self::CurseForgeError(err)
            }
        } else {
            Self::CurseForgeError(err)
        }
    }
}

impl From<ferinth::Error> for Error {
    fn from(err: ferinth::Error) -> Self {
        if let ferinth::Error::ReqwestError(source) = &err {
            if Some(StatusCode::NOT_FOUND) == source.status() {
                Self::DoesNotExist
            } else {
                Self::ModrinthError(err)
            }
        } else {
            Self::ModrinthError(err)
        }
    }
}

/// Check if the project of `project_id` exists and is a modpack
///
/// Returns the project struct
pub async fn curseforge(existing_groups: &[ModpackGroup], project_id: i32) -> Result<Mod> {
    let project = CURSEFORGE_API.get_mod(project_id).await?;

    if existing_groups.iter().any(|modpack| {
        modpack.name == project.name
            || matches!(&modpack.source, ModpackSource::CurseForgeHosted(id) if *id == project.id)
    }) {
        Err(Error::AlreadyAdded)

    } else if !project.links.website_url.as_str().contains("modpacks") {
        Err(Error::NotAModpack)
    } else {
        Ok(project)
    }
}

/// Check if the project of `project_id` exists and is a modpack
///
/// Returns the project struct
pub async fn modrinth(existing_groups: &[ModpackGroup], project_id: &str) -> Result<Project> {
    let project = MODRINTH_API.project_get(project_id).await?;

    if existing_groups.iter().any(|modpack| {
        modpack.name == project.title
            || matches!(
                &modpack.source,
                ModpackSource::ModrinthHosted(proj_id) if proj_id == &project.id
            )
    }) {
        Err(Error::AlreadyAdded)

    } else if project.project_type != ProjectType::Modpack {
        Err(Error::NotAModpack)
    } else {
        Ok(project)
    }
}
