#!/usr/bin/env node
// CCS/1 PURPOSE:WN-174 English is the source language of both apps, Polish a translation.
// Checks (node scripts/check-language.cjs, no dependencies, from any folder):
// 1. Each Polish table has exactly the keys (and placeholders) of its English one: Android app and wind-core
//    (values/ vs values-pl/), Windows native (en.json vs pl.json) and frontend (en.js vs pl.js).
// 2. No Polish letters in tracked sources outside the Polish tables. A line may keep them on purpose with a marker:
//    `i18n: data` (Unicode test data, names, typed text) or `i18n: pl` (a check of the Polish texts, or system UI
//    shown in the device's language). Whole files that exist to check Polish are listed in ALLOWED with the reason.
'use strict';
const { execFileSync } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');

const repo = path.resolve(__dirname, '..');
const POLISH = /[ąćęłńóśźżĄĆĘŁŃÓŚŹŻ]/u;
const MARKER = /i18n: (data|pl)\b/;
const TABLES = [
  'apps/android/app/src/main/res/values-pl/strings.xml',
  'apps/android/wind-core/src/main/res/values-pl/strings.xml',
  'apps/windows/src-tauri/locales/pl.json',
  'apps/windows/frontend/locales/pl.js',
];
const ALLOWED = {
  'apps/android/app/src/androidTest/java/io/github/prejas3/windconnect/LocalizationTest.kt': 'checks the Polish resources',
  'apps/android/app/src/androidTest/java/io/github/prejas3/windconnect/PcActionNamesTest.kt': "an older PC's Polish labels, named in the phone's language",
  'apps/windows/scripts/check-i18n.cjs': 'renders the pages in pl-PL and looks for Polish in the English ones',
  'scripts/check-language.cjs': 'this check',
};
// Text the repository keeps in Polish on purpose: the Polish store listing and changelog for Polish readers.
const ALLOWED_DIRS = ['apps/android/fastlane/metadata/android/pl-PL/'];
const TEXT = /\.(kt|kts|java|rs|js|cjs|mjs|ts|html|css|json|xml|toml|yml|yaml|ps1|sh|wxs|txt|properties|gradle)$/i;

const errors = [];

// 1. Parity.
const placeholders = text => [...String(text).matchAll(/\{(\w+)\}|%(\d+)\$[a-z]/g)].map(m => m[1] ?? `%${m[2]}`).sort().join();
function same(name, pl, en, { plurals = false } = {}) {
  const pk = Object.keys(pl).sort(), ek = Object.keys(en).sort();
  for (const k of ek) if (!(k in pl)) errors.push(`${name}: "${k}" has no Polish text`);
  for (const k of pk) if (!(k in en)) errors.push(`${name}: "${k}" is only in Polish (English is the source)`);
  if (plurals) return;
  for (const k of pk) if (k in en && placeholders(pl[k]) !== placeholders(en[k])) errors.push(`${name}: "${k}" placeholders differ`);
}
function androidTable(file) {
  const xml = fs.readFileSync(path.join(repo, file), 'utf8');
  const strings = {}, plurals = {};
  for (const m of xml.matchAll(/<string\s+name="([^"]+)"([^>]*)>([\s\S]*?)<\/string>/g)) {
    if (/translatable="false"/.test(m[2])) continue;
    strings[m[1]] = m[3];
  }
  for (const m of xml.matchAll(/<plurals\s+name="([^"]+)"[^>]*>([\s\S]*?)<\/plurals>/g)) plurals[m[1]] = m[2];
  return { strings, plurals };
}
for (const mod of ['app', 'wind-core']) {
  const base = `apps/android/${mod}/src/main/res`;
  const en = androidTable(`${base}/values/strings.xml`), pl = androidTable(`${base}/values-pl/strings.xml`);
  same(`android ${mod} strings`, pl.strings, en.strings);
  same(`android ${mod} plurals`, pl.plurals, en.plurals, { plurals: true });
}
const json = file => JSON.parse(fs.readFileSync(path.join(repo, file), 'utf8'));
same('windows native', json('apps/windows/src-tauri/locales/pl.json'), json('apps/windows/src-tauri/locales/en.json'));
// The frontend tables are ES modules: `export default { "key": "text", ... };`
const esTable = file => {
  const source = fs.readFileSync(path.join(repo, file), 'utf8');
  const body = source.slice(source.indexOf('{', source.indexOf('export default')), source.lastIndexOf('}') + 1);
  return JSON.parse(body.replace(/,\s*}$/, '}'));
};
same('windows frontend', esTable('apps/windows/frontend/locales/pl.js'), esTable('apps/windows/frontend/locales/en.js'));

// 2. No Polish outside the Polish tables.
const files = execFileSync('git', ['ls-files', '-z'], { cwd: repo, encoding: 'utf8' }).split('\0').filter(Boolean);
let scanned = 0;
for (const file of files) {
  if (!TEXT.test(file) || TABLES.includes(file) || ALLOWED[file] || ALLOWED_DIRS.some(d => file.startsWith(d))) continue;
  const full = path.join(repo, file);
  if (!fs.existsSync(full)) continue;
  scanned++;
  fs.readFileSync(full, 'utf8').split('\n').forEach((line, i) => {
    if (POLISH.test(line) && !MARKER.test(line)) errors.push(`${file}:${i + 1}: Polish outside a Polish table: ${line.trim().slice(0, 120)}`);
  });
}

if (errors.length) {
  for (const e of errors) console.error(e);
  console.error(`\n${errors.length} problem(s). Move UI text to the tables (English in values/, en.json, en.js), or mark deliberate Polish with "i18n: data" / "i18n: pl".`);
  process.exit(1);
}
console.log(`Language check ok: 4 Polish tables match their English ones; ${scanned} sources have no Polish outside them.`);
