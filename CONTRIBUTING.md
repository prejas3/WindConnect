# Contributing

Keep a change to the scope of its task. Describe its purpose and how to check it.
New code and project documentation are released under the MIT license.

## Language (WN-164)

Everything developers read is in English: documentation (`*.md`), code comments
and doc comments, test names and test/CI messages, commit messages, pull requests
and release notes. The app's user interface is written in English and translated
to Polish (WN-63, WN-174): its texts live only in the localization resources, English
first (`apps/android/**/values/strings.xml`, `apps/windows/frontend/locales/en.js`,
`apps/windows/src-tauri/locales/en.json`) and Polish beside them (`values-pl/`, `pl.js`,
`pl.json`). Tests run in English. `node scripts/check-language.cjs` checks that both
languages have the same keys and that no Polish is left outside the Polish tables; a
line keeping Polish letters on purpose says so with `i18n: data` (Unicode test data)
or `i18n: pl` (a check of the Polish texts).

## Rust

The Rust toolchain is pinned in `rust-toolchain.toml` (WN-163), so local builds and
CI use the same compiler and lints; rustup installs it on first use. Moving to a
newer Rust is a change of its own: update the file and fix what its clippy finds.
Before handing a change in for review, run from the repository root:

```sh
cargo fmt --all -- --check
cargo build --locked
```

When you add behaviour, add the matching tests and run `cargo test --locked`.
Update `Cargo.lock` when dependencies change and include it in the commit.

## Apps

Windows client: [apps/windows/README.md](apps/windows/README.md).
Android client: [apps/android/README.md](apps/android/README.md).

## Versions and tags (WN-114)

One `X.Y.Z` version for the whole repository. A tag always names a version,
never a card: the card number is in the commit message, not in the tag.

- A card with a new feature or a change of behaviour ends with one commit
  on `master` titled `WN-N (vX.Y.Z): description`. That commit raises the
  version (usually `Y`; a fix without a new feature may raise `Z`) and gets
  the tag `vX.Y.Z`, pushed together with `master`.
- Commits that don't change the version (tests, cleanups, fixes within a card)
  get no tag and have no version in their title.
- A card doesn't name a version or tag in advance: whoever commits picks the
  next version after the current one on `master`. A card asking for a specific
  tag is out of date if the version has already passed it.
- Change the version in all places at once: `Cargo.toml`
  (`workspace.package`) and `Cargo.lock`, `apps/android/app/build.gradle.kts`
  (`versionName` and `versionCode` + 1), `apps/android/wind-core/build.gradle.kts`,
  both `tauri.conf.json` under `apps/windows/src-tauri`, `relay/package.json`
  and `relay/package-lock.json`.
- Before the commit and before the tag, run `node scripts/check-version.cjs`
  (for a release: `--tag vX.Y.Z`). It checks that all places agree, the tag on
  `HEAD` and that `versionCode` went up; Core CI runs it too. CI checks out
  shallowly without tags, so there it checks only that the files agree and says
  so ("No previous v* tag here…"); the tag and `versionCode` are checked by the
  local run before the commit.
- Existing tags are never rewritten and versions never go down. An exception
  from before this rule: `v0.32.0` points at a commit with version 0.40.0 (WN-32).

## Agents' work: disk space (WN-95)

Every slot and every review copy builds from scratch, so working folders grow
quickly. Rules:

- After a finished card (Done) and after every review, run the cleanup:
  `pwsh scripts/clean-build.ps1` (preview: `-WhatIf`, the whole `target`:
  `-Full`). The script removes build outputs only: Cargo's incremental cache,
  MSI packages of older versions, Gradle builds and the fuzzing `target`. It
  never touches code or files tracked by Git, and it works on a clean slot too.
- Don't keep copies of MSI or APK files from previous versions (also outside `target`).
- Keep full test and build logs in `%TEMP%` (the agent's working folder),
  only until the card ends, never in the repository.
- Android: debug builds and tests compile one ABI, `x86_64` by default
  (emulator); for a phone use `-PwindAbi=arm64-v8a`. Release and CI build both.
- Cargo's `dev`/`test` profile has line tables only for our crates and no
  debug info for dependencies; for a debugger with full info set
  `CARGO_PROFILE_DEV_DEBUG=true` for a while.
