// CCS/1 FLOW:GSM ring->production mute->kill captured app PID (not force-stop)->GSM idle->manifest restore, no WindCore/UI.
// DEP:debug app/test APKs; ANDROID_SERIAL names a disposable emulator; fixture leaves no changes on a physical device.
const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const assert = require('node:assert/strict');
const serial = process.env.ANDROID_SERIAL;
assert(serial?.startsWith('emulator-'), 'ANDROID_SERIAL must name the disposable emulator');
const adb = path.join(process.env.ANDROID_HOME, 'platform-tools/adb.exe');
const pkg = 'io.github.prejas3.windconnect';
const runner = `${pkg}.test/androidx.test.runner.AndroidJUnitRunner`;
const root = fs.mkdtempSync(path.join(os.tmpdir(), 'windconnect-wn132-crash-'));
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
function device(...args) {
  const r = spawnSync(adb, ['-s', serial, ...args], { encoding: 'utf8', windowsHide: true, timeout: 90000 });
  assert.equal(r.status, 0, r.stdout + r.stderr);
  return r.stdout;
}
function instrument(method) {
  const text = device('shell', 'am', 'instrument', '-w', '-r', '-e', 'class', `${pkg}.RingMuteCrashFixture#${method}`,
    '-e', 'ringerCrashFixture', 'true', runner);
  fs.writeFileSync(path.join(root, `${method}.log`), text);
  assert(text.includes('OK (1 test)'), text);
  return text;
}
function ownership() { return device('shell', 'run-as', pkg, 'cat', 'shared_prefs/wind_calls.xml'); }
(async () => {
  let calling = false;
  try {
    device('install', '-r', 'apps/android/app/build/outputs/apk/debug/app-debug.apk');
    device('install', '-r', '-t', 'apps/android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk');
    device('shell', 'pm', 'grant', pkg, 'android.permission.READ_PHONE_STATE');
    device('emu', 'gsm', 'call', '5550132'); calling = true;
    await delay(1500);
    const seeded = instrument('seedMute');
    const pid = seeded.match(/mutedPid=(\d+)/)?.[1];
    assert(pid, seeded);
    assert(ownership().includes('name="ring_muted_for_pc" value="true"'), 'ownership persisted before death');
    // CCS/1 SEC:only the exact PID reported by our instrumentation; run-as enforces the app UID.
    const live = device('shell', `if [ -d /proc/${pid} ]; then echo alive; fi`).trim();
    if (live === 'alive') device('shell', 'run-as', pkg, 'kill', '-9', pid);
    await delay(500);
    assert.equal(device('shell', `if [ -d /proc/${pid} ]; then echo alive; fi`).trim(), '', 'original process ended');
    assert(ownership().includes('name="ring_muted_for_pc" value="true"'), 'no core restart has restored it early');
    device('emu', 'gsm', 'cancel', '5550132'); calling = false;
    const deadline = Date.now() + 15000;
    while (!ownership().includes('name="ring_muted_for_pc" value="false"')) {
      assert(Date.now() < deadline, 'real IDLE receiver must clear ownership before another instrumentation starts');
      await delay(100);
    }
    instrument('verifyRestoredWithoutCore');
    const report = { result: 'PASS', killedPid: Number(pid), restoredBy: 'real PHONE_STATE IDLE', artifacts: root };
    fs.writeFileSync(path.join(root, 'proof.json'), JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report, null, 2));
  } finally {
    if (calling) device('emu', 'gsm', 'cancel', '5550132');
  }
})().catch(error => { console.error(error, 'artifacts:', root); process.exitCode = 1; });
