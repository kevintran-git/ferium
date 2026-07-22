use crate::{
    default_semaphore,
    download::{clean, download},
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
    upgrade::{mod_downloadable, DownloadData},
};
use parking_lot::Mutex;
use std::{
    ffi::OsString,
    fs::read_dir,
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
) -> Result<(Vec<DownloadData>, bool)> {
    let progress_bar = Arc::new(Mutex::new(ProgressBar::new(0).with_style(STYLE_NO.clone())));
    let mut tasks = JoinSet::new();
    let mut done_mods = Vec::new();
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

    for mod_ in mods.to_vec() {
        mod_sender.send(mod_)?;
    }

    let mut initial = true;


    while (Arc::strong_count(&mod_sender) > 1 || initial)
        && !skip_requested.load(Ordering::Relaxed)
    {
        if let Ok(mod_) = mod_rcvr.try_recv() {
            initial = false;

            if done_mods.contains(&mod_.identifier) {
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
                        progress_bar.lock().println(format!(
                            "{} {:pad_len$}  {}",
                            TICK.clone(),
                            mod_.name,
                            download_file.filename().dimmed()
                        ));
                        for dep in take(&mut download_file.dependencies) {
                            dep_sender.send(Mod::new(
                                format!(
                                    "Dependency: {}",
                                    match &dep {
                                        ModIdentifier::CurseForgeProject(id, _) => id.to_string(),
                                        ModIdentifier::ModrinthProject(id, _) => id.to_owned(),
                                        ModIdentifier::GitHubRepository(..) => unreachable!(),
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
                        Ok(Some(download_file))
                    }
                    Err(err) => {
                        if is_rate_limited_or_unauthorised(&err) {
                            progress_bar.lock().finish_and_clear();
                            bail!(err);
                        }
                        progress_bar.lock().println(format!(
                            "{}",
                            format!("{CROSS} {:pad_len$}  {err}", mod_.name).red()
                        ));
                        Ok(None)
                    }
                }
            });
        }
    }

    let (results, any_skipped) = if skip_requested.load(Ordering::Relaxed) {
        progress_bar.lock().println(
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
    let (to_download, error) = get_platform_downloadables(&profile.mods, &profile.filters).await?;
    let mut to_install = Vec::new();
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

    finish_upgrade(&profile.output_dir, to_download, to_install, error).await
}

pub async fn upgrade_packs(profile: &Profile, kind: ProjectKind) -> Result<()> {
    let filters = kind.applicable_filters(profile.filters.clone());
    let (to_download, error) = get_platform_downloadables(profile.mods(kind), &filters).await?;
    finish_upgrade(&profile.dir(kind), to_download, Vec::new(), error).await
}

async fn finish_upgrade(
    output_dir: &Path,
    mut to_download: Vec<DownloadData>,
    mut to_install: Vec<(OsString, PathBuf)>,
    error: bool,
) -> Result<()> {
    clean(output_dir, &mut to_download, &mut to_install).await?;
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
