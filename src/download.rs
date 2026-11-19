use crate::{default_semaphore, SEMAPHORE, STYLE_BYTE, TICK};
use anyhow::{anyhow, bail, Error, Result};
use colored::Colorize as _;
use fs_extra::{
    dir::{copy as copy_dir, CopyOptions as DirCopyOptions},
    file::{move_file, CopyOptions as FileCopyOptions},
};
use indicatif::ProgressBar;
use libium::{iter_ext::IterExt as _, manifest::dedupe_by_manifest_id, upgrade::DownloadData};
use parking_lot::Mutex;
use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    fs::{copy, create_dir_all, read_dir, remove_file},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::task::JoinSet;

/// Print a status line above an active progress bar on a TTY, or directly to stdout when
/// stderr is not a TTY (where `indicatif` hides the progress bar and suppresses `pb.println`).
pub fn progress_println(pb: &ProgressBar, msg: impl AsRef<str>) {
    if pb.is_hidden() {
        println!("{}", msg.as_ref());
    } else {
        pb.println(msg);
    }
}

/// Options controlling how [`clean`] and [`clean_selective`] manage existing files.
#[derive(Default)]
pub struct CleanOptions {
    /// Leave `.txt` sidecar files in place and carry over `<old>.txt` -> `<new>.txt` when a
    /// shaderpack is replaced by a differently named version.
    pub preserve_txt_sidecars: bool,
    /// Filenames that must never be moved to `.old` (for example tracked local files already
    /// in place).
    pub protected_filenames: HashSet<OsString>,
    /// Filenames in `to_install` that should overwrite an existing destination file instead of
    /// being skipped as already up to date.
    pub force_install_filenames: HashSet<OsString>,
}

fn is_txt_file(filename: &str) -> bool {
    Path::new(filename)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("txt"))
}

fn is_part_file(filename: &str) -> bool {
    Path::new(filename)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("part"))
}

fn move_to_old(src: &Path, old_dir: &Path, filename: &str) -> Result<()> {
    create_dir_all(old_dir)?;
    let mut opts = FileCopyOptions::new();
    opts.overwrite = true;
    move_file(src, old_dir.join(filename), &opts)?;
    Ok(())
}

const GENERIC_PACK_TOKENS: &[&str] = &[
    "shader",
    "shaders",
    "shaderpack",
    "pack",
    "resourcepack",
    "beta",
    "alpha",
    "rc",
    "pre",
    "hotfix",
    "fix",
];

fn is_version_token(tok: &str) -> bool {
    let bytes = tok.as_bytes();
    if bytes.is_empty() {
        return true;
    }
    // Preserve resolution markers like "16x", "32x", "64x", "128x" as variant identifiers.
    if bytes.len() >= 2
        && bytes[bytes.len() - 1] == b'x'
        && bytes[..bytes.len() - 1].iter().all(u8::is_ascii_digit)
    {
        return false;
    }
    if bytes[0].is_ascii_digit() {
        return true;
    }
    if matches!(bytes[0], b'v' | b'r' | b'b') && bytes.len() >= 2 && bytes[1].is_ascii_digit() {
        return true;
    }
    if let Some(rest) = tok.strip_prefix("mc") {
        if rest.as_bytes().first().is_some_and(u8::is_ascii_digit) {
            return true;
        }
    }
    false
}

/// Extract normalized identity tokens from a pack filename, stripping extension, version tokens,
/// and generic words like "shader"/"pack".
fn pack_identity_tokens(filename: &str) -> Vec<String> {
    let stem = filename
        .strip_suffix(".zip")
        .or_else(|| filename.strip_suffix(".ZIP"))
        .or_else(|| filename.strip_suffix(".jar"))
        .or_else(|| filename.strip_suffix(".JAR"))
        .unwrap_or(filename);

    stem.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|tok| !tok.is_empty())
        .map(str::to_ascii_lowercase)
        .filter(|tok| !is_version_token(tok) && !GENERIC_PACK_TOKENS.contains(&tok.as_str()))
        .collect()
}

fn common_prefix_len(a: &str, b: &str) -> usize {
    a.bytes()
        .zip(b.bytes())
        .take_while(|(x, y)| x.eq_ignore_ascii_case(y))
        .count()
}

