use super::switch;
use anyhow::{Context as _, Result};
use colored::Colorize as _;
use inquire::{Confirm, Select};
use libium::{
    config::{filters::ProfileParameters as _, structs::Config},
    iter_ext::IterExt as _,
};
use std::{cmp::Ordering, fs::remove_dir_all};

pub fn delete(
    config: &mut Config,
    profile_name: Option<String>,
    switch_to: Option<String>,
) -> Result<()> {
    let selection = if let Some(profile_name) = profile_name {
        config
            .profiles
            .iter()
            .position(|profile| profile.name.eq_ignore_ascii_case(&profile_name))
            .context("The profile name provided does not exist")?
    } else {
        let profile_names = config
            .profiles
            .iter()
            .map(|profile| {
                format!(
                    "{:6} {:7} {} {}",
                    profile
                        .filters
                        .mod_loader()
                        .map(ToString::to_string)
                        .unwrap_or_default()
                        .purple(),
                    profile
                        .filters
                        .game_versions()
                        .map(|v| v.iter().display(", "))
                        .unwrap_or_default()
                        .green(),
                    profile.name.bold(),
                    format!("({} mods)", profile.mods.len()).yellow(),
                )
            })
            .collect_vec();

        if let Ok(selection) = Select::new("Select which profile to delete", profile_names)
            .with_starting_cursor(config.active_profile)
            .raw_prompt()
        {
            selection.index
        } else {
            return Ok(());
        }
    };
    let removed_profile = config.profiles.remove(selection);

    if removed_profile.output_dir.is_dir()
        && Confirm::new(&format!(
            "Also delete the output directory `{}`?",
            removed_profile.output_dir.display()
        ))
        .with_default(false)
        .prompt()
        .unwrap_or_default()
    {
        remove_dir_all(&removed_profile.output_dir)?;
        println!("{}", "Output directory deleted".yellow());
    }

    match config.active_profile.cmp(&selection) {
        Ordering::Equal => {
            if config.profiles.len() > 1 {
                switch(config, switch_to)?;
            } else {
                config.active_profile = 0;
            }
        }
        Ordering::Greater => {
            config.active_profile -= 1;
        }
        Ordering::Less => (),
    }

    Ok(())
}
