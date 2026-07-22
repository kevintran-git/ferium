pub mod filters;
pub mod structs;
use std::{
    fs::{create_dir_all, rename, File},
    io::{BufReader, Result},
    path::Path,
};

/// Open the config file at `path` and deserialise it into a config struct
pub fn read_config(path: impl AsRef<Path>) -> Result<structs::Config> {
    let path = match path.as_ref().canonicalize() {
        Ok(path) => path,
        Err(_) => {
            create_dir_all(path.as_ref().parent().expect("Invalid config directory"))?;
            write_config(&path, &structs::Config::default())?;
            path.as_ref().to_owned()
        }
    };

    let config_file = BufReader::new(File::open(&path)?);
    let mut config: structs::Config = serde_json::from_reader(config_file)?;

    config.migrate();

    Ok(config)
}

/// Serialise `config` and write it to the config file at `path`
pub fn write_config(path: impl AsRef<Path>, config: &structs::Config) -> Result<()> {
    let path = path.as_ref();
    let mut temp_path = path.as_os_str().to_owned();
    temp_path.push(".tmp");
    let temp_path = Path::new(&temp_path);

    let config_file = File::create(temp_path)?;
    serde_json::to_writer_pretty(config_file, config)?;
    rename(temp_path, path)?;
    Ok(())
}
