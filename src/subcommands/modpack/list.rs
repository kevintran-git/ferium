use colored::Colorize as _;
use libium::config::structs::{ModpackGroup, ModpackSource, Profile, ProjectKind};

const KINDS: [ProjectKind; 3] = [
    ProjectKind::Mod,
    ProjectKind::ResourcePack,
    ProjectKind::ShaderPack,
];

pub fn list(profile: &Profile) {
    for group in &profile.modpacks {
        let mod_count = KINDS
            .iter()
            .map(|&kind| {
                profile
                    .mods(kind)
                    .iter()
                    .filter(|m| m.source_modpack.as_deref() == Some(group.name.as_str()))
                    .count()
            })
            .sum::<usize>();

        info(group, mod_count);
    }
}

pub fn info(group: &ModpackGroup, mod_count: usize) {
    println!(
        "{}
        \r  Source:            {}
        \r  Mods tracked:      {}
        \r  Last synced:       {}
        \r  Install overrides: {}\n",
        group.name.bold(),
        match &group.source {
            ModpackSource::CurseForgeHosted(id) =>
                format!("{:10} {}", "CurseForge".red(), id.to_string().dimmed()),
            ModpackSource::ModrinthHosted(id) =>
                format!("{:10} {}", "Modrinth".green(), id.dimmed()),
            ModpackSource::MrpackFile(url) =>
                format!("{:10} {}", "mrpack".cyan(), url.as_str().dimmed()),
            ModpackSource::CurseForgeZipFile(url) =>
                format!("{:10} {}", "CF zip".magenta(), url.as_str().dimmed()),
        },
        mod_count,
        group
            .last_seen_version
            .as_deref()
            .unwrap_or("never")
            .dimmed(),
        group.install_overrides
    );
}