/// Pair old pack filenames with newly downloaded/installed pack filenames that represent the same
/// pack at a different version.
pub fn match_upgraded_packs(old_files: &[String], new_files: &[String]) -> Vec<(String, String)> {
    let old_tokens: Vec<Vec<String>> = old_files.iter().map(|f| pack_identity_tokens(f)).collect();
    let new_tokens: Vec<Vec<String>> = new_files.iter().map(|f| pack_identity_tokens(f)).collect();

    let mut candidates: Vec<(usize, usize, usize)> = Vec::new();
    for (oi, ot) in old_tokens.iter().enumerate() {
        if ot.is_empty() {
            continue;
        }
        for (ni, nt) in new_tokens.iter().enumerate() {
            if nt.is_empty() || ot != nt {
                continue;
            }
            let prefix_score = common_prefix_len(&old_files[oi], &new_files[ni]);
            candidates.push((prefix_score, oi, ni));
        }
    }

    candidates.sort_by(|a, b| b.0.cmp(&a.0));

    let mut used_old = HashSet::new();
    let mut used_new = HashSet::new();
    let mut pairs = Vec::new();
    for (_, oi, ni) in candidates {
        if used_old.insert(oi) && used_new.insert(ni) {
            pairs.push((old_files[oi].clone(), new_files[ni].clone()));
        }
    }
    pairs
}

fn carry_over_sidecar(directory: &Path, old_dir: &Path, old_fn: &str, new_fn: &str) -> Result<()> {
    let old_txt_name = format!("{old_fn}.txt");
    let old_txt_path = directory.join(&old_txt_name);
    if !old_txt_path.is_file() {
        return Ok(());
    }

    let new_txt_name = format!("{new_fn}.txt");
    let new_txt_path = directory.join(&new_txt_name);
    if !new_txt_path.exists() {
        copy(&old_txt_path, &new_txt_path)?;
        println!(
            "{} Carried settings   {} -> {}",
            *TICK,
            old_txt_name.dimmed(),
            new_txt_name.dimmed(),
        );
    }

    move_to_old(&old_txt_path, old_dir, &old_txt_name)?;
    Ok(())
}

/// Check the given `directory`
///
/// - If there are files there that are not in `to_download`, `to_install`, or
///   `options.protected_filenames`, they will be moved to `directory`/.old
/// - If a file in `to_download` or `to_install` is already there, it will be removed from the
///   respective vector (unless listed in `options.force_install_filenames`)
/// - If the file is a `.part` temp file, it will be deleted
/// - When `options.preserve_txt_sidecars` is set, `.txt` files are left in place and any
///   `<old>.txt` belonging to an upgraded pack is copied to `<new>.txt` (without overwriting)
pub async fn clean(
    directory: &Path,
    to_download: &mut Vec<DownloadData>,
    to_install: &mut Vec<(OsString, PathBuf)>,
    options: &CleanOptions,
) -> Result<()> {
    let dupes = find_dupes_by_key(to_download, DownloadData::filename);
    if !dupes.is_empty() {
        println!(
            "{}",
            format!(
                "WARNING: {} duplicate files were found {}. Remove the mod it belongs to",
                dupes.len(),
                dupes
                    .into_iter()
                    .map(|i| to_download.remove(i).filename())
                    .display(", ")
            )
            .yellow()
            .bold()
        );
    }
    if !directory.exists() {
        create_dir_all(directory)?;
        return Ok(());
    }

    let old_dir = directory.join(".old");
    let mut moved_old_files = Vec::new();

    for file in read_dir(directory)? {
        let file = file?;
        if file.file_type()?.is_file() {
            let os_name = file.file_name();
            let filename = os_name.to_string_lossy();
            let filename = filename.as_ref();

            if options.preserve_txt_sidecars && is_txt_file(filename) {
                continue;
            }

            if let Some(index) = to_download
                .iter()
                .position(|thing| filename == thing.filename())
            {
                to_download.remove(index);
            } else if let Some(index) = to_install.iter().position(|thing| os_name == thing.0) {
                if !options.force_install_filenames.contains(&os_name) {
                    to_install.remove(index);
                }
            } else if options.protected_filenames.contains(&os_name) {
                // Keep tracked local files in place.
            } else if is_part_file(filename) {
                remove_file(file.path())?;
            } else {
                move_to_old(&file.path(), &old_dir, filename)?;
                moved_old_files.push(filename.to_owned());
            }
        }
    }

    if options.preserve_txt_sidecars && !moved_old_files.is_empty() {
        let mut new_filenames: Vec<String> = to_download.iter().map(DownloadData::filename).collect();
        new_filenames.extend(
            to_install
                .iter()
                .map(|(name, _)| name.to_string_lossy().into_owned()),
        );

        let pairs = match_upgraded_packs(&moved_old_files, &new_filenames);
        let paired_old: HashMap<String, String> = pairs.into_iter().collect();

        for old_fn in &moved_old_files {
            if let Some(new_fn) = paired_old.get(old_fn) {
                carry_over_sidecar(directory, &old_dir, old_fn, new_fn)?;
            } else {
                let old_txt_name = format!("{old_fn}.txt");
                let old_txt_path = directory.join(&old_txt_name);
                if old_txt_path.is_file() {
                    move_to_old(&old_txt_path, &old_dir, &old_txt_name)?;
                }
            }
        }
    }

    Ok(())
}

