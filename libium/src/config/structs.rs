use super::filters::Filter;
use derive_more::derive::Display;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    str::FromStr,
};
use url::Url;

pub const CURRENT_CONFIG_VERSION: u32 = 2;

#[derive(Deserialize, Serialize, Debug, Default, Clone)]
pub struct Config {
    #[serde(default)]
    pub version: u32,

    #[serde(skip_serializing_if = "is_zero")]
    #[serde(default)]
    pub active_profile: usize,

    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(default)]
    pub profiles: Vec<Profile>,
}

const fn is_zero(n: &usize) -> bool {
    *n == 0
}

impl Config {
    /// `raw` is the config file's contents parsed generically, kept around so version < 2
    /// migration can recover the `modpacks`/`active_modpack` fields this struct no longer has
    pub(crate) fn migrate(&mut self, raw: &serde_json::Value) {
        if self.version < 1 {
            self.profiles
                .iter_mut()
                .for_each(Profile::backwards_compat);
        }
        if self.version < 2 {
            if let Some(modpacks) = raw.get("modpacks").and_then(serde_json::Value::as_array) {
                for modpack in modpacks {
                    match serde_json::from_value::<LegacyModpack>(modpack.clone()) {
                        Ok(legacy) => {
                            eprintln!(
                                "Migrated modpack '{}' into its own profile; run `hopper upgrade` after switching to it to populate its mods",
                                legacy.name
                            );
                            self.profiles.push(legacy.into());
                        }
                        Err(err) => {
                            eprintln!("WARNING: failed to migrate a legacy modpack entry: {err}");
                        }
                    }
                }
            }
        }
        self.version = CURRENT_CONFIG_VERSION;
    }
}

#[derive(Deserialize, Debug)]
struct LegacyModpack {
    name: String,
    output_dir: PathBuf,
    install_overrides: bool,
    identifier: LegacyModpackIdentifier,
}

#[derive(Deserialize, Debug)]
enum LegacyModpackIdentifier {
    CurseForgeModpack(i32),
    ModrinthModpack(String),
}

impl From<LegacyModpack> for Profile {
    fn from(legacy: LegacyModpack) -> Self {
        Self {
            name: legacy.name.clone(),
            output_dir: legacy.output_dir,
            filters: vec![],
            mods: vec![],
            shaderpacks: vec![],
            resourcepacks: vec![],
            modpacks: vec![ModpackGroup {
                name: legacy.name,
                source: match legacy.identifier {
                    LegacyModpackIdentifier::CurseForgeModpack(id) => {
                        ModpackSource::CurseForgeHosted(id)
                    }
                    LegacyModpackIdentifier::ModrinthModpack(id) => {
                        ModpackSource::ModrinthHosted(id)
                    }
                },
                last_seen_version: None,
                install_overrides: legacy.install_overrides,
                excluded: vec![],
            }],
            game_version: None,
            mod_loader: None,
        }
    }
}

/// A possibly-changing named group of mods tracked within a [`Profile`], sourced from a modpack
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ModpackGroup {
    pub name: String,
    pub source: ModpackSource,

    /// Version id (Modrinth/CurseForge project) or content hash (direct file/URL) last applied —
    /// lets the refresh pass skip work / skip re-applying overrides when the source hasn't
    /// actually changed
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    pub last_seen_version: Option<String>,

    pub install_overrides: bool,

    /// Mods the source currently provides that the user explicitly removed via `hopper remove` —
    /// the refresh pass must not re-add these
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(default)]
    pub excluded: Vec<ModIdentifier>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub enum ModpackSource {
    /// A real Modrinth modpack project — resolved via the Modrinth API, always yields an `.mrpack`
    ModrinthHosted(String),
    /// A real CurseForge modpack project — resolved via the CurseForge API, always yields a
    /// CurseForge-manifest zip
    CurseForgeHosted(i32),
    /// A direct `.mrpack`, from anywhere — GitHub releases, a personal server, a Modrinth CDN
    /// link, a local file, any third-party launcher's export
    MrpackFile(Url),
    /// A direct CurseForge-manifest zip that isn't tied to a CurseForge project page
    CurseForgeZipFile(Url),
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Profile {
    pub name: String,

    /// The directory to download mod files to
    pub output_dir: PathBuf,

    #[serde(default)]
    pub filters: Vec<Filter>,

    pub mods: Vec<Mod>,

    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(default)]
    pub shaderpacks: Vec<Mod>,

    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(default)]
    pub resourcepacks: Vec<Mod>,

    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(default)]
    pub modpacks: Vec<ModpackGroup>,

    #[serde(skip_serializing)]
    game_version: Option<String>,
    #[serde(skip_serializing)]
    mod_loader: Option<ModLoader>,
}

