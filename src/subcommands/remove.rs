use anyhow::{bail, Result};
use colored::Colorize as _;
use inquire::MultiSelect;
use libium::{
    config::structs::{Mod, ModIdentifier},
    iter_ext::IterExt as _,
};

/// If `to_remove` is empty, display a list of projects in `mods` to select from and remove selected ones
///
/// Else, search the given strings with the projects' name and IDs and remove them
pub fn remove(mods: &mut Vec<Mod>, to_remove: Vec<String>, noun: &str) -> Result<()> {
    let mut indices_to_remove = if to_remove.is_empty() {
        let mod_info = mods
            .iter()
            .map(|mod_| {
                format!(
                    "{:11}  {}{}",
                    match &mod_.identifier {
                        ModIdentifier::CurseForgeProject(id, _) =>
                            format!("CF {:8}", id.to_string()),
                        ModIdentifier::ModrinthProject(id, _) => format!("MR {id:8}"),
                        ModIdentifier::GitHubRepository(..) => "GH".to_string(),
                    },
                    match &mod_.identifier {
                        ModIdentifier::ModrinthProject(..)
                        | ModIdentifier::CurseForgeProject(..) => mod_.name.clone(),
                        ModIdentifier::GitHubRepository((owner, repo), _) =>
                            format!("{owner}/{repo}"),
                    },
                    match &mod_.identifier {
                        ModIdentifier::CurseForgeProject(_, Some(pin)) => format!(" (📌 {pin})"),
                        ModIdentifier::ModrinthProject(_, Some(pin))
                        | ModIdentifier::GitHubRepository(_, Some(pin)) => format!(" (📌 {pin})"),
                        _ => String::new(),
                    },
                )
            })
            .collect_vec();
        MultiSelect::new(&format!("Select {noun} to remove"), mod_info.clone())
            .raw_prompt_skippable()?
            .unwrap_or_default()
            .iter()
            .map(|o| o.index)
            .collect_vec()
    } else {
        let mut items_to_remove = Vec::new();
        for to_remove in to_remove {
            if let Some(index) = mods.iter().position(|mod_| {
                mod_.name.eq_ignore_ascii_case(&to_remove)
                    || match &mod_.identifier {
                        ModIdentifier::CurseForgeProject(id, _) => id.to_string() == to_remove,
                        ModIdentifier::ModrinthProject(id, _) => id == &to_remove,
                        ModIdentifier::GitHubRepository((owner, name), _) => {
                            format!("{owner}/{name}").eq_ignore_ascii_case(&to_remove)
                        }
                    }
                    || mod_
                        .slug
                        .as_ref()
                        .is_some_and(|slug| to_remove.eq_ignore_ascii_case(slug))
            }) {
                items_to_remove.push(index);
            } else {
                bail!("No {noun} with ID or name {to_remove} found in this profile");
            }
        }
        items_to_remove
    };

    indices_to_remove.sort_unstable();
    indices_to_remove.reverse();

    let mut removed = Vec::new();
    for index in indices_to_remove {
        removed.push(mods.remove(index).name);
    }

    if !removed.is_empty() {
        println!(
            "Removed {}",
            removed.iter().map(|txt| txt.bold()).display(", ")
        );
    }

    Ok(())
}
