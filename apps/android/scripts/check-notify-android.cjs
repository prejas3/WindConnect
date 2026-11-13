// CCS/1 PURPOSE:WN-42 phone notifications to a PC that takes notify/1 (PROTOCOL.md §9.7), on an emulator. notify_peer stands in for
// the PC until WN-43; NotifyProofTest drives the real WindCore and measures postTime -> written to the stream (p95 < 50 ms). Here:
// what reached the PC: the 20 plain ones and the chat once each, the app icon once and before its first notification, nothing of
// the media one or the unchanged re-post, Removed for the snoozed one only. Restores the listener access.
// Run from the repository root after `gradlew :app:assembleDebug :app:assembleDebugAndroidTest` and
// `cargo build --locked -p wind-ffi --example notify_peer`. ANDROID_SERIAL picks the emulator.
const { spawn, spawnSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const assert = require('node:assert/strict');

const adb = path.join(process.env.ANDROID_HOME, 'platform-tools/adb.exe');
const serial = process.env.ANDROID_SERIAL || 'emulator-5590';
const port = Number(process.env.NOTIFY_PEER_PORT || 47830);
const runner = 'io.github.prejas3.windconnect.test/androidx.test.runner.AndroidJUnitRunner';
const LISTENER = 'io.github.prejas3.windconnect/io.github.prejas3.windconnect.core.NotifyListenerService';
const root = fs.mkdtempSync(path.join(os.tmpdir(), 'windconnect-wn42-notify-'));
const device = (...args) => { const r = spawnSync(adb, ['-s', serial, ...args], { encoding: 'utf8', windowsHide: true, timeout: 180000 }); assert.equal(r.status, 0, r.stderr + r.stdout); return r.stdout; };
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));

// One pass: a fresh notify_peer, NotifyProofTest with `measure`, then what the peer got.
async function pass(androidKey, measure, label) {
  let peer, peerOut = '';
  try {
    // Android keeps at most 50 notifications per app: clear the shell's from earlier runs (a force-stop cancels them).
    device('shell', 'am', 'force-stop', 'com.android.shell');
    peer = spawn(path.resolve('target/debug/examples/notify_peer.exe'), [androidKey, `127.0.0.1:${port}`, '180'], { windowsHide: true });
    peer.stdout.on('data', d => { peerOut += d; }); peer.stderr.on('data', d => { peerOut += d; });
    for (let i = 0; i < 100 && !/PC_PUBLIC_KEY=/.test(peerOut); i++) await delay(50);
    const pcKey = peerOut.match(/PC_PUBLIC_KEY=([0-9a-f]{64})/)?.[1];
    assert(pcKey, peerOut);
    const run = spawnSync(adb, ['-s', serial, 'shell', 'am', 'instrument', '-w', '-r', '-e', 'class',
      'io.github.prejas3.windconnect.NotifyProofTest', '-e', 'pcAddress', `10.0.2.2:${port}`, '-e', 'pcKey', pcKey, '-e', 'measure', measure, '-e', 'tag', `${label}${Date.now() % 1000000}x`, runner],
      { encoding: 'utf8', windowsHide: true, timeout: 240000 });
    const output = run.stdout + run.stderr;
    fs.writeFileSync(path.join(root, `instrumentation-${label}.log`), output);
    for (let i = 0; i < 100 && !/PEER_DONE/.test(peerOut); i++) await delay(50);
    fs.writeFileSync(path.join(root, `peer-${label}.log`), peerOut);
    assert(output.includes('OK (1 test)'), output);
    const proof = output.match(/notifyProof=([^\n]*)/)?.[1];

    const lines = peerOut.split(/\r?\n/);
    const posted = lines.filter(l => l.startsWith('NOTIFY_POSTED'));
    const icons = lines.filter(l => l.startsWith('NOTIFY_ICON'));
    const removed = lines.filter(l => l.startsWith('NOTIFY_REMOVED'));
    assert.equal(lines.filter(l => l.startsWith('NOTIFY_MALFORMED')).length, 0, peerOut);
    assert.equal(posted.length, 21, 'the 20 plain ones and the chat, once each:\n' + posted.join('\n'));
    for (let i = 0; i < 20; i++) { const c = String.fromCharCode(97 + i); assert.equal(posted.filter(l => l.includes(`title=Some("Ola_${c}") text=Some("Wiadomosc_${c}")`)).length, 1, `Ola_${c}`); }
    assert(posted.some(l => l.includes('title=Some("Rodzina") text=Some("Piotr: klucze")')), 'the chat: its last message and who wrote it');
    assert(!peerOut.includes('Piosenka'), 'a player is skipped');
    assert(posted.every(l => l.includes('package="com.android.shell"')));
    assert.equal(icons.length, 1, 'one icon for one app');
    assert(lines.indexOf(icons[0]) < lines.indexOf(posted[0]), 'the icon before the first notification naming it');
    const key = icons[0].match(/key=([0-9a-f]{16})/)[1];
    assert(posted.every(l => l.endsWith(`icon=${key}`)));
    assert.equal(removed.length, 1, removed.join('\n'));
    const removedId = removed[0].match(/id=([0-9a-f]{32})/)[1];
    const proof2 = posted.find(l => l.includes('title=Some("Ola_b")'));
    assert(proof2.includes(`id=${removedId}`), 'Removed for the snoozed one');
    return { measure, proof, posted: posted.length, icons: icons.length, removed: removed.length };
  } finally {
    if (peer && peer.exitCode === null) peer.kill();
  }
}

(async () => {
  let granted = false, assistant = null;
  try {
    device('install', '-r', 'apps/android/app/build/outputs/apk/debug/app-debug.apk');
    device('install', '-r', '-t', 'apps/android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk');
    device('shell', 'cmd', 'notification', 'allow_listener', LISTENER); granted = true;
    const id = device('shell', 'am', 'instrument', '-w', '-r', '-e', 'class',
      'io.github.prejas3.windconnect.CoreConnectionTest#identityLoadsThroughUniFFIAndSurvivesActivityRecreation', runner);
    const androidKey = id.match(/androidPublicKey=([0-9a-f]{64})/)?.[1];
    assert(androidKey, id);
    const report = { passes: [], artifacts: root };
    // As the phone is: the assistant's hold comes before every listener; our part is callback -> written.
    report.passes.push(await pass(androidKey, 'listener', 'assistant'));
    // Control: without the assistant, Android's postTime itself -> written must be under 50 ms.
    assistant = device('shell', 'cmd', 'notification', 'get_approved_assistant').trim(); if (assistant === 'null' || !assistant) assistant = null;
    if (assistant) device('shell', 'cmd', 'notification', 'disallow_assistant', assistant);
    report.passes.push(await pass(androidKey, 'post', 'no-assistant'));
    fs.writeFileSync(path.join(root, 'run.json'), JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report, null, 2));
  } finally {
    if (assistant) device('shell', 'cmd', 'notification', 'allow_assistant', assistant);
    if (granted) device('shell', 'cmd', 'notification', 'disallow_listener', LISTENER);
  }
})().catch(error => { console.error(error); console.error('artifacts:', root); process.exitCode = 1; });
