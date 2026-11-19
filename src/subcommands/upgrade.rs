use super::mod_matches;
use crate::{
    default_semaphore,
    download::{clean, clean_selective, download, progress_println, CleanOptions},
    CROSS, SEMAPHORE, STYLE_NO, TICK,
};
use anyhow::{anyhow, bail, Result};
use colored::Colorize as _;
use indicatif::ProgressBar;
use libium::{
    config::{
        filters::{Filter, ProfileParameters as _},
        structs::{Mod, ModIdentifier, ModLoader, Profile, ProjectKind},
    },
    upgrade::{manifest_resolve, mod_downloadable, DownloadData},
};
use parking_lot::Mutex;
use std::{
    collections::HashSet,
    ffi::OsString,
    fs::{metadata, read_dir},
    io::{stdin, IsTerminal as _},
    mem::take,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::Duration,
};
use tokio::task::JoinSet;

/// Whether `err` means every other mod on the same platform will fail identically,
/// so continuing to check them one by one would just repeat the same failure
fn is_rate_limited_or_unauthorised(err: &mod_downloadable::Error) -> bool {
    match err {
        mod_downloadable::Error::ModrinthError(ferinth::Error::RateLimitExceeded(_)) => true,
        mod_downloadable::Error::GitHubError(octocrab::Error::GitHub { source, .. }) => {
            source.status_code == 403 && source.message.to_lowercase().contains("rate limit")
        }
        mod_downloadable::Error::CurseForgeError(furse::Error::ReqwestError(source)) => {
            source.status() == Some(reqwest::StatusCode::FORBIDDEN)
        }
        _ => false,
    }
}

fn file_needs_update(src: &Path, dst: &Path) -> bool {
    let (Ok(src_meta), Ok(dst_meta)) = (metadata(src), metadata(dst)) else {
        return true;
    };
    if src_meta.len() != dst_meta.len() {
        return true;
    }
    match (src_meta.modified(), dst_meta.modified()) {
        (Ok(src_mtime), Ok(dst_mtime)) => src_mtime > dst_mtime,
        _ => false,
    }
}

struct LocalSyncPlan {
    to_install: Vec<(OsString, PathBuf)>,
    protected: HashSet<OsString>,
    force_install: HashSet<OsString>,
    error: bool,
}

fn plan_local_entries(mods: &[&Mod], output_dir: &Path) -> LocalSyncPlan {
    let mut plan = LocalSyncPlan {
        to_install: Vec::new(),
        protected: HashSet::new(),
        force_install: HashSet::new(),
        error: false,
    };

    for mod_ in mods {
        let Some(source_path) = mod_.identifier.local_path() else {
            continue;
        };
        let Some(file_name) = source_path.file_name() else {
            println!(
                "{}",
                format!("{CROSS} {}  Invalid local path: {}", mod_.name, source_path.display())
                    .red()
            );
            plan.error = true;
            continue;
        };
        let os_name = file_name.to_os_string();
        let dest = output_dir.join(file_name);

        if source_path.exists() {
            let same_location = dest
                .canonicalize()
                .ok()
                .zip(source_path.canonicalize().ok())
                .is_some_and(|(d, s)| d == s);
            if same_location {
                plan.protected.insert(os_name);
            } else if !dest.exists() {
                plan.to_install.push((os_name, source_path.to_path_buf()));
            } else if source_path.is_file() && dest.is_file() {
                if file_needs_update(source_path, &dest) {
                    plan.force_install.insert(os_name.clone());
                    plan.to_install.push((os_name, source_path.to_path_buf()));
                } else {
                    plan.protected.insert(os_name);
                }
            } else {
                plan.protected.insert(os_name);
            }
        } else if dest.exists() {
            plan.protected.insert(os_name);
        } else {
            println!(
                "{}",
                format!(
                    "{CROSS} {}  Local file not found: {}",
                    mod_.name,
                    source_path.display()
                )
                .red()
            );
            plan.error = true;
        }
    }

    plan
}

