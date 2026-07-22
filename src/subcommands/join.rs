use crate::{subcommands, subcommands::modpack, TICK};
use anyhow::{Context as _, Result};
use colored::Colorize as _;
use libium::{
    config::structs::{Config, Profile},
    get_minecraft_dir, loader_install,
    modpack::group::{self, Resolution},
};
use std::path::PathBuf;

pub async fn join(
    config: &mut Config,
    identifier: String,
    minecraft_dir: Option<PathBuf>,
    no_install_loader: bool,
) -> Result<()> {
    let minecraft_dir = minecraft_dir.unwrap_or_else(get_minecraft_dir);
    let output_dir = minecraft_dir.join("mods");

    let (source, default_name) = modpack::add::resolve_identifier(&[], &identifier).await?;

    let resolved = match group::resolve(&source, None).await? {
        Resolution::Changed(resolved) => resolved,
        Resolution::Unchanged => {
            unreachable!("a source resolved with no prior last_seen_version is always Changed")
        }
    };
    let game_version = resolved
        .game_version
        .context("Could not determine which Minecraft version this modpack needs")?;
    let mod_loader = resolved
        .mod_loader
        .context("Could not determine which mod loader this modpack needs")?;

    let profile_index = if let Some(i) = config
        .profiles
        .iter()
        .position(|profile| profile.output_dir == output_dir)
    {
        i
    } else {
        config.profiles.push(Profile::new(
            default_name,
            output_dir,
            vec![game_version.clone()],
            mod_loader,
        ));
        config.profiles.len() - 1
    };
    config.active_profile = profile_index;

    if !no_install_loader {
        let version_id =
            loader_install::install_fabric_loader(&minecraft_dir, &game_version).await?;
        let profile_name = config.profiles[profile_index].name.clone();
        loader_install::upsert_launcher_profile(
            &minecraft_dir,
            &format!("hopper-{profile_name}"),
            &profile_name,
            &version_id,
        )?;
        println!("{} Installed Fabric loader {version_id}", *TICK);
    }

    let profile = &mut config.profiles[profile_index];
    modpack::add(profile, identifier, None, false).await?;
    subcommands::upgrade(profile).await?;

    println!("\n{}", "You're all set!".green().bold());
    println!("Add your own mods any time with `hopper add <slug>`.");
    println!("Pick up future server updates with `hopper upgrade`.");

    Ok(())
}