impl Profile {
    /// A simple constructor that automatically deals with converting to filters
    pub fn new(
        name: String,
        output_dir: PathBuf,
        game_versions: Vec<String>,
        mod_loader: ModLoader,
    ) -> Self {
        Self {
            name,
            output_dir,
            filters: vec![
                Filter::ModLoaderPrefer(match mod_loader {
                    ModLoader::Quilt => vec![ModLoader::Quilt, ModLoader::Fabric],
                    _ => vec![mod_loader],
                }),
                Filter::GameVersionStrict(game_versions),
            ],
            mods: vec![],
            shaderpacks: vec![],
            resourcepacks: vec![],
            modpacks: vec![],
            game_version: None,
            mod_loader: None,
        }
    }

    /// Convert the v4 profile's `game_version` and `mod_loader` fields into filters
    pub(crate) fn backwards_compat(&mut self) {
        if let (Some(version), Some(loader)) = (self.game_version.take(), self.mod_loader.take()) {
            self.filters = vec![
                Filter::ModLoaderPrefer(match loader {
                    ModLoader::Quilt => vec![ModLoader::Quilt, ModLoader::Fabric],
                    _ => vec![loader],
                }),
                Filter::GameVersionStrict(vec![version]),
            ];
        }

        for mod_ in &self.mods {
            if mod_.check_game_version.is_some() || mod_.check_mod_loader.is_some() {
                eprintln!("WARNING: Check overrides found for {}", mod_.name);
                eprintln!("Migrate to the new filter system if necessary!");
            }
        }
    }

    pub fn push_mod(
        &mut self,
        kind: ProjectKind,
        name: String,
        identifier: ModIdentifier,
        slug: String,
        override_filters: bool,
        filters: Vec<Filter>,
    ) {
        self.mods_mut(kind).push(Mod {
            name,
            slug: Some(slug),
            identifier,
            filters,
            override_filters,
            source_modpack: None,
            check_game_version: None,
            check_mod_loader: None,
        })
    }

    pub const fn mods(&self, kind: ProjectKind) -> &Vec<Mod> {
        match kind {
            ProjectKind::Mod => &self.mods,
            ProjectKind::ResourcePack => &self.resourcepacks,
            ProjectKind::ShaderPack => &self.shaderpacks,
        }
    }

    pub const fn mods_mut(&mut self, kind: ProjectKind) -> &mut Vec<Mod> {
        match kind {
            ProjectKind::Mod => &mut self.mods,
            ProjectKind::ResourcePack => &mut self.resourcepacks,
            ProjectKind::ShaderPack => &mut self.shaderpacks,
        }
    }

    pub fn dir(&self, kind: ProjectKind) -> PathBuf {
        match kind {
            ProjectKind::Mod => self.output_dir.clone(),
            ProjectKind::ResourcePack => sibling_dir(&self.output_dir, "resourcepacks"),
            ProjectKind::ShaderPack => sibling_dir(&self.output_dir, "shaderpacks"),
        }
    }
}

