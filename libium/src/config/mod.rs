pub mod filters;
pub mod structs;
use std::{
    fs::{copy, create_dir_all, read_to_string, rename, File},
    io::Result,
    path::Path,
};

pub fn read_config(path: impl AsRef<Path>) -> Result<structs::Config> {
    let path = path.as_ref();
    if !path.try_exists()? {
        create_dir_all(path.parent().expect("Invalid config directory"))?;
        write_config(path, &structs::Config::default())?;
    }

    let contents = read_to_string(path)?;
    let mut config: structs::Config = serde_json::from_str(&contents)?;
    let raw: serde_json::Value = serde_json::from_str(&contents)?;

    let version_before = config.version;
    config.migrate(&raw);
    if config.version != version_before {
        write_config(path, &config)?;
    }

    Ok(config)
}

pub fn write_config(path: impl AsRef<Path>, config: &structs::Config) -> Result<()> {
    let path = path.as_ref();
    if path.exists() {
        let mut backup_path = path.as_os_str().to_owned();
        backup_path.push(".bak");
        if let Err(err) = copy(path, backup_path) {
            eprintln!("Warning: failed to back up config before writing: {err}");
        }
    }

    let mut temp_path = path.as_os_str().to_owned();
    temp_path.push(".tmp");
    let temp_path = Path::new(&temp_path);

    let config_file = File::create(temp_path)?;
    serde_json::to_writer_pretty(config_file, config)?;
    rename(temp_path, path)?;
    Ok(())
}