/// Get the latest compatible downloadable for `mods`
///
/// If an error occurs with a resolving task, instead of failing immediately,
/// resolution will continue and the error return flag is set to true.
///
/// Pressing enter on a terminal aborts any still-unresolved mods and
/// continues with whatever has resolved so far.
pub async fn get_platform_downloadables(
    mods: &[Mod],
    filters: &[Filter],
) -> Result<(Vec<(ModIdentifier, DownloadData)>, bool)> {
    if mods.is_empty() {
        return Ok((Vec::new(), false));
    }

    let progress_bar = Arc::new(Mutex::new(ProgressBar::new(0).with_style(STYLE_NO.clone())));
    let mut tasks = JoinSet::new();
    let mut done_mods: Vec<ModIdentifier> = Vec::new();
    let (mod_sender, mod_rcvr) = mpsc::channel();

    let mod_sender = Arc::new(mod_sender);

    println!("{}\n", "Determining the Latest Compatible Versions".bold());

    let skip_requested = Arc::new(AtomicBool::new(false));
    if stdin().is_terminal() {
        println!(
            "{}\n",
            "(Press enter to stop waiting and continue with whatever has resolved so far)"
                .dimmed()
        );
        let skip_requested = Arc::clone(&skip_requested);
        std::thread::spawn(move || {
            let mut line = String::new();
            if stdin().read_line(&mut line).is_ok() {
                skip_requested.store(true, Ordering::Relaxed);
            }
        });
    }

    progress_bar
        .lock()
        .enable_steady_tick(Duration::from_millis(100));
    let pad_len = mods
        .iter()
        .map(|m| m.name.len())
        .max()
        .unwrap_or(20)
        .clamp(20, 50);

    for mod_ in mods.iter().cloned() {
        mod_sender.send(mod_)?;
    }

    let mut initial = true;

    while (Arc::strong_count(&mod_sender) > 1 || initial)
        && !skip_requested.load(Ordering::Relaxed)
    {
        if let Ok(mod_) = mod_rcvr.try_recv() {
            initial = false;

            if done_mods
                .iter()
                .any(|done| done.is_same_as(&mod_.identifier))
            {
                continue;
            }

            done_mods.push(mod_.identifier.clone());
            progress_bar.lock().inc_length(1);

            let filters = filters.to_vec();
            let dep_sender = Arc::clone(&mod_sender);
            let progress_bar = Arc::clone(&progress_bar);

            tasks.spawn(async move {
                let permit = SEMAPHORE.get_or_init(default_semaphore).acquire().await?;

                let result = mod_.fetch_download_file(filters).await;

                drop(permit);

                progress_bar.lock().inc(1);
                match result {
                    Ok(mut download_file) => {
                        progress_println(
                            &progress_bar.lock(),
                            format!(
                                "{} {:pad_len$}  {}",
                                TICK.clone(),
                                mod_.name,
                                download_file.filename().dimmed()
                            ),
                        );
                        let identifier = mod_.identifier.clone();
                        for dep in take(&mut download_file.dependencies) {
                            dep_sender.send(Mod::new(
                                format!(
                                    "Dependency: {}",
                                    match &dep {
                                        ModIdentifier::CurseForgeProject(id, _) => id.to_string(),
                                        ModIdentifier::ModrinthProject(id, _) => id.to_owned(),
                                        ModIdentifier::GitHubRepository(..)
                                        | ModIdentifier::LocalFile(..) => unreachable!(),
                                    }
                                ),
                                match dep {
                                    ModIdentifier::ModrinthProject(id, Some(_)) => {
                                        ModIdentifier::ModrinthProject(id, None)
                                    }
                                    _ => dep,
                                },
                                vec![],
                                false,
                            ))?;
                        }
                        Ok(Some((identifier, download_file)))
                    }
                    Err(err) => {
                        if is_rate_limited_or_unauthorised(&err) {
                            progress_bar.lock().finish_and_clear();
                            bail!(err);
                        }
                        progress_println(
                            &progress_bar.lock(),
                            format!(
                                "{}",
                                format!("{CROSS} {:pad_len$}  {err}", mod_.name).red()
                            ),
                        );
                        Ok(None)
                    }
                }
            });
        }
    }

    let (results, any_skipped) = if skip_requested.load(Ordering::Relaxed) {
        progress_println(
            &progress_bar.lock(),
            "Skipping remaining checks, continuing with what has resolved so far"
                .yellow()
                .bold()
                .to_string(),
        );
        tasks.abort_all();
        let mut results = Vec::new();
        let mut any_skipped = false;
        while let Some(res) = tasks.join_next().await {
            match res {
                Ok(inner) => results.push(inner?),
                Err(join_err) if join_err.is_cancelled() => any_skipped = true,
                Err(join_err) => return Err(join_err.into()),
            }
        }
        (results, any_skipped)
    } else {
        (
            tasks
                .join_all()
                .await
                .into_iter()
                .collect::<Result<Vec<_>>>()?,
            false,
        )
    };

    Arc::try_unwrap(progress_bar)
        .map_err(|_| anyhow!("Failed to run threads to completion"))?
        .into_inner()
        .finish_and_clear();

    let error = any_skipped || results.iter().any(Option::is_none);
    let to_download = results.into_iter().flatten().collect();

    Ok((to_download, error))
}

