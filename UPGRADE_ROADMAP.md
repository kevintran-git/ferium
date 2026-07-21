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
- [ ] Add explicit schema version to `Config` (model on the existing
      `Profile::backwards_compat` pattern in `libium/src/config/structs.rs`
      and `mod.rs`).
- [ ] Migration runs automatically on `read_config`, old configs upgrade
      in-memory before use, and are rewritten in the new format on next save.
- [x] String-based version pins for CurseForge/GitHub (tag name, display
      name, or file name, not just numeric IDs) — commit `61cbd99`.

### Phase 1 — Low-risk wins
- [ ] Manifest-ID-based duplicate JAR cleanup (bug #4), ported from modmgr's
      approach but covering Fabric/Forge/NeoForge/Quilt.
- [ ] HTTP connect/read timeouts on the shared client (bug #8).

### Phase 2 — Network resilience
- [ ] Verify/fix GitHub token handling for rate limits (bug #6).
- [ ] Verify/fix CurseForge API key handling (bug #5).
- [ ] Harden Modrinth response parsing so schema drift doesn't hard-abort
      (bug #7).

### Phase 3 — Shaderpacks & resourcepacks (new feature, from modmgr)
- [ ] Extend `Profile` with tracked shaderpacks/resourcepacks (reusing
      `Mod`/`ModIdentifier`), default-empty for backward compatibility.
- [ ] CLI subcommands mirroring mod add/list/remove/upgrade, targeting
      `output_dir/shaderpacks` and `output_dir/resourcepacks`.

### Phase 4 — Big subsystems
- [ ] Recursive dependency resolution using structured dependency metadata
      (Modrinth's project-id-based deps, not manifest-ID slug guessing) —
      bug #2.
- [ ] Range-aware game version matching, extending the `Filter` system —
      bug #3.

### Phase 5 — Safety fixes modmgr got wrong
- [ ] Fault-tolerant modpack import: skip-and-warn on a single bad file
      instead of aborting the whole import (bug #14).
- [ ] `ferium scan` ignores dotfiles/backup dirs like `.old/` (bug #10).
- [ ] Confirmed cleanup of a profile's output dir on `profile delete`,
      instead of silently orphaning files (bug #11).
- [ ] Audit download atomicity (tmp file + rename) everywhere ferium writes
      downloaded files, to confirm bug #9 isn't actually present upstream.

### Phase 6 — Distribution to the other Apple Silicon Mac
- [ ] Decide install method (build `--release` locally + transfer binary,
      vs. `cargo install --git` on the other machine, vs. a GitHub Release
      build via the existing `.github/workflows/release.yml`).
- [ ] Get the other Mac's `~/.config/ferium/config.json` and profiles
      working against this fork with zero manual migration.

## Status log

- 2026-07-21: Roadmap created. Phase 0 not yet started.