/// Selective variant of [`clean`] used when upgrading only specific packs: leaves all unrelated
/// files in `directory` untouched, skips packs that are already up to date, and only moves older
/// versions of the packs being upgraded to `.old` (carrying over `.txt` sidecars when enabled).
pub async fn clean_selective(
    directory: &Path,
    to_download: &mut Vec<DownloadData>,
    to_install: &mut Vec<(OsString, PathBuf)>,
    options: &CleanOptions,
) -> Result<()> {
    let dupes = find_dupes_by_key(to_download, DownloadData::filename);
    for i in dupes {
        to_download.remove(i);
    }

    if !directory.exists() {
        create_dir_all(directory)?;
        return Ok(());
    }

    let mut existing_files = Vec::new();
    for file in read_dir(directory)? {
        let file = file?;
        if file.file_type()?.is_file() {
            let os_name = file.file_name();
            let filename = os_name.to_string_lossy().into_owned();
            if let Some(index) = to_download
                .iter()
                .position(|thing| filename == thing.filename())
            {
                to_download.remove(index);
                continue;
            }
            if let Some(index) = to_install.iter().position(|thing| os_name == thing.0) {
                if !options.force_install_filenames.contains(&os_name) {
                    to_install.remove(index);
                }
                continue;
            }
            if options.protected_filenames.contains(&os_name)
                || is_part_file(&filename)
                || (options.preserve_txt_sidecars && is_txt_file(&filename))
            {
                continue;
            }
            existing_files.push(filename);
        }
    }

    let mut new_filenames: Vec<String> = to_download.iter().map(DownloadData::filename).collect();
    new_filenames.extend(
        to_install
            .iter()
            .map(|(name, _)| name.to_string_lossy().into_owned()),
    );

    if new_filenames.is_empty() || existing_files.is_empty() {
        return Ok(());
    }

    let old_dir = directory.join(".old");
    for (old_fn, new_fn) in match_upgraded_packs(&existing_files, &new_filenames) {
        move_to_old(&directory.join(&old_fn), &old_dir, &old_fn)?;
        if options.preserve_txt_sidecars {
            carry_over_sidecar(directory, &old_dir, &old_fn, &new_fn)?;
        }
    }

    Ok(())
}

/// Read all entries from `directory` into a list of `(filename, path)` pairs for [`install_files`]
pub fn read_overrides(directory: &Path) -> Result<Vec<(OsString, PathBuf)>> {
    let mut to_install = Vec::new();
    if directory.exists() {
        for file in read_dir(directory)? {
            let file = file?;
            to_install.push((file.file_name(), file.path()));
        }
    }
    Ok(to_install)
}