fn sibling_dir(dir: &Path, name: &str) -> PathBuf {
    dir.parent().unwrap_or(dir).join(name)
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Mod {
    pub name: String,
    pub identifier: ModIdentifier,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,

    /// Custom filters that apply only for this mod
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(default)]
    pub filters: Vec<Filter>,

    /// Whether the filters specified above replace or apply with the profile's filters
    #[serde(skip_serializing_if = "is_false")]
    #[serde(default)]
    pub override_filters: bool,

    /// The name of the [`ModpackGroup`] that added this mod, if any.
    /// `None` means freestanding — added directly by the user, never touched by group
    /// reconciliation.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    pub source_modpack: Option<String>,

    #[serde(skip_serializing)]
    check_game_version: Option<bool>,
    #[serde(skip_serializing)]
    check_mod_loader: Option<bool>,
}

impl Mod {
    pub fn new(
        name: String,
        identifier: ModIdentifier,
        filters: Vec<Filter>,
        override_filters: bool,
    ) -> Self {
        Self {
            name,
            slug: None,
            identifier,
            filters,
            override_filters,
            source_modpack: None,
            check_game_version: None,
            check_mod_loader: None,
        }
    }
}

const fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StringOrInt {
    String(String),
    Int(i32),
}

impl<'de> serde::Deserialize<'de> for StringOrInt {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct StringOrIntVisitor;
        impl<'de> serde::de::Visitor<'de> for StringOrIntVisitor {
            type Value = StringOrInt;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("string or integer")
            }
            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
                Ok(StringOrInt::String(value.to_string()))
            }
            fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
                Ok(StringOrInt::String(value))
            }
            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(StringOrInt::Int(value as i32))
            }
            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(StringOrInt::Int(value as i32))
            }
        }
        deserializer.deserialize_any(StringOrIntVisitor)
    }
}

impl serde::Serialize for StringOrInt {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::String(s) => serializer.serialize_str(s),
            Self::Int(i) => serializer.serialize_i32(*i),
        }
    }
}

