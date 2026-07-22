use anyhow::{Context as _, Result};
use colored::Colorize as _;
use inquire::Select;
use libium::{
    config::structs::{Profile, ProjectKind},
    iter_ext::IterExt as _,
};

const KINDS: [ProjectKind; 3] = [
    ProjectKind::Mod,
    ProjectKind::ResourcePack,
    ProjectKind::ShaderPack,
];

pub fn remove(profile: &mut Profile, modpack_name: Option<String>, delete_mods: bool) -> Result<()> {
    let selection = if let Some(modpack_name) = modpack_name {
        profile
            .modpacks
            .iter()
            .position(|g| g.name.eq_ignore_ascii_case(&modpack_name))
            .context("No modpack group with that name is tracked in this profile")?
    } else {
        let names = profile
            .modpacks
            .iter()
            .map(|g| g.name.clone())
            .collect_vec();
        match Select::new("Select which modpack group to remove", names).raw_prompt() {
            Ok(selection) => selection.index,
            Err(_) => return Ok(()),
        }
    };

    let group = profile.modpacks.remove(selection);

    for kind in KINDS {
        let bucket = profile.mods_mut(kind);
        if delete_mods {
            bucket.retain(|m| m.source_modpack.as_deref() != Some(group.name.as_str()));
        } else {
            for m in bucket.iter_mut() {
                if m.source_modpack.as_deref() == Some(group.name.as_str()) {
                    m.source_modpack = None;
                }
            }
        }
    }

    println!("{} {}", "Removed modpack group".green(), group.name.bold());
    Ok(())
}
