#![expect(clippy::unwrap_used)]

use crate::{
    actual_main,
    cli::{
        Hopper, FilterArguments, ModpackSubCommands, PackSubCommands, Platform,
        ProfileSubCommands, SubCommands,
    },
};
use libium::config::structs::ModLoader;
use std::{
    env::current_dir,
    fs::{copy, create_dir_all, read_to_string, remove_dir_all, write},
    path::PathBuf,
};

const DEFAULT: Hopper = Hopper {
    subcommand: SubCommands::Profile { subcommand: None },
    threads: None,
    parallel_tasks: 10,
    github_token: None,
    curseforge_api_key: None,
    config_file: None,
};

fn get_args(subcommand: SubCommands, config_file: Option<&str>) -> Hopper {
    let running = PathBuf::from(".")
        .join("tests")
        .join("configs")
        .join("running")
        .join(format!("{:X}.json", rand::random::<u32>()));
    let _ = create_dir_all(running.parent().unwrap());
    if let Some(config_file) = config_file {
        copy(format!("./tests/configs/{config_file}.json"), &running).unwrap();
    }
    Hopper {
        subcommand,
        config_file: Some(running),
        ..DEFAULT
    }
}


#[tokio::test(flavor = "multi_thread")]
async fn create_profile_no_profiles_to_import() {
    assert!(matches!(
        actual_main(get_args(
            SubCommands::Profile {
                subcommand: Some(ProfileSubCommands::Create {
                    import: Some(None),
                    game_version: vec!["1.21.4".to_owned()],
                    mod_loader: Some(ModLoader::Fabric),
                    name: Some("Test Profile".to_owned()),
                    output_dir: Some(current_dir().unwrap().join("tests").join("mods")),
                })
            },
            None,
        ))
        .await,
        Err(_),
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn create_profile_rel_dir() {
    assert!(matches!(
        actual_main(get_args(
            SubCommands::Profile {
                subcommand: Some(ProfileSubCommands::Create {
                    import: Some(None),
                    game_version: vec!["1.21.4".to_owned()],
                    mod_loader: Some(ModLoader::Fabric),
                    name: Some("Test Profile".to_owned()),
                    output_dir: Some(PathBuf::from(".").join("tests").join("mods")),
                })
            },
            None,
        ))
        .await,
        Err(_),
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn create_profile_import_mods() {
    assert!(matches!(
        actual_main(get_args(
            SubCommands::Profile {
                subcommand: Some(ProfileSubCommands::Create {
                    import: Some(Some("Default Modded".to_owned())),
                    game_version: vec!["1.21.4".to_owned()],
                    mod_loader: Some(ModLoader::Fabric),
                    name: Some("Test Profile".to_owned()),
                    output_dir: Some(current_dir().unwrap().join("tests").join("mods")),
                })
            },
            Some("one_profile_full"),
        ))
        .await,
        Ok(()),
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn create_profile_existing_name() {
    assert!(matches!(
        actual_main(get_args(
            SubCommands::Profile {
                subcommand: Some(ProfileSubCommands::Create {
                    import: None,
                    game_version: vec!["1.21.4".to_owned()],
                    mod_loader: Some(ModLoader::Fabric),
                    name: Some("Default Modded".to_owned()),
                    output_dir: Some(current_dir().unwrap().join("tests").join("mods"))
                })
            },
            None,
        ))
        .await,
        Ok(()),
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn create_profile() {
    assert!(matches!(
        actual_main(get_args(
            SubCommands::Profile {
                subcommand: Some(ProfileSubCommands::Create {
                    import: None,
                    game_version: vec!["1.21.4".to_owned()],
                    mod_loader: Some(ModLoader::Fabric),
                    name: Some("Test Profile".to_owned()),
                    output_dir: Some(current_dir().unwrap().join("tests").join("mods"))
                })
            },
            None,
        ))
        .await,
        Ok(()),
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn add_modrinth() {
    assert!(matches!(
        actual_main(get_args(
            SubCommands::Add {
                identifiers: vec!["starlight".to_owned()],
                force: false,
                filters: FilterArguments::default(),
            },
            Some("empty_profile"),
        ))
        .await,
        Ok(()),
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn add_curseforge() {
    assert!(matches!(
        actual_main(get_args(
            SubCommands::Add {
                identifiers: vec!["591388".to_owned()],
                force: false,
                filters: FilterArguments::default(),
            },
            Some("empty_profile"),
        ))
        .await,
        Ok(()),
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn add_github() {
    assert!(matches!(
        actual_main(get_args(
            SubCommands::Add {
                identifiers: vec!["CaffeineMC/sodium".to_owned()],
                force: false,
                filters: FilterArguments::default(),
            },
            Some("empty_profile"),
        ))
        .await,
        Ok(()),
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn add_shaderpack() {
    assert!(matches!(
        actual_main(get_args(
            SubCommands::Shaderpack {
                subcommand: Some(PackSubCommands::Add {
                    identifiers: vec!["complementary-reimagined".to_owned()],
                    force: true,
                    filters: Box::default(),
                }),
            },
            Some("empty_profile"),
        ))
        .await,
        Ok(()),
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn add_resourcepack() {
    assert!(matches!(
        actual_main(get_args(
            SubCommands::Resourcepack {
                subcommand: Some(PackSubCommands::Add {
                    identifiers: vec!["fresh-animations".to_owned()],
                    force: true,
                    filters: Box::default(),
                }),
            },
            Some("empty_profile"),
        ))
        .await,
        Ok(()),
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn add_shaderpack_wrong_kind() {
    assert!(matches!(
        actual_main(get_args(
            SubCommands::Shaderpack {
                subcommand: Some(PackSubCommands::Add {
                    identifiers: vec!["starlight".to_owned()],
                    force: true,
                    filters: Box::default(),
                }),
            },
            Some("empty_profile"),
        ))
        .await,
        Err(_),
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn add_resolves_dependencies() {
    let args = get_args(
        SubCommands::Add {
            identifiers: vec!["iris".to_owned()],
            force: true,
            filters: FilterArguments::default(),
        },
        Some("empty_profile"),
    );
    let config_file = args.config_file.clone().unwrap();
    assert!(matches!(actual_main(args).await, Ok(())));

    let config = read_to_string(config_file).unwrap();
    assert!(config.contains("AANobbMI"));
}

#[tokio::test(flavor = "multi_thread")]
async fn add_game_version_range() {
    assert!(matches!(
        actual_main(get_args(
            SubCommands::Add {
                identifiers: vec!["carpet".to_owned()],
                force: false,
                filters: FilterArguments {
                    game_version_range: Some("1.18..1.19".to_owned()),
                    ..FilterArguments::default()
                },
            },
            Some("empty_profile"),
        ))
        .await,
        Ok(()),
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn add_game_version_range_unknown_bound() {
    assert!(matches!(
        actual_main(get_args(
            SubCommands::Add {
                identifiers: vec!["carpet".to_owned()],
                force: false,
                filters: FilterArguments {
                    game_version_range: Some("9.99..9.999".to_owned()),
                    ..FilterArguments::default()
                },
            },
            Some("empty_profile"),
        ))
        .await,
        Err(_),
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn add_all() {
    assert!(matches!(
        actual_main(get_args(
            SubCommands::Add {
                identifiers: vec![
                    "starlight".to_owned(),
                    "591388".to_owned(),
                    "CaffeineMC/sodium".to_owned()
                ],
                force: false,
                filters: FilterArguments::default(),
            },
            Some("empty_profile"),
        ))
        .await,
        Ok(()),
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn already_added() {
    assert!(matches!(
        actual_main(get_args(
            SubCommands::Add {
                identifiers: vec![
                    "starlight".to_owned(),
                    "591388".to_owned(),
                    "CaffeineMC/sodium".to_owned()
                ],
                force: false,
                filters: FilterArguments::default(),
            },
            Some("one_profile_full"),
        ))
        .await,
        Ok(()),
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn add_all_pinned() {
    assert!(matches!(
        actual_main(get_args(
            SubCommands::Add {
                identifiers: vec![
                    "starlight:HZYU0kdg".to_owned(),
                    "591388:6713391".to_owned(),
                    "CaffeineMC/sodium:RA_kwDODijHac4Kh-Lc".to_owned()
                ],
                force: false,
                filters: FilterArguments::default(),
            },
            Some("empty_profile"),
        ))
        .await,
        Ok(()),
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn add_all_invalid_pins() {
    assert!(matches!(
        actual_main(get_args(
            SubCommands::Add {
                identifiers: vec![
                    "starlight:ihzX2Dvy".to_owned(),
                    "591388:4947005".to_owned(),
                    "CaffeineMC/sodium:kwDODijHac4Kh".to_owned()
                ],
                force: false,
                filters: FilterArguments::default(),
            },
            Some("empty_profile"),
        ))
        .await,
        Err(_),
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn scan() {
    assert!(matches!(
        actual_main(get_args(
            SubCommands::Scan {
                platform: Platform::default(),
                directory: Some(current_dir().unwrap().join("tests").join("test_mods")),
                force: false,
            },
            Some("empty_profile"),
        ))
        .await,
        Ok(()),
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn scan_ignores_hidden_files() {
    let args = get_args(
        SubCommands::Scan {
            platform: Platform::default(),
            directory: Some(current_dir().unwrap().join("tests").join("test_mods_hidden")),
            force: false,
        },
        Some("empty_profile"),
    );
    let config_file = args.config_file.clone().unwrap();
    assert!(matches!(actual_main(args).await, Ok(())));

    let config = read_to_string(config_file).unwrap();
    assert!(config.contains("\"mods\": []"));
}

#[tokio::test(flavor = "multi_thread")]
async fn modpack_add_modrinth() {
    let args = get_args(
        SubCommands::Modpack {
            subcommand: Some(ModpackSubCommands::Add {
                identifier: "1KVo5zza".to_owned(),
                name: None,
                no_overrides: false,
            }),
        },
        Some("empty_profile"),
    );
    let config_file = args.config_file.clone().unwrap();
    assert!(matches!(actual_main(args).await, Ok(())));

    let config = read_to_string(config_file).unwrap();
    assert!(config.contains("ModrinthHosted"));
}

#[tokio::test(flavor = "multi_thread")]
async fn modpack_add_curseforge() {
    let args = get_args(
        SubCommands::Modpack {
            subcommand: Some(ModpackSubCommands::Add {
                identifier: "452013".to_owned(),
                name: None,
                no_overrides: false,
            }),
        },
        Some("empty_profile"),
    );
    let config_file = args.config_file.clone().unwrap();
    assert!(matches!(actual_main(args).await, Ok(())));

    let config = read_to_string(config_file).unwrap();
    assert!(config.contains("CurseForgeHosted"));
}

#[tokio::test(flavor = "multi_thread")]
async fn modpack_add_duplicate_name() {
    let args = get_args(
        SubCommands::Modpack {
            subcommand: Some(ModpackSubCommands::Add {
                identifier: "1KVo5zza".to_owned(),
                name: Some("Default Modded".to_owned()),
                no_overrides: true,
            }),
        },
        Some("empty_profile"),
    );
    assert!(matches!(actual_main(args.clone()).await, Ok(())));
    assert!(matches!(actual_main(args).await, Err(_)));
}

#[tokio::test(flavor = "multi_thread")]
async fn modpack_list() {
    assert!(matches!(
        actual_main(get_args(
            SubCommands::Modpack {
                subcommand: Some(ModpackSubCommands::List)
            },
            Some("one_profile_with_modpack")
        ))
        .await,
        Ok(()),
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn modpack_remove_detaches_mods_by_default() {
    let args = get_args(
        SubCommands::Modpack {
            subcommand: Some(ModpackSubCommands::Remove {
                modpack_name: Some("Cobblemon".to_owned()),
                delete_mods: false,
            }),
        },
        Some("one_profile_with_modpack"),
    );
    let config_file = args.config_file.clone().unwrap();
    assert!(matches!(actual_main(args).await, Ok(())));

    let config = read_to_string(config_file).unwrap();
    assert!(!config.contains("\"modpacks\""));
    assert!(config.contains("Sodium"));
}

#[tokio::test(flavor = "multi_thread")]
async fn modpack_remove_can_delete_mods() {
    let args = get_args(
        SubCommands::Modpack {
            subcommand: Some(ModpackSubCommands::Remove {
                modpack_name: Some("Cobblemon".to_owned()),
                delete_mods: true,
            }),
        },
        Some("one_profile_with_modpack"),
    );
    let config_file = args.config_file.clone().unwrap();
    assert!(matches!(actual_main(args).await, Ok(())));

    let config = read_to_string(config_file).unwrap();
    assert!(!config.contains("Sodium"));
}

#[tokio::test(flavor = "multi_thread")]
async fn migrate_legacy_modpacks_into_profiles() {
    let args = get_args(
        SubCommands::Profile {
            subcommand: Some(ProfileSubCommands::List),
        },
        Some("legacy_modpacks"),
    );
    let config_file = args.config_file.clone().unwrap();
    assert!(matches!(actual_main(args).await, Ok(())));

    let config = read_to_string(config_file).unwrap();
    assert!(config.contains("CurseForgeHosted"));
    assert!(config.contains("ModrinthHosted"));
    assert!(config.contains("\"version\": 2"));
}

#[tokio::test(flavor = "multi_thread")]
async fn list_no_profile() {
    assert!(matches!(
        actual_main(get_args(
            SubCommands::List {
                verbose: false,
                markdown: false
            },
            Some("empty"),
        ))
        .await,
        Err(_),
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn list_empty_profile() {
    assert!(matches!(
        actual_main(get_args(
            SubCommands::List {
                verbose: false,
                markdown: false
            },
            Some("empty_profile"),
        ))
        .await,
        Err(_),
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn list() {
    assert!(matches!(
        actual_main(get_args(
            SubCommands::List {
                verbose: false,
                markdown: false
            },
            Some("one_profile_full"),
        ))
        .await,
        Ok(()),
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn list_verbose() {
    assert!(matches!(
        actual_main(get_args(
            SubCommands::List {
                verbose: true,
                markdown: false
            },
            Some("one_profile_full"),
        ))
        .await,
        Ok(()),
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn list_markdown() {
    assert!(matches!(
        actual_main(get_args(
            SubCommands::List {
                verbose: true,
                markdown: true
            },
            Some("one_profile_full"),
        ))
        .await,
        Ok(()),
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn list_profiles() {
    assert!(matches!(
        actual_main(get_args(
            SubCommands::Profiles,
            Some("two_profiles_one_empty"),
        ))
        .await,
        Ok(()),
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn upgrade() {
    assert!(matches!(
        actual_main(get_args(SubCommands::Upgrade, Some("one_profile_full"))).await,
        Ok(()),
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn upgrade_refreshes_modpack_groups() {
    assert!(matches!(
        actual_main(get_args(
            SubCommands::Upgrade,
            Some("one_profile_with_modpack")
        ))
        .await,
        Ok(()),
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn profile_switch() {
    assert!(matches!(
        actual_main(get_args(
            SubCommands::Profile {
                subcommand: Some(ProfileSubCommands::Switch {
                    profile_name: Some("Profile Two".to_owned())
                })
            },
            Some("two_profiles_one_empty")
        ))
        .await,
        Ok(()),
    ));
}


#[tokio::test(flavor = "multi_thread")]
async fn remove_fail() {
    assert!(matches!(
        actual_main(get_args(
            SubCommands::Remove {
                mod_names: vec![
                    "starlght (fabric)".to_owned(),
                    "incendum".to_owned(),
                    "sodum".to_owned(),
                ]
            },
            Some("two_profiles_one_empty")
        ))
        .await,
        Err(_),
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn remove_name() {
    assert!(matches!(
        actual_main(get_args(
            SubCommands::Remove {
                mod_names: vec![
                    "starlight (fabric)".to_owned(),
                    "incendium".to_owned(),
                    "sodium".to_owned(),
                ]
            },
            Some("two_profiles_one_empty")
        ))
        .await,
        Ok(()),
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn remove_id() {
    assert!(matches!(
        actual_main(get_args(
            SubCommands::Remove {
                mod_names: vec![
                    "H8CaAYZC".to_owned(),
                    "591388".to_owned(),
                    "caffeinemc/sodium".to_owned(),
                ]
            },
            Some("two_profiles_one_empty")
        ))
        .await,
        Ok(()),
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn remove_slug() {
    let mut args = get_args(
        SubCommands::List {
            verbose: true,
            markdown: false,
        },
        Some("two_profiles_one_empty"),
    );
    assert!(matches!(actual_main(args.clone()).await, Ok(())));

    args.subcommand = SubCommands::Remove {
        mod_names: vec![
            "starlight".to_owned(),
            "incendium".to_owned(),
            "sodium".to_owned(),
        ],
    };
    assert!(matches!(actual_main(args).await, Ok(())));
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_profile() {
    assert!(matches!(
        actual_main(get_args(
            SubCommands::Profile {
                subcommand: Some(ProfileSubCommands::Delete {
                    profile_name: Some("Profile Two".to_owned()),
                    switch_to: None
                })
            },
            Some("two_profiles_one_empty")
        ))
        .await,
        Ok(()),
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_profile_keeps_output_dir_without_confirmation() {
    let output_dir = current_dir()
        .unwrap()
        .join("tests")
        .join("profile_delete_output");
    create_dir_all(&output_dir).unwrap();
    write(output_dir.join("marker.txt"), b"keep me").unwrap();

    assert!(matches!(
        actual_main(get_args(
            SubCommands::Profile {
                subcommand: Some(ProfileSubCommands::Delete {
                    profile_name: Some("Solo Profile".to_owned()),
                    switch_to: None
                })
            },
            Some("profile_with_own_dir"),
        ))
        .await,
        Ok(()),
    ));

    assert!(output_dir.join("marker.txt").exists());
    remove_dir_all(&output_dir).unwrap();
}

