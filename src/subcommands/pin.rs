use crate::TICK;
use anyhow::{ensure, Context as _, Result};
use colored::Colorize as _;
use inquire::{MultiSelect, Select};
use libium::{
    config::structs::{Mod, ModIdentifier, Profile, ProjectKind},
    iter_ext::IterExt as _,
    pin::available_pins,
};

fn mod_matches(mod_: &Mod, name: &str) -> bool {
    mod_.name.eq_ignore_ascii_case(name)
        || match &mod_.identifier {
            ModIdentifier::CurseForgeProject(id, _) => id.to_string() == name,
            ModIdentifier::ModrinthProject(id, _) => id == name,
            ModIdentifier::GitHubRepository((owner, repo), _) => {
                format!("{owner}/{repo}").eq_ignore_ascii_case(name)
            }
        }
        || mod_
            .slug
            .as_ref()
            .is_some_and(|slug| name.eq_ignore_ascii_case(slug))
}

fn mod_label(mod_: &Mod) -> String {
    format!(
        "{}{}",
        match &mod_.identifier {
            ModIdentifier::ModrinthProject(..) | ModIdentifier::CurseForgeProject(..) =>
                mod_.name.clone(),
            ModIdentifier::GitHubRepository((owner, repo), _) => format!("{owner}/{repo}"),
        },
        mod_.identifier
            .pin()
            .map_or(String::new(), |pin| format!(" (📌 {pin})")),
    )
}

/// Resolves `name` to the index of the mod it refers to, or prompts the user to pick one
fn resolve_one(mods: &[Mod], name: Option<String>, noun: &str) -> Result<usize> {
    if let Some(name) = name {
        mods.iter()
            .position(|mod_| mod_matches(mod_, &name))
            .with_context(|| format!("No {noun} with ID or name {name} found in this profile"))
    } else {
        let labels = mods.iter().map(mod_label).collect_vec();
        Select::new(&format!("Select the {noun} to pin"), labels)
            .raw_prompt()
            .map(|selection| selection.index)
            .context("No selection was made")
    }
}

/// Pins `mod_name` (or an interactively picked mod, if `mod_name` is absent) to `version`
/// (or an interactively picked version, if `version` is absent)
pub async fn pin(
    profile: &mut Profile,
    kind: ProjectKind,
    mod_name: Option<String>,
    version: Option<String>,
    noun: &str,
) -> Result<()> {
    let index = resolve_one(profile.mods(kind), mod_name, noun)?;
    let mod_ = &profile.mods(kind)[index];
    let identifier = mod_.identifier.clone();
    let name = mod_.name.clone();

    let options = available_pins(&identifier).await?;
    ensure!(
        !options.is_empty(),
        "{name} has no available versions to pin to"
    );

    let chosen = if let Some(version) = version {
        options
            .iter()
            .find(|o| o.pin == version)
            .or_else(|| {
                let version = version.to_lowercase();
                options
                    .iter()
                    .find(|o| o.label.to_lowercase().contains(&version))
            })
            .cloned()
            .with_context(|| format!("No version matching `{version}` was found for {name}"))?
    } else {
        let labels = options.iter().map(|o| o.label.clone()).collect_vec();
        let selection = Select::new(&format!("Select the version to pin {name} to"), labels)
            .raw_prompt()
            .context("No selection was made")?;
        options[selection.index].clone()
    };

    profile.mods_mut(kind)[index].identifier = identifier.with_pin(chosen.pin);
    println!("{} Pinned {} to {}", *TICK, name.bold(), chosen.label.dimmed());
    Ok(())
}

/// Unpins `to_unpin` (or interactively picked mods, if `to_unpin` is empty)
pub fn unpin(
    profile: &mut Profile,
    kind: ProjectKind,
    to_unpin: Vec<String>,
    noun: &str,
) -> Result<()> {
    let mods = profile.mods(kind);
    let indices = if to_unpin.is_empty() {
        let pinned = mods
            .iter()
            .enumerate()
            .filter(|(_, mod_)| mod_.identifier.pin().is_some())
            .map(|(i, _)| i)
            .collect_vec();
        ensure!(!pinned.is_empty(), "No {noun}s are currently pinned");

        let labels = pinned.iter().map(|&i| mod_label(&mods[i])).collect_vec();
        MultiSelect::new(&format!("Select {noun}s to unpin"), labels)
            .raw_prompt_skippable()?
            .unwrap_or_default()
            .iter()
            .map(|o| pinned[o.index])
            .collect_vec()
    } else {
        let mut indices = Vec::new();
        for name in to_unpin {
            let index = mods
                .iter()
                .position(|mod_| mod_matches(mod_, &name))
                .with_context(|| {
                    format!("No {noun} with ID or name {name} found in this profile")
                })?;
            indices.push(index);
        }
        indices
    };

    let mut unpinned = Vec::new();
    for index in indices {
        let target = &mut profile.mods_mut(kind)[index];
        if target.identifier.pin().is_some() {
            target.identifier = target.identifier.clone().without_pin();
            unpinned.push(target.name.clone());
        }
    }

    if unpinned.is_empty() {
        println!("Nothing to unpin");
    } else {
        println!(
            "Unpinned {}",
            unpinned.iter().map(|name| name.bold()).display(", ")
        );
    }
    Ok(())
}