/// Copy each `to_install` file/directory into `output_dir`, overwriting existing files
pub fn install_files(output_dir: &Path, to_install: Vec<(OsString, PathBuf)>) -> Result<()> {
    for (name, path) in to_install {
        if path.is_file() {
            copy(path, output_dir.join(&name))?;
        } else if path.is_dir() {
            let mut copy_options = DirCopyOptions::new();
            copy_options.overwrite = true;
            copy_dir(path, output_dir, &copy_options)?;
        } else {
            bail!("Could not determine whether installable is a file or folder")
        }
        println!(
            "{} Installed          {}",
            *TICK,
            name.to_string_lossy().dimmed()
        );
    }
    Ok(())
}

/// Download and install the files in `to_download` and `to_install` to `output_dir`
pub async fn download(
    output_dir: PathBuf,
    to_download: Vec<DownloadData>,
    to_install: Vec<(OsString, PathBuf)>,
) -> Result<()> {
    let progress_bar = Arc::new(Mutex::new(
        ProgressBar::new(
            to_download
                .iter()
                .map(|downloadable| downloadable.length as u64)
                .sum(),
        )
        .with_style(STYLE_BYTE.clone()),
    ));
    progress_bar
        .lock()
        .enable_steady_tick(Duration::from_millis(100));
    let mut tasks = JoinSet::new();
    let client = reqwest::Client::new();

    let resolved_filenames: HashSet<String> =
        to_download.iter().map(DownloadData::filename).collect();

    for downloadable in to_download {
        let progress_bar = Arc::clone(&progress_bar);
        let client = client.clone();
        let output_dir = output_dir.clone();

        tasks.spawn(async move {
            let _permit = SEMAPHORE.get_or_init(default_semaphore).acquire().await?;

            let (length, filename) = downloadable
                .download(client, &output_dir, |additional| {
                    progress_bar.lock().inc(additional as u64);
                })
                .await?;
            progress_println(
                &progress_bar.lock(),
                format!(
                    "{} Downloaded  {:>7}  {}",
                    *TICK,
                    size::Size::from_bytes(length)
                        .format()
                        .with_base(size::Base::Base10)
                        .to_string(),
                    filename.dimmed(),
                ),
            );
            Ok::<(), Error>(())
        });
    }
    for res in tasks.join_all().await {
        res?;
    }
    Arc::try_unwrap(progress_bar)
        .map_err(|_| anyhow!("Failed to run threads to completion"))?
        .into_inner()
        .finish_and_clear();
    install_files(&output_dir, to_install)?;

    let deduped = dedupe_by_manifest_id(&output_dir, &resolved_filenames)?;
    if !deduped.is_empty() {
        println!(
            "{}",
            format!(
                "WARNING: {} duplicate mod jars were found and moved to .old {}",
                deduped.len(),
                deduped.iter().display(", ")
            )
            .yellow()
            .bold()
        );
    }

    Ok(())
}

/// Find duplicates of the items in `slice` using a value obtained by the `key` closure
///
/// Returns the indices of duplicate items in reverse order for easy removal
fn find_dupes_by_key<T, V, F>(slice: &mut [T], key: F) -> Vec<usize>
where
    V: Eq + Ord,
    F: Fn(&T) -> V,
{
    let mut indices = Vec::new();
    if slice.len() < 2 {
        return indices;
    }
    slice.sort_unstable_by_key(&key);
    for i in 0..(slice.len() - 1) {
        if key(&slice[i]) == key(&slice[i + 1]) {
            indices.push(i);
        }
    }
    indices.reverse();
    indices
}

#[cfg(test)]
#[expect(clippy::unwrap_used)]
mod tests {
    use super::*;
    use reqwest::Url;
    use std::fs::{read_to_string, remove_dir_all, write};

