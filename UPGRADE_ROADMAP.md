# Ferium Fork Upgrade Roadmap

Tracking document for turning this fork (`kevintran-git/ferium`, forked from
`gorilla-devs/ferium`) into a **strict upgrade**: a superset of upstream
ferium's behavior, config format, and CLI, plus fixes for known bugs and the
useful ideas from the `otherpc/` Python replacement (`modmgr.py`).

This file is the durable source of truth for the plan — re-read it after any
context clear instead of re-deriving the plan from scratch.

## Ground rules ("strict upgrade")

- Never remove or break an existing CLI command, flag, or config field.
- `config.json` written by upstream ferium must always load correctly here.
- Any new config field is additive, uses `#[serde(default)]`, and goes
  through the versioned migration path (Phase 0) once that exists.
- CurseForge, Modrinth, and GitHub all stay fully supported — never drop a
  provider to make another feature easier (this was `modmgr.py`'s biggest
  mistake).

## Ideas evaluated from `otherpc/modmgr.py`

Full writeup of the comparison lives in Claude's memory
(`project_modmgr_evaluation`); summary:

**Port these:**
- Duplicate-JAR dedup keyed by the mod's own manifest ID (`fabric.mod.json`
  "id", Forge/NeoForge `mods.toml` "modId", `quilt.mod.json` "id") instead of
  filename matching — catches renamed files across versions.
