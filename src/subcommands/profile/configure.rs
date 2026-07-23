use super::{check_output_directory, pick_minecraft_versions, pick_mod_loader};
use crate::file_picker::pick_folder;
use anyhow::{Context as _, Result};
use inquire::{Confirm, Select, Text};
use libium::{
    config::filters::ProfileParameters as _,
    config::structs::{ModLoader, Profile},
};
use std::path::PathBuf;

pub async fn configure(
    profile: &mut Profile,
    game_versions: Vec<String>,
    mod_loaders: Vec<ModLoader>,
    name: Option<String>,
    output_dir: Option<PathBuf>,
    strict_deps: Option<bool>,
) -> Result<()> {
    let mut interactive = true;

    if !game_versions.is_empty() {
        *profile
            .filters
            .game_versions_mut()
            .context("Active profile does not filter by game version")? = game_versions;

        interactive = false;
    }
    if !mod_loaders.is_empty() {
        *profile
            .filters
            .mod_loaders_mut()
            .context("Active profile does not filter mod loader")? = mod_loaders;

        interactive = false;
    }
    if let Some(name) = name {
        profile.name = name;
        interactive = false;
    }
    if let Some(output_dir) = output_dir {
        profile.output_dir = output_dir;
        interactive = false;
    }
    if let Some(strict_deps) = strict_deps {
        profile.strict_deps = strict_deps;
        interactive = false;
    }

    if interactive {
        let items = vec![
            "Mods output directory",
            "Minecraft version",
            "Mod loader",
            "Profile Name",
            "Strict dependency resolution",
            "Quit",
        ];

        while let Ok(selection) =
            Select::new("Which setting would you like to change", items.clone()).raw_prompt()
        {
            match selection.index {
                0 => {
                    if let Some(dir) = pick_folder(
                        &profile.output_dir,
                        "Pick an output directory",
                        "Output Directory",
                    )? {
                        check_output_directory(&dir).await?;
                        profile.output_dir = dir;
                    }
                }
                1 => {
                    let Some(versions) = profile.filters.game_versions_mut() else {
                        println!("Active profile does not filter by game version");
                        continue;
                    };

                    if let Ok(selection) = pick_minecraft_versions(versions).await {
                        *versions = selection;
                    }
                }
                2 => {
                    let Some(loaders) = profile.filters.mod_loaders_mut() else {
                        println!("Active profile does not filter mod loader");
                        continue;
                    };

                    if let Ok(selection) = pick_mod_loader(loaders.first()) {
                        *loaders = match selection {
                            ModLoader::Quilt => vec![ModLoader::Quilt, ModLoader::Fabric],
                            loader => vec![loader],
                        }
                    }
                }
                3 => {
                    if let Ok(new_name) = Text::new("Change the profile's name")
                        .with_default(&profile.name)
                        .prompt()
                    {
                        profile.name = new_name;
                    } else {
                        continue;
                    }
                }
                4 => {
                    if let Ok(enabled) = Confirm::new("Enable strict dependency resolution?")
                        .with_default(profile.strict_deps)
                        .prompt()
                    {
                        profile.strict_deps = enabled;
                    }
                }
                5 => break,
                _ => unreachable!(),
            }
            println!();
        }
    }

    Ok(())
}