    fn fake_dl(filename: &str) -> DownloadData {
        DownloadData {
            download_url: Url::parse("https://example.invalid/file.zip").unwrap(),
            output: PathBuf::from(filename),
            length: 0,
            dependencies: Vec::new(),
            conflicts: Vec::new(),
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hopper-test-{name}-{:x}",
            rand::random::<u64>()
        ));
        create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn match_upgraded_packs_pairs_versions_without_false_positives() {
        let old = vec![
            "MakeUp-UltraFast-9.5g.zip".to_owned(),
            "MakeUp-UltraFast-9.5c (Custom Sky).zip".to_owned(),
            "ComplementaryUnbound_r5.9.3.zip".to_owned(),
            "ComplementaryReimagined_r5.9.3.zip".to_owned(),
            "Mellow Shader v3.3.zip".to_owned(),
            "Bliss_v2.1.2_(Chocapic13_Shaders_edit).zip".to_owned(),
            "Faithful 32x - 26.2.zip".to_owned(),
        ];
        let new = vec![
            "MakeUp-UltraFast-9.6.zip".to_owned(),
            "ComplementaryUnbound_r5.10.zip".to_owned(),
            "Mellow v3.5.zip".to_owned(),
            "Bliss_v2.2.0_(Chocapic13_Shaders_edit).zip".to_owned(),
            "Faithful 32x - 26.3.zip".to_owned(),
            "Faithful 64x - 26.3.zip".to_owned(),
        ];

        let mut pairs = match_upgraded_packs(&old, &new);
        pairs.sort();
        assert_eq!(
            pairs,
            vec![
                (
                    "Bliss_v2.1.2_(Chocapic13_Shaders_edit).zip".to_owned(),
                    "Bliss_v2.2.0_(Chocapic13_Shaders_edit).zip".to_owned(),
                ),
                (
                    "ComplementaryUnbound_r5.9.3.zip".to_owned(),
                    "ComplementaryUnbound_r5.10.zip".to_owned(),
                ),
                (
                    "Faithful 32x - 26.2.zip".to_owned(),
                    "Faithful 32x - 26.3.zip".to_owned(),
                ),
                (
                    "MakeUp-UltraFast-9.5g.zip".to_owned(),
                    "MakeUp-UltraFast-9.6.zip".to_owned(),
                ),
                (
                    "Mellow Shader v3.3.zip".to_owned(),
                    "Mellow v3.5.zip".to_owned(),
                ),
            ]
        );
    }