- Standalone shaderpack/resourcepack tracking and upgrading, modeled after
  mod tracking (modmgr had this, upstream ferium doesn't).

**Do NOT port** (these were regressions in modmgr, not improvements):
- Dropping CurseForge/GitHub support.
- Disabling TLS verification (`ssl._create_unverified_context()`).
- Non-atomic direct-to-target downloads (no tmp file + rename).
- A separate `pinned`/`dont_check_for_updates` boolean bolted onto mods,
  disconnected from the real pin (which must stay embedded in the
  `ModIdentifier` itself, e.g. `PinnedModrinthProject`).
- Unsanitized `.mrpack` path extraction (zip-slip risk).
- Non-recursive, slug-guessing dependency resolution.
- Reading a stale/v4-shaped config (`game_version`/`mod_loader` flat fields)
  instead of the current `filters` array.

## Phases

### Phase 0 — Config schema versioning (foundation)
- [x] Add explicit schema version to `Config` (`version: u32`, defaults to
      `0` for pre-existing configs, migrated via `Config::migrate` in
      `libium/src/config/structs.rs`, called from `read_config` in `mod.rs`).
- [x] Migration runs automatically on `read_config`, old configs upgrade
      in-memory before use, and are rewritten in the new format on next save.
- [x] String-based version pins for CurseForge/GitHub (tag name, display
      name, or file name, not just numeric IDs) — commit `61cbd99`.

### Phase 1 — Low-risk wins
- [x] Manifest-ID-based duplicate JAR cleanup (bug #4), ported from modmgr's
      approach but covering Fabric/Forge/NeoForge/Quilt
      (`libium/src/manifest.rs`, wired into `src/download.rs::download`).
- [x] Fix for the `upgrade` resolution stage hanging forever on a single
      unresponsive Modrinth/CurseForge/GitHub request (bug #8). Root cause:
      `ferinth`/`furse`/`octocrab` build their own internal `reqwest::Client`
      with no timeout and no way to inject one, and the resolution loop in
      `get_platform_downloadables` (`src/subcommands/upgrade.rs`) waits for
      *every* mod's check to finish before downloading anything — so one dead
      connection blocks every already-resolved mod from downloading too.
      Rejected an automatic timeout (would either fail fast on legitimately
      slow-but-alive connections, or need an arbitrary guessed duration).
      Instead: pressing enter while checks are in progress aborts whatever
      hasn't resolved yet and proceeds to download everything that has,
      exactly like an already-failed mod is handled. Only wired up when
      stdin is a terminal, so scripted/non-interactive runs are unaffected.
- [x] Fixed pre-existing test suite compile error: `src/tests.rs` used the
      unstable `std::assert_matches` (nightly-only), converted all call sites
      to stable `assert!(matches!(...))`.
- [x] Fixed pre-existing rustls panic: both `ring` and `aws-lc-rs` crypto
      provider features end up active in the dependency graph (two `reqwest`
      versions resolve, one pulling in each backend), so rustls can't
      auto-select a default provider and panics on the first TLS connection.
      This isn't test-only — it would hit the real binary too, just
      depending on which network call happens to run first. Fixed by adding
      `rustls` as an explicit dependency and calling
      `rustls::crypto::ring::default_provider().install_default()` once at
      the top of `actual_main`, so both the real CLI and every test go
      through it.
- Confirmed via test suite run: after the above two fixes, the remaining
  test failures (`add_github`, `add_all`, `add_all_pinned`, `already_added`,
  `list_markdown`, `list_verbose`, `remove_slug`, `upgrade`) are all GitHub
  API rate limiting (403, unauthenticated 60 req/hour) in this dev
  environment with no `GITHUB_TOKEN` set — not a code bug. Feeds directly
  into the Phase 2 GitHub token item below.

### Phase 2 — Network resilience
- [x] Fixed a crash in `ferium add <github-repo>`: the GraphQL error handler
      indexed `err.path[0]` unconditionally, but GitHub omits `path` on
      query-level errors (rate limiting, abuse detection) — panicked instead
      of surfacing the actual error message (bug #6).
- [x] `upgrade`'s per-mod resolution loop now recognizes GitHub rate limits
      and invalid/unauthorized CurseForge API keys the same way it already
      did for Modrinth's rate limit: bail immediately instead of repeating
      the identical failure once per remaining mod on that platform
      (bugs #5/#6).
- [x] Added hints to the top-level error output pointing at
      `--github-token`/`--curseforge-api-key` when the error looks like a
      rate limit or a rejected CurseForge key.
- [x] Harden Modrinth response parsing: a version with an empty `files`
      list (schema drift, or a version whose files were pulled after a
      malware scan) crashed via an unchecked `self.files[0]` in
      `VersionExt::get_version_file`. Same class of crash already partially
      fixed for modpacks with zero versions in #508 — this fixes it one
      layer down, for a single version with zero files. `from_mr_version`
      is now fallible; when checking all versions of a project the bad one
      is skipped instead of aborting the whole check (bug #7).

### Phase 3 — Shaderpacks & resourcepacks (new feature, from modmgr)
- [x] Extended `Profile` with `shaderpacks: Vec<Mod>` and
      `resourcepacks: Vec<Mod>`, both `#[serde(default)]` and skipped when
      empty, so old configs load unchanged. Added `ProjectKind` (`Mod` /
      `ResourcePack` / `ShaderPack`) plus `Profile::mods(kind)` /
      `mods_mut(kind)` / `dir(kind)` so the add/list/remove/upgrade paths can
      be generic over which list they're operating on instead of tripling
      the logic.
- [x] CLI subcommands mirroring mod add/list/remove/upgrade:
      `ferium shaderpack <add|list|remove|upgrade>` and
      `ferium resourcepack <add|list|remove|upgrade>`, sharing one
      `PackSubCommands` definition.
- [x] Corrected the original plan text above: shaderpacks/resourcepacks
      download to `shaderpacks`/`resourcepacks` directories *alongside*
      `output_dir` (i.e. siblings of the mods folder), not nested inside it.
      Nesting them inside `output_dir` (the mods folder) would mean Iris/
      OptiFine and Minecraft's resource pack picker never actually find the
      files, since they always look next to `mods`, not inside it.
- [x] `libium::add()` now takes a `ProjectKind` and checks the right thing
      per kind instead of hardcoding "is this a mod": CurseForge project
      category via the site URL segment (`mc-mods` / `texture-packs` /
      `shaders`), Modrinth via `project_type`. Wrong-kind adds now fail with
      "The project is not a shader pack" etc. instead of silently accepting
      any project ID.
- [x] Mod-loader filters (Fabric/Forge/Quilt/NeoForge) don't apply to
      shaderpacks/resourcepacks, so they're stripped from the profile's
      filters before checking/downloading non-mod kinds
      (`ProjectKind::applicable_filters`).
- [x] Found and fixed a real bug this surfaced: `check::select_latest`
      unconditionally required a `ModLoaderPrefer` filter to pick a final
      answer (it only computed the winning index from the "run last"
      mod-loader-preference filters). Since mod profiles always carry one,
      this was never exercised — but stripping mod-loader filters for
      shaderpacks/resourcepacks meant every add/upgrade failed with
      "Failed to find a compatible combination" even for fully compatible
      projects. Fixed by falling back to the intersected, non-mod-loader
      filter results directly when there's no mod-loader filter to run last.
- [x] Added `tests::add_shaderpack`, `add_resourcepack`, and
      `add_shaderpack_wrong_kind`; verified end-to-end by hand against real
      Modrinth projects (add, wrong-kind rejection, list, upgrade — which
      downloaded into sibling `shaderpacks`/`resourcepacks` directories as
      expected — and remove).

### Phase 4 — Big subsystems
- [x] `upgrade` already resolved dependencies recursively at download time
      via structured, project-ID-based metadata (`DownloadData.dependencies`,
      fed back into the same resolution queue) — that part of bug #2 turned
      out to already be fixed upstream, unlike modmgr's fragile manifest-ID
      slug guessing. The actual gap: `ferium add` never looked at
      dependencies at all, so a required dependency (e.g. Fabric API) was
      silently missing from the profile until the next `upgrade` pulled it
      in ephemerally, without ever being tracked. `libium::add()` is now a
      thin wrapper around the original per-batch logic (renamed
      `add_batch`): after a CurseForge/Modrinth project is added, it fetches
      that project's resolved download file, reads its required
      dependencies, and feeds any not already tracked back into another
      `add_batch` round (as `ProjectKind::Mod`, since that's what a
      "required dependency" is in practice), looping until a round adds
      nothing new. Optional dependencies are intentionally left alone (no
      interactive prompting in the library layer). Verified live: adding
      `sodium-extra` transitively pulled in both `Sodium` and `Fabric API`
      with no duplicates, and `iris` correctly skipped `Sodium` when it was
      already tracked. Covered by `tests::add_resolves_dependencies`.
- [x] Added `Filter::GameVersionRange { from, to }` — bug #3. Either bound
      can be omitted (open-ended). Matching resolves both bounds against
      Modrinth's ordered game-version tag list (the same list
      `GameVersionMinor` already uses) and accepts any release version
      between them, inclusive, regardless of which bound is numerically
      larger. Exposed as `--game-version-range FROM..TO` (also `FROM..` /
      `..TO`) alongside the existing `--game-version-strict`/`-minor` flags
      in `FilterArguments`; parsing the range string can fail (bad syntax,
      no bounds, or an unrecognised version), so
      `impl From<FilterArguments> for Vec<Filter>` became `TryFrom`. An
      unknown bound surfaces as a normal incompatibility error ("9.99 is not
      a known game version") rather than silently matching nothing. Covered
      by `tests::add_game_version_range` and
      `add_game_version_range_unknown_bound`.

### Phase 5 — Safety fixes modmgr got wrong
- [x] Fault-tolerant modpack import (bug #14): `from_modpack_file` used
      `file.downloads.first().expect(...)`, so a single Modrinth modpack file
      with no download URLs would panic and abort the entire `modpack
      upgrade`, instead of just that one file being unavailable. Now returns
      `Option<DownloadData>`; the caller in
      `src/subcommands/modpack/upgrade.rs` skips the file with a warning
      instead. The CurseForge side (`try_from_cf_file` and its caller) was
      already fault-tolerant per-file, so this only affected Modrinth
      modpacks. Covered by `tests::from_modpack_file_no_downloads`/
      `from_modpack_file_with_download` in `libium`.
- [x] `ferium scan` ignores dotfiles (bug #10): a hidden file directly in the
      scanned directory (e.g. a dot-prefixed manual backup that still ends in
      `.jar`) was picked up and could be added to the profile as if it were a
      real mod, since `scan()` only checked for a `.jar` extension. Now
      entries whose filename starts with `.` are skipped before the
      extension check. Directories (including `.old`, which `download.rs`'s
      `clean()` creates for removed files) were already excluded since `scan`
      only reads files at the top level, not recursively. Found and fixed a
      second, related bug this surfaced: `scan()` unconditionally called
      CurseForge's fingerprint-matching endpoint even with an empty hash
      list, which CurseForge rejects with 400 Bad Request — a directory
      containing only ignorable files (or no jars at all) would error out
      instead of returning no matches. Fixed with an early return when there
      are no hashes to look up. Covered by `tests::scan_ignores_hidden_files`.
- [x] Confirmed cleanup of a profile's output dir on `profile delete` (bug
      #11): previously the profile entry was removed from `config.profiles`
      but its `output_dir` and everything downloaded into it were left on
      disk, orphaned, with no way to know they used to belong to a profile.
      `profile delete` now asks "Also delete the output directory `<path>`?"
      (default no) when the directory exists, and removes it recursively
      only on explicit confirmation. Covered by
      `tests::delete_profile_keeps_output_dir_without_confirmation`, and
      verified live that a non-empty directory survives an unconfirmed
      delete.
- [x] Audited download atomicity (bug #9): `DownloadData::download` already
      wrote to a `.part` temp file and `rename`d it into place, which is
      atomic for the final swap — but the temp file was opened with
      `.append(true)`, not truncated. Since every download re-fetches the
      whole file from scratch (no HTTP range/resume support), a `.part` file
      left over from a previous crashed or interrupted download would get
      new bytes appended after the stale ones, silently corrupting the final
      renamed file instead of being an atomicity failure at the rename step.
      Fixed by opening with `.truncate(true)` instead. Verified live by
      seeding a `.part` file with 500KB of garbage before an upgrade and
      confirming the final file came out byte-exact. While auditing, found
      the same pattern in `write_config` (`libium/src/config/mod.rs`): it
      wrote JSON directly into the live `config.json` with `File::create`,
      so a crash or serialization error partway through could truncate the
      user's entire config (all profiles and tracked mods), not just one
      download. Fixed the same way: write to `config.json.tmp`, then
      `rename` over the real path.

### Phase 6 — Modpacks as profile-scoped groups
- [x] Removed the old standalone `Modpack`/`ModpackIdentifier` config concept
      entirely; a modpack is now a `ModpackGroup` (`name`, `source`,
      `last_seen_version`, `install_overrides`, `excluded`) living on
      `Profile.modpacks`, sourced from a 4-variant `ModpackSource`
      (`ModrinthHosted`/`CurseForgeHosted` project, or a raw `MrpackFile`/
      `CurseForgeZipFile` URL for third-party archives).
- [x] `Mod.source_modpack: Option<String>` tags which group (if any) added a
      given mod, so `hopper add`-ed mods and modpack-provided mods coexist in
      the same list without either clobbering the other.
- [x] `Config` schema bumped to version 2; `Config::migrate` converts every
      legacy top-level `modpacks[]`/`active_modpack` entry into its own new
      profile (`libium/src/config/structs.rs`). Old v0/v1 configs still
      migrate through the existing `Profile::backwards_compat` path first.
- [x] `hopper modpack add|list|remove` replaces the old
      `add/configure/delete/switch/upgrade` set. `remove` defaults to
      detaching (`source_modpack` cleared, mods kept); `--delete-mods`
      opts into actually deleting them — named this way instead of the
      originally-planned `--keep-mods`, since a "keep" flag defaulting to
      "keep" is a confusing double-negative.
- [x] `hopper upgrade` runs a group-refresh pass (resolve → diff → add /
      update / remove / warn-on-conflict → install overrides) for every
      tracked group before its normal download pass — one pipeline, no
      separate sync command. Pure reconciliation logic (`diff_entries`) and
      `.mrpack` file classification are unit-tested without any network
      access (`libium/src/modpack/group.rs`).
- [x] Fixed a real bug found while wiring this up: config migrations were
      resolved entirely in memory but never reached disk, because the
      pre-existing "skip write if unchanged" check compared the in-memory
      config against itself post-migration instead of against the
      pre-migration version. `read_config` now snapshots `version` before
      calling `migrate` and writes whenever that changed.

### Phase 7 — `hopper join`
- [x] One command to bring a vanilla `.minecraft` to a joinable state:
      installs a Fabric loader via Fabric's meta API and upserts
      `launcher_profiles.json` (`libium/src/loader_install.rs`), then
      creates or reuses a profile for that instance, tracks the modpack as a
      group, and runs the upgrade pipeline.
- [x] `--no-install-loader` skips the loader/launcher-profile step for
      instances that already have one (e.g. Prism, MultiMC).

### Phase 8 — Distribution to the other Apple Silicon Mac
- [ ] Decide install method (build `--release` locally + transfer binary,
      vs. `cargo install --git` on the other machine, vs. a GitHub Release
      build via the existing `.github/workflows/release.yml`).
- [ ] Get the other Mac's `~/.config/ferium/config.json` and profiles
      working against this fork with zero manual migration.

## Status log

- 2026-07-21: Roadmap created. Phase 0 not yet started.
- 2026-07-21: Phase 0 done — `Config.version`, `Config::migrate`, wired into
  `read_config`.
- 2026-07-21: Phase 1 done — manifest-ID dedup, resolution-stage skip signal,
  plus two pre-existing bugs fixed along the way (`assert_matches` nightly
  feature, rustls dual-crypto-provider panic).
- 2026-07-21: Phase 2 done — GitHub GraphQL error-path crash fix, early bail
  on GitHub rate limit / bad CurseForge key during `upgrade`, top-level error
  hints, and a Modrinth version-with-no-files crash fix.
- 2026-07-21: Phase 3 done — shaderpack/resourcepack tracking with new
  `ferium shaderpack`/`ferium resourcepack` subcommands, `ProjectKind`-based
  generalization of add/list/remove/upgrade, and a `check::select_latest`
  bug fix (see above) that the new mod-loader-less code path surfaced.
- 2026-07-21: Phase 4 done — `ferium add` now recursively resolves and
  tracks required dependencies (it never did before; `upgrade` already
  handled this correctly), and a new `Filter::GameVersionRange` extends the
  filter system with `--game-version-range FROM..TO` matching.
- 2026-07-21: Phase 5 done — fault-tolerant Modrinth modpack file import
  (skip-and-warn instead of panicking), `scan` ignores dotfiles and no longer
  400s on an all-hidden/empty directory, confirmed output-dir cleanup on
  `profile delete`, and an atomicity audit that found and fixed a real bug:
  `.part` temp files (for both mod downloads and `config.json` itself) were
  opened in append mode instead of being truncated, so a leftover file from a
  crashed run could silently corrupt the next write.
- 2026-07-21: Project renamed to `hopper`.
- 2026-07-21: Phase 6 done — modpacks are now `ModpackGroup`s tracked per
  profile instead of a standalone top-level concept; config version bumped
  to 2 with automatic migration of legacy modpacks into profiles;
  `hopper modpack add/list/remove` replace the old command set; `hopper
  upgrade` runs group-refresh before its download pass; fixed a bug where
  migrations never persisted to disk.
- 2026-07-21: Phase 7 done — `hopper join` installs a Fabric loader and
  launcher profile into a vanilla `.minecraft`, then tracks the modpack and
  upgrades, in one command (`--no-install-loader` to skip the loader step).