impl From<StringOrInt> for String {
    fn from(from: StringOrInt) -> Self {
        match from {
            StringOrInt::String(s) => s,
            StringOrInt::Int(i) => i.to_string(),
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub enum ConfigModIdentifier {
    CurseForgeProject(i32),
    ModrinthProject(String),
    GitHubRepository(String, String),

    PinnedCurseForgeProject(i32, StringOrInt),
    PinnedModrinthProject(String, String),
    PinnedGitHubRepository((String, String), String),
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(from = "ConfigModIdentifier", into = "ConfigModIdentifier")]
pub enum ModIdentifier {
    CurseForgeProject(i32, Option<String>),
    ModrinthProject(String, Option<String>),
    GitHubRepository((String, String), Option<String>),
}

impl From<ConfigModIdentifier> for ModIdentifier {
    fn from(from: ConfigModIdentifier) -> Self {
        match from {
            ConfigModIdentifier::CurseForgeProject(p) => ModIdentifier::CurseForgeProject(p, None),
            ConfigModIdentifier::ModrinthProject(p) => ModIdentifier::ModrinthProject(p, None),
            ConfigModIdentifier::GitHubRepository(o, r) => {
                ModIdentifier::GitHubRepository((o, r), None)
            }
            ConfigModIdentifier::PinnedCurseForgeProject(p, v) => {
                ModIdentifier::CurseForgeProject(p, Some(v.into()))
            }
            ConfigModIdentifier::PinnedModrinthProject(p, v) => {
                ModIdentifier::ModrinthProject(p, Some(v))
            }
            ConfigModIdentifier::PinnedGitHubRepository(p, v) => {
                ModIdentifier::GitHubRepository(p, Some(v))
            }
        }
    }
}

impl From<ModIdentifier> for ConfigModIdentifier {
    fn from(from: ModIdentifier) -> Self {
        match from {
            ModIdentifier::CurseForgeProject(p, None) => ConfigModIdentifier::CurseForgeProject(p),
            ModIdentifier::ModrinthProject(p, None) => ConfigModIdentifier::ModrinthProject(p),
            ModIdentifier::GitHubRepository((o, r), None) => {
                ConfigModIdentifier::GitHubRepository(o, r)
            }
            ModIdentifier::CurseForgeProject(p, Some(v)) => {
                ConfigModIdentifier::PinnedCurseForgeProject(p, StringOrInt::String(v))
            }
            ModIdentifier::ModrinthProject(p, Some(v)) => {
                ConfigModIdentifier::PinnedModrinthProject(p, v)
            }
            ModIdentifier::GitHubRepository(p, Some(v)) => {
                ConfigModIdentifier::PinnedGitHubRepository(p, v)
            }
        }
    }
}

impl ModIdentifier {
    /// Checks if `self` and `other` refer to the same project,
    /// ignoring any differences in pinning.
    pub fn is_same_as(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::CurseForgeProject(l0, _), Self::CurseForgeProject(r0, _)) => l0 == r0,
            (Self::ModrinthProject(l0, _), Self::ModrinthProject(r0, _)) => l0 == r0,
            (Self::GitHubRepository(l0, _), Self::GitHubRepository(r0, _)) => l0 == r0,
            _ => false,
        }
    }

    /// Returns the same project, pinned to `pin`
    #[must_use]
    pub fn with_pin(self, pin: String) -> Self {
        match self {
            Self::CurseForgeProject(p, _) => Self::CurseForgeProject(p, Some(pin)),
            Self::ModrinthProject(p, _) => Self::ModrinthProject(p, Some(pin)),
            Self::GitHubRepository(p, _) => Self::GitHubRepository(p, Some(pin)),
        }
    }

    /// Returns the same project, with any pin removed
    #[must_use]
    pub fn without_pin(self) -> Self {
        match self {
            Self::CurseForgeProject(p, _) => Self::CurseForgeProject(p, None),
            Self::ModrinthProject(p, _) => Self::ModrinthProject(p, None),
            Self::GitHubRepository(p, _) => Self::GitHubRepository(p, None),
        }
    }

    /// Returns the pin, if this project is pinned to one
    pub const fn pin(&self) -> Option<&String> {
        match self {
            Self::CurseForgeProject(_, pin)
            | Self::ModrinthProject(_, pin)
            | Self::GitHubRepository(_, pin) => pin.as_ref(),
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Display, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ModLoader {
    Quilt,
    Fabric,
    Forge,
    #[clap(name = "neoforge")]
    NeoForge,
}

#[derive(thiserror::Error, Debug, PartialEq, Eq)]
#[error("The given string is not a recognised mod loader")]
pub struct ModLoaderParseError;

impl FromStr for ModLoader {
    type Err = ModLoaderParseError;

    fn from_str(from: &str) -> Result<Self, Self::Err> {
        match from.trim().to_lowercase().as_str() {
            "quilt" => Ok(Self::Quilt),
            "fabric" => Ok(Self::Fabric),
            "forge" => Ok(Self::Forge),
            "neoforge" => Ok(Self::NeoForge),
            _ => Err(Self::Err {}),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectKind {
    Mod,
    ResourcePack,
    ShaderPack,
}

impl ProjectKind {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Mod => "mod",
            Self::ResourcePack => "resource pack",
            Self::ShaderPack => "shader pack",
        }
    }

    pub const fn cf_url_segment(self) -> &'static str {
        match self {
            Self::Mod => "mc-mods",
            Self::ResourcePack => "texture-packs",
            Self::ShaderPack => "shaders",
        }
    }

    pub const fn mr_project_type(self) -> ferinth::structures::project::ProjectType {
        use ferinth::structures::project::ProjectType;
        match self {
            Self::Mod => ProjectType::Mod,
            Self::ResourcePack => ProjectType::Resourcepack,
            Self::ShaderPack => ProjectType::Shader,
        }
    }

    pub const fn uses_mod_loader(self) -> bool {
        matches!(self, Self::Mod)
    }

    pub fn applicable_filters(self, filters: Vec<Filter>) -> Vec<Filter> {
        if self.uses_mod_loader() {
            filters
        } else {
            filters
                .into_iter()
                .filter(|f| !matches!(f, Filter::ModLoaderPrefer(_) | Filter::ModLoaderAny(_)))
                .collect()
        }
    }
}
