#!/usr/bin/env node
// WN-124 (decision D2): the permissions of a built APK match its store.
//   play:   none of the SMS, Call Log and contacts permissions Google Play restricts;
//   github: SMS, Call Log and contacts are there (the full build).
// Usage: node scripts/check-android-store-permissions.cjs <apk> <github|play>
// Reads the APK with aapt from ANDROID_HOME/build-tools (the newest installed).
'use strict';
const { execFileSync } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');

const PLAY_FORBIDDEN = [
  'android.permission.READ_SMS', 'android.permission.SEND_SMS', 'android.permission.RECEIVE_SMS',
  'android.permission.RECEIVE_MMS', 'android.permission.RECEIVE_WAP_PUSH',
  'android.permission.READ_CALL_LOG', 'android.permission.WRITE_CALL_LOG',
  'android.permission.READ_CONTACTS', 'android.permission.ANSWER_PHONE_CALLS',
];
const GITHUB_REQUIRED = [
  'android.permission.READ_SMS', 'android.permission.SEND_SMS',
  'android.permission.READ_CALL_LOG', 'android.permission.READ_CONTACTS',
];
// What both builds keep (the rest of the app).
const BOTH_REQUIRED = ['android.permission.INTERNET', 'android.permission.READ_PHONE_STATE'];

function aapt() {
  const home = process.env.ANDROID_HOME || process.env.ANDROID_SDK_ROOT;
  if (!home) throw new Error('set ANDROID_HOME');
  const tools = path.join(home, 'build-tools');
  const versions = fs.readdirSync(tools).sort((a, b) => a.localeCompare(b, undefined, { numeric: true }));
  for (const v of versions.reverse()) {
    for (const name of ['aapt.exe', 'aapt']) {
      const file = path.join(tools, v, name);
      if (fs.existsSync(file)) return file;
    }
  }
  throw new Error('no aapt under ' + tools);
}

function permissions(apk) {
  const out = execFileSync(aapt(), ['dump', 'permissions', apk], { encoding: 'utf8' });
  return new Set([...out.matchAll(/uses-permission(?:-sdk-23)?: name='([^']+)'/g)].map(m => m[1]));
}

function check(apk, store) {
  const have = permissions(apk);
  const problems = [];
  for (const p of BOTH_REQUIRED) if (!have.has(p)) problems.push(`missing ${p}`);
  if (store === 'play') for (const p of PLAY_FORBIDDEN) { if (have.has(p)) problems.push(`play declares ${p}`); }
  else if (store === 'github') for (const p of GITHUB_REQUIRED) { if (!have.has(p)) problems.push(`github lacks ${p}`); }
  else problems.push(`unknown store ${store}`);
  return { have, problems };
}

if (require.main === module) {
  const [apk, store] = process.argv.slice(2);
  if (!apk || !store) { console.error('usage: check-android-store-permissions.cjs <apk> <github|play>'); process.exit(2); }
  if (!fs.existsSync(apk)) { console.error(`no APK at ${apk}`); process.exit(2); }
  const { have, problems } = check(apk, store);
  if (problems.length) {
    console.error(`${path.basename(apk)} (${store}): ${problems.join('; ')}`);
    process.exit(1);
  }
  console.log(`${path.basename(apk)} (${store}): ok, ${have.size} permissions`);
}
module.exports = { check, PLAY_FORBIDDEN };