pub async fn upgrade(profile: &Profile) -> Result<()> {
    let remote_mods: Vec<Mod> = profile
        .mods
        .iter()
        .filter(|m| m.identifier.local_path().is_none())
        .cloned()
        .collect();
    let local_mods: Vec<&Mod> = profile
        .mods
        .iter()
        .filter(|m| m.identifier.local_path().is_some())
        .collect();

    let (mut to_download, remote_error) =
        get_platform_downloadables(&remote_mods, &profile.filters).await?;

    if profile.strict_deps && !to_download.is_empty() {
        println!("\n{}", "Checking Cross-Mod Dependency Requirements".bold());
        let notes = manifest_resolve::apply_strict_deps(&mut to_download, &profile.filters).await;
        for note in notes {
            println!("{}", note.yellow());
        }
    }

    let to_download = to_download.into_iter().map(|(_, dl)| dl).collect();
    let local_plan = plan_local_entries(&local_mods, &profile.output_dir);
    let mut to_install = local_plan.to_install;

    if profile.output_dir.join("user").exists()
        && profile.filters.mod_loader() != Some(&ModLoader::Quilt)
    {
        for file in read_dir(profile.output_dir.join("user"))? {
            let file = file?;
            let path = file.path();
            if path.is_file()
                && path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("jar"))
            {
                to_install.push((file.file_name(), path));
            }
        }
    }

    let options = CleanOptions {
        preserve_txt_sidecars: false,
        protected_filenames: local_plan.protected,
        force_install_filenames: local_plan.force_install,
    };

    finish_upgrade(
        &profile.output_dir,
        to_download,
        to_install,
        &options,
        false,
        remote_error || local_plan.error,
    )
    .await
}

pub async fn upgrade_packs(profile: &Profile, kind: ProjectKind, names: &[String]) -> Result<()> {
    let all_packs = profile.mods(kind);
    let output_dir = profile.dir(kind);
    let preserve_txt_sidecars = kind == ProjectKind::ShaderPack;

    let (selected, selective): (Vec<&Mod>, bool) = if names.is_empty() {
        (all_packs.iter().collect(), false)
    } else {
        let mut indices = Vec::new();
        for name in names {
            let Some(index) = all_packs.iter().position(|pack| mod_matches(pack, name)) else {
                bail!(
                    "No {} with ID or name {name} found in this profile",
                    kind.name()
                );
            };
            if !indices.contains(&index) {
                indices.push(index);
            }
        }
        (indices.into_iter().map(|i| &all_packs[i]).collect(), true)
    };

    let remote_packs: Vec<Mod> = selected
        .iter()
        .filter(|m| m.identifier.local_path().is_none())
        .map(|&m| m.clone())
        .collect();
    let local_packs: Vec<&Mod> = selected
        .iter()
        .filter(|m| m.identifier.local_path().is_some())
        .copied()
        .collect();

    let filters = kind.applicable_filters(profile.filters.clone());
    let (to_download, remote_error) = get_platform_downloadables(&remote_packs, &filters).await?;
    let to_download = to_download.into_iter().map(|(_, dl)| dl).collect();

    let mut local_plan = plan_local_entries(&local_packs, &output_dir);
    if selective {
        // Never treat any other tracked local pack as an old version of a targeted remote pack.
        for pack in all_packs {
            if let Some(file_name) = pack.identifier.local_path().and_then(Path::file_name) {
                local_plan.protected.insert(file_name.to_os_string());
            }
        }
    }

    let options = CleanOptions {
        preserve_txt_sidecars,
        protected_filenames: local_plan.protected,
        force_install_filenames: local_plan.force_install,
    };

    finish_upgrade(
        &output_dir,
        to_download,
        local_plan.to_install,
        &options,
        selective,
        remote_error || local_plan.error,
    )
    .await
}

async fn finish_upgrade(
    output_dir: &Path,
    mut to_download: Vec<DownloadData>,
    mut to_install: Vec<(OsString, PathBuf)>,
    options: &CleanOptions,
    selective: bool,
    error: bool,
) -> Result<()> {
    if selective {
        clean_selective(output_dir, &mut to_download, &mut to_install, options).await?;
    } else {
        clean(output_dir, &mut to_download, &mut to_install, options).await?;
    }
    to_download
        .iter_mut()
        .map(|thing| thing.output = thing.filename().into())
        .for_each(drop);
    if to_download.is_empty() && to_install.is_empty() {
        println!("\n{}", "All up to date!".bold());
    } else {
        println!("\n{}\n", "Downloading Files".bold());
        download(output_dir.to_path_buf(), to_download, to_install).await?;
    }

    if error {
        Err(anyhow!(
            "\nCould not get the latest compatible version of some mods"
        ))
    } else {
        Ok(())
    }
}
