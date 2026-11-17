#!/usr/bin/env node
// WN-114: one version for the whole repository, and release tags that match it.
//
// Checks, from the repository root (or any folder inside it):
//   1. Every place that carries the app version holds the same X.Y.Z.
//   2. A v* tag on HEAD is exactly v<that version> (tags name versions, never cards).
//   3. Android versionCode grows past the one at the previous release tag.
//
// Usage: node scripts/check-version.cjs [--tag vX.Y.Z]
//   --tag  also require HEAD's version to be this tag (a release run passes
//          the tag it is building, e.g. GITHUB_REF_NAME).
// Exit 0 when all checks hold, 1 with a list of problems otherwise.
'use strict';

const { execFileSync } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');

const SEMVER = /^\d+\.\d+\.\d+$/;
const TAG = /^v(\d+\.\d+\.\d+)$/;

// Tags made before this rule whose commit carries another version. They stay
// as they are (WN-114: never rewrite a published or local tag).
const LEGACY_TAGS = new Map([['v0.32.0', '0.40.0']]);

// [file, label, extractor returning the version strings found]
const SOURCES = [
  ['Cargo.toml', 'workspace.package.version', (t) => {
    const block = t.split(/^\[workspace\.package\]\s*$/m)[1];
    return match1(block && block.split(/^\[/m)[0], /^version\s*=\s*"([^"]+)"/m);
  }],
  ['Cargo.lock', 'our crates', (t) => {
    const out = [];
    for (const name of ['wind-core', 'wind-ffi', 'windctl', 'windconnect-windows']) {
      const re = new RegExp(`\\[\\[package\\]\\]\\s*name = "${name}"\\s*version = "([^"]+)"`);
      const m = t.match(re);
      if (m) out.push(m[1]);
    }
    return out;
  }],
  ['apps/android/app/build.gradle.kts', 'versionName', (t) => match1(t, /versionName\s*=\s*"([^"]+)"/)],
  ['apps/android/wind-core/build.gradle.kts', 'version', (t) => match1(t, /^version\s*=\s*"([^"]+)"/m)],
  ['apps/windows/src-tauri/tauri.conf.json', 'version', (t) => [JSON.parse(t).version]],
  ['apps/windows/src-tauri/examples/icon-probe/tauri.conf.json', 'version', (t) => [JSON.parse(t).version]],
  ['relay/package.json', 'version', (t) => [JSON.parse(t).version]],
  ['relay/package-lock.json', 'version', (t) => {
    const j = JSON.parse(t);
    return [j.version, j.packages && j.packages[''] && j.packages[''].version];
  }],
];

function match1(text, re) {
  const m = text && text.match(re);
  return m ? [m[1]] : [];
}

function git(args, cwd) {
  return execFileSync('git', args, { cwd, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim();
}

function versionCode(text) {
  const m = text.match(/versionCode\s*=\s*(\d+)/);
  return m ? Number(m[1]) : null;
}

function main(argv) {
  const tagArg = argv.indexOf('--tag') >= 0 ? argv[argv.indexOf('--tag') + 1] : null;
  const root = git(['rev-parse', '--show-toplevel'], process.cwd());
  const problems = [];
  const found = [];

  for (const [file, label, extract] of SOURCES) {
    let text;
    try {
      text = fs.readFileSync(path.join(root, file), 'utf8');
    } catch {
      problems.push(`${file}: missing`);
      continue;
    }
    let values;
    try {
      values = extract(text);
    } catch (e) {
      problems.push(`${file}: unreadable (${e.message})`);
      continue;
    }
    if (!values.length || values.some((v) => !v)) problems.push(`${file}: no ${label} found`);
    for (const v of values.filter(Boolean)) found.push({ file, label, v });
  }

  const version = found.length ? found[0].v : null;
  if (version && !SEMVER.test(version)) problems.push(`${found[0].file}: "${version}" is not X.Y.Z`);
  for (const f of found) {
    if (f.v !== version) problems.push(`${f.file} (${f.label}): ${f.v}, expected ${version} (from ${found[0].file})`);
  }

  // A version change not committed yet (the check run before the commit):
  // HEAD's tags belong to the old version, and the previous release is the
  // last tag up to HEAD itself.
  let dirty = false;
  try {
    git(['diff', '--quiet', 'HEAD', '--', ...SOURCES.map(([f]) => f)], root);
  } catch {
    dirty = true;
  }

  // Tags on HEAD name the version HEAD carries.
  const headTags = dirty
    ? []
    : git(['tag', '--points-at', 'HEAD'], root).split(/\r?\n/).filter((t) => t.startsWith('v'));
  for (const t of headTags) {
    const m = t.match(TAG);
    if (!m) problems.push(`tag ${t} on HEAD is not vX.Y.Z`);
    else if (LEGACY_TAGS.has(t) ? LEGACY_TAGS.get(t) !== version : m[1] !== version)
      problems.push(`tag ${t} on HEAD, but HEAD's version is ${version}`);
  }
  if (tagArg) {
    const m = tagArg.match(TAG);
    if (!m) problems.push(`--tag ${tagArg} is not vX.Y.Z`);
    else if (m[1] !== version) problems.push(`--tag ${tagArg}, but HEAD's version is ${version}`);
  }

  // versionCode grows past the previous release tag's (an Android update needs it).
  const code = versionCode(fs.readFileSync(path.join(root, 'apps/android/app/build.gradle.kts'), 'utf8'));
  let prev = '';
  try {
    prev = git(['describe', '--tags', '--abbrev=0', '--match', 'v[0-9]*', dirty ? 'HEAD' : 'HEAD^'], root);
  } catch {
    // No earlier tag (or a shallow clone): nothing to compare with.
  }
  if (prev) {
    let prevVersion = null;
    let prevCode = null;
    try {
      prevCode = versionCode(git(['show', `${prev}:apps/android/app/build.gradle.kts`], root));
      prevVersion = (git(['show', `${prev}:apps/android/app/build.gradle.kts`], root).match(/versionName\s*=\s*"([^"]+)"/) || [])[1];
    } catch {
      // The file did not exist at that tag.
    }
    if (prevCode != null && prevVersion !== version && !(code > prevCode))
      problems.push(`versionCode ${code} must be greater than ${prevCode} at ${prev} (version changed ${prevVersion} -> ${version})`);
  }

  if (problems.length) {
    console.error(`Version check failed (version ${version}):`);
    for (const p of problems) console.error(`  - ${p}`);
    return 1;
  }
  const tagNote = dirty
    ? ', uncommitted version change (HEAD tags not checked)'
    : headTags.length ? `, HEAD tagged ${headTags.join(', ')}` : '';
  console.log(`Version ${version} in ${found.length} places, versionCode ${code}${tagNote}${prev ? `, previous tag ${prev}` : ''}.`);
  // WN-146: a clone without tags (CI's shallow checkout while tags stay local) checks only that the files agree.
  if (!prev) console.log('No previous v* tag here: versionCode growth not checked' + (headTags.length || dirty ? '.' : ', and no tag on HEAD to check.'));
  return 0;
}

process.exitCode = main(process.argv.slice(2));
