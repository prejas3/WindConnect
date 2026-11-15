// CCS/1 PURPOSE:WN-123 real notification listener + consent cycle + new QUIC session; the Windows receiver must get idle only.
// DEP:debug app/test APKs, cargo build -p wind-ffi --example notify_peer; ANDROID_SERIAL must name an emulator.
const { spawn, spawnSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const assert = require('node:assert/strict');
const serial = process.env.ANDROID_SERIAL;
assert(serial?.startsWith('emulator-'), 'ANDROID_SERIAL must name the disposable test emulator');
const adb = path.join(process.env.ANDROID_HOME, 'platform-tools/adb.exe');
const runner = 'io.github.prejas3.windconnect.test/androidx.test.runner.AndroidJUnitRunner';
const listener = 'io.github.prejas3.windconnect/io.github.prejas3.windconnect.core.NotifyListenerService';
const root = fs.mkdtempSync(path.join(os.tmpdir(), 'windconnect-wn123-lifecycle-'));
const port = 47836;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
function device(...args) {
  const r = spawnSync(adb, ['-s', serial, ...args], { encoding: 'utf8', windowsHide: true, timeout: 120000 });
  assert.equal(r.status, 0, r.stdout + r.stderr);
  return r.stdout;
}
let peer, output = '';
(async () => {
  try {
    device('install', '-r', 'apps/android/app/build/outputs/apk/debug/app-debug.apk');
    device('install', '-r', '-t', 'apps/android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk');
    device('shell', 'pm', 'grant', 'io.github.prejas3.windconnect', 'android.permission.POST_NOTIFICATIONS');
    device('shell', 'cmd', 'notification', 'allow_listener', listener);
    const identity = device('shell', 'am', 'instrument', '-w', '-r', '-e', 'class',
      'io.github.prejas3.windconnect.CoreConnectionTest#identityLoadsThroughUniFFIAndSurvivesActivityRecreation', runner);
    fs.writeFileSync(path.join(root, 'identity.log'), identity);
    const androidKey = identity.match(/androidPublicKey=([0-9a-f]{64})/)?.[1];
    assert(androidKey, identity);
    peer = spawn(path.resolve('target/debug/examples/notify_peer.exe'), [androidKey, `127.0.0.1:${port}`, '100'], { windowsHide: true });
    peer.stdout.on('data', d => { output += d; });
    peer.stderr.on('data', d => { output += d; });
    for (let i = 0; i < 100 && !output.includes('PC_PUBLIC_KEY='); i++) await delay(50);
    const key = output.match(/PC_PUBLIC_KEY=([0-9a-f]{64})/)?.[1];
    assert(key, output);
    const result = device('shell', 'am', 'instrument', '-w', '-r', '-e', 'class',
      'io.github.prejas3.windconnect.CallNotificationLifecycleTest', '-e', 'pcAddress', `10.0.2.2:${port}`, '-e', 'pcKey', key, runner);
    fs.writeFileSync(path.join(root, 'instrumentation.log'), result);
    for (let i = 0; i < 100 && !output.includes('PEER_DONE'); i++) await delay(50);
    fs.writeFileSync(path.join(root, 'peer.log'), output);
    assert(result.includes('OK (1 test)'), result);
    const states = [...output.matchAll(/CALL_STATE=(\w+)/g)].map(m => m[1]);
    assert(states.length > 0, 'PC receives the new session call state');
    assert(states.every(s => s === 'Idle'), `PC must never receive stale ringing: ${states}`);
    console.log(JSON.stringify({ artifacts: root, states, result: 'PASS' }, null, 2));
  } finally {
    if (peer && peer.exitCode === null) {
      peer.kill();
      await new Promise(resolve => peer.once('close', resolve));
    }
    device('shell', 'cmd', 'notification', 'disallow_listener', listener);
  }
})().catch(error => { console.error(error, 'artifacts:', root); process.exitCode = 1; });