    #[tokio::test]
    async fn clean_carries_over_shader_settings_and_preserves_unrelated_txt() {
        let dir = temp_dir("clean-shader-txt");
        let old_dir = dir.join(".old");
        create_dir_all(&old_dir).unwrap();

        // Pre-populate .old with a same-named file to verify we overwrite rather than delete.
        write(old_dir.join("MakeUp-UltraFast-9.5g.zip"), "very-old").unwrap();

        write(dir.join("MakeUp-UltraFast-9.5g.zip"), "zip-95g").unwrap();
        write(dir.join("MakeUp-UltraFast-9.5g.zip.txt"), "SHADOW_QTY=1\n").unwrap();

        write(dir.join("ComplementaryUnbound_r5.9.3.zip"), "zip-593").unwrap();
        write(dir.join("ComplementaryUnbound_r5.9.3.zip.txt"), "OLD_SETTING=1\n").unwrap();
        write(dir.join("ComplementaryUnbound_r5.10.zip.txt"), "KEEP_EXISTING=1\n").unwrap();

        write(dir.join("Bliss_v2.1.2.zip"), "zip-bliss").unwrap();
        write(dir.join("Bliss_v2.1.2.zip.txt"), "BLISS=1\n").unwrap();

        // Preset for an unzipped/Euphoria folder that isn't a tracked zip file.
        write(
            dir.join("ComplementaryUnbound_r5.10 + EuphoriaPatches_1.11.1.txt"),
            "EUPHORIA=1\n",
        )
        .unwrap();

        // Protected local shaderpack + its settings.
        write(dir.join("MakeUp-Custom.zip"), "local-zip").unwrap();
        write(dir.join("MakeUp-Custom.zip.txt"), "CUSTOM=1\n").unwrap();

        // Temp .part file should be deleted; regular file ending in "part" without dot moved to .old.
        write(dir.join("interrupted.zip.part"), "partial").unwrap();
        write(dir.join("my_shader_part"), "not-a-part-ext").unwrap();

        let mut to_download = vec![
            fake_dl("MakeUp-UltraFast-9.6.zip"),
            fake_dl("ComplementaryUnbound_r5.10.zip"),
            fake_dl("Bliss_v2.1.2.zip"),
        ];
        let mut to_install = Vec::new();
        let mut options = CleanOptions {
            preserve_txt_sidecars: true,
            ..CleanOptions::default()
        };
        options
            .protected_filenames
            .insert(OsString::from("MakeUp-Custom.zip"));

        clean(&dir, &mut to_download, &mut to_install, &options)
            .await
            .unwrap();

        // Bliss was already present and should be skipped; the two upgraded packs remain.
        let remaining: Vec<String> = to_download.iter().map(DownloadData::filename).collect();
        assert_eq!(
            remaining,
            vec!["ComplementaryUnbound_r5.10.zip", "MakeUp-UltraFast-9.6.zip"]
        );

        // MakeUp settings carried over to 9.6 and old zip+txt backed up in .old (overwriting old backup).
        assert_eq!(
            read_to_string(dir.join("MakeUp-UltraFast-9.6.zip.txt")).unwrap(),
            "SHADOW_QTY=1\n"
        );
        assert_eq!(
            read_to_string(old_dir.join("MakeUp-UltraFast-9.5g.zip")).unwrap(),
            "zip-95g"
        );
        assert_eq!(
            read_to_string(old_dir.join("MakeUp-UltraFast-9.5g.zip.txt")).unwrap(),
            "SHADOW_QTY=1\n"
        );

        // Existing Complementary r5.10 settings were NOT overwritten.
        assert_eq!(
            read_to_string(dir.join("ComplementaryUnbound_r5.10.zip.txt")).unwrap(),
            "KEEP_EXISTING=1\n"
        );
        assert!(old_dir.join("ComplementaryUnbound_r5.9.3.zip.txt").exists());

        // Unchanged pack, Euphoria folder preset, and protected local pack untouched.
        assert!(dir.join("Bliss_v2.1.2.zip").exists());
        assert_eq!(
            read_to_string(dir.join("Bliss_v2.1.2.zip.txt")).unwrap(),
            "BLISS=1\n"
        );
        assert_eq!(
            read_to_string(dir.join("ComplementaryUnbound_r5.10 + EuphoriaPatches_1.11.1.txt"))
                .unwrap(),
            "EUPHORIA=1\n"
        );
        assert!(dir.join("MakeUp-Custom.zip").exists());
        assert_eq!(
            read_to_string(dir.join("MakeUp-Custom.zip.txt")).unwrap(),
            "CUSTOM=1\n"
        );

        // .part file removed, non-.part file safely moved to .old.
        assert!(!dir.join("interrupted.zip.part").exists());
        assert!(!old_dir.join("interrupted.zip.part").exists());
        assert!(old_dir.join("my_shader_part").exists());

        let _ = remove_dir_all(dir);
    }

    #[tokio::test]
    async fn clean_selective_only_replaces_targeted_pack() {
        let dir = temp_dir("clean-selective");
        write(dir.join("ComplementaryUnbound_r5.9.3.zip"), "cu-old").unwrap();
        write(dir.join("ComplementaryUnbound_r5.9.3.zip.txt"), "CU=1\n").unwrap();
        write(dir.join("MakeUp-UltraFast-9.5g.zip"), "mu-old").unwrap();
        write(dir.join("MakeUp-UltraFast-9.5g.zip.txt"), "MU=1\n").unwrap();

        let mut to_download = vec![fake_dl("ComplementaryUnbound_r5.10.zip")];
        let mut to_install = Vec::new();
        let options = CleanOptions {
            preserve_txt_sidecars: true,
            ..CleanOptions::default()
        };

        clean_selective(&dir, &mut to_download, &mut to_install, &options)
            .await
            .unwrap();

        assert_eq!(to_download.len(), 1);
        assert!(dir.join(".old").join("ComplementaryUnbound_r5.9.3.zip").exists());
        assert_eq!(
            read_to_string(dir.join("ComplementaryUnbound_r5.10.zip.txt")).unwrap(),
            "CU=1\n"
        );
        // Untargeted MakeUp pack and settings untouched.
        assert!(dir.join("MakeUp-UltraFast-9.5g.zip").exists());
        assert_eq!(
            read_to_string(dir.join("MakeUp-UltraFast-9.5g.zip.txt")).unwrap(),
            "MU=1\n"
        );

        let _ = remove_dir_all(dir);
    }
}
