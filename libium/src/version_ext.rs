use ferinth::structures::version::{Version, VersionFile};

pub trait VersionExt {
    /// Gets the primary (or first) version file of a version, or `None` if it has no files
    fn get_version_file(&self) -> Option<&VersionFile>;
    /// Consumes and returns the primary (or first) version file of a version, or `None` if it has no files
    fn into_version_file(self) -> Option<VersionFile>;
}

impl VersionExt for Version {
    fn get_version_file(&self) -> Option<&VersionFile> {
        self.files.iter().find(|f| f.primary).or(self.files.first())
    }

    fn into_version_file(mut self) -> Option<VersionFile> {
        if self.files.is_empty() {
            return None;
        }
        let fallback = self.files.swap_remove(0);
        Some(
            self.files
                .into_iter()
                .find(|f| f.primary)
                .unwrap_or(fallback),
        )
    }
}
