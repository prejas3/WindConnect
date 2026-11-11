// CCS/1 PURPOSE:WN-90 - a second PC is found during a session with the first, within 5 s of a network change, with no MulticastLock.
// FLOW:android_peer (PC1 session) + PC2 discovery packet every 2 s via emulator UDP redirect -> MulticastLockTest#otherPcFoundDuringSessionWithoutLock
// LIMIT:the emulator's NAT carries the packet in unsolicited; the phone's query -> PC unicast answer is the host test
// query_is_answered_by_unicast_so_the_asker_needs_no_broadcast (wind-core). A physical phone is still required for the Wi-Fi chip.
// Needs: the debug and androidTest APKs built, `cargo build --locked -p wind-ffi --example android_peer`, ANDROID_HOME.
const { spawn, spawnSync } = require('node:child_process');
const fs = require('node:fs/promises');
const path = require('node:path');
const os = require('node:os');
const crypto = require('node:crypto');
const dgram = require('node:dgram');
const assert = require('node:assert/strict');
const root = path.join(os.tmpdir(), 'windconnect-wn90-' + crypto.randomUUID());
const adb = path.join(process.env.ANDROID_HOME, 'platform-tools', process.platform === 'win32' ? 'adb.exe' : 'adb');
const serial = process.env.ANDROID_SERIAL || 'emulator-5556';
const runner = 'io.github.prejas3.windconnect.test/androidx.test.runner.AndroidJUnitRunner';
const peerExe = path.resolve('target/debug/examples/android_peer' + (process.platform === 'win32' ? '.exe' : ''));
// Host ports, changeable when another emulator or test uses the defaults.
const PC_PORT = Number(process.env.PC_PORT || 47820);
const REDIR_PORT = Number(process.env.REDIR_PORT || 47823);
const HINT_PERIOD_MS = 2000; // a PC's broadcast query period (BroadcastConfig::default)
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
function device(...args) {
  const result = spawnSync(adb, ['-s', serial, ...args], { windowsHide: true, encoding: 'utf8', timeout: 120000 });
  assert.equal(result.status, 0, result.stderr);
  return result.stdout;
}
function instrumentArgs(test, extra = []) {
  return ['-s', serial, 'shell', 'am', 'instrument', '-w', '-r', '-e', 'class', `io.github.prejas3.windconnect.${test}`, ...extra, runner];
}
function instrument(test, extra = []) { return device(...instrumentArgs(test, extra).slice(2)); }
// Asynchronous, so the packet timer keeps running while the test does (spawnSync blocks the event loop).
function instrumentAsync(test, extra = []) {
  return new Promise((resolve, reject) => {
    const child = spawn(adb, instrumentArgs(test, extra), { windowsHide: true });
    let output = '';
    child.stdout.on('data', data => { output += data; });
    child.stderr.on('data', data => { output += data; });
    const timeout = setTimeout(() => child.kill(), 180000);
    child.on('error', reject);
    child.on('close', () => { clearTimeout(timeout); resolve(output); });
  });
}
let peer, socket, timer, redirected = false;
(async () => {
  try {
    await fs.mkdir(root, { recursive: true });
    device('install', '-r', '-d', 'apps/android/app/build/outputs/apk/debug/app-debug.apk');
    device('install', '-r', '-d', '-t', 'apps/android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk');
    const identity = instrument('CoreConnectionTest#identityLoadsThroughUniFFIAndSurvivesActivityRecreation');
    const androidKey = identity.match(/androidPublicKey=([0-9a-f]{64})/)?.[1];
    assert(androidKey, identity);

    peer = spawn(peerExe, [androidKey, `127.0.0.1:${PC_PORT}`], { windowsHide: true });
    let peerOutput = '';
    peer.stdout.on('data', data => { peerOutput += data; });
    peer.stderr.on('data', data => { peerOutput += data; });
    for (let i = 0; i < 100 && !/PC_PUBLIC_KEY=([0-9a-f]{64})/.test(peerOutput); i++) await delay(100);
    const pcKey = peerOutput.match(/PC_PUBLIC_KEY=([0-9a-f]{64})/)?.[1];
    assert(pcKey, peerOutput);

    // PC2: a public discovery packet (PROTOCOL.md §2.0.1) of a fingerprint nobody owns; discovery grants no trust.
    const otherPc = crypto.randomBytes(32);
    const packet = Buffer.alloc(39); packet.write('WCD1'); packet[4] = 1; packet.writeUInt16BE(47830, 5); otherPc.copy(packet, 7);
    device('emu', 'redir', 'add', `udp:${REDIR_PORT}:47821`); redirected = true;
    socket = dgram.createSocket({ type: 'udp4', reuseAddr: true });
    await new Promise((resolve, reject) => { socket.once('error', reject); socket.bind({ port: 47821, address: '0.0.0.0', exclusive: false }, resolve); });
    const send = () => socket.send(packet, REDIR_PORT, '127.0.0.1');
    send(); timer = setInterval(send, HINT_PERIOD_MS);

    const output = await instrumentAsync('MulticastLockTest#otherPcFoundDuringSessionWithoutLock',
      ['-e', 'pcAddress', `10.0.2.2:${PC_PORT}`, '-e', 'pcKey', pcKey, '-e', 'otherPc', otherPc.toString('hex')]);
    await fs.writeFile(path.join(root, 'instrumentation.log'), output + '\n--- android_peer ---\n' + peerOutput);
    assert(output.includes('OK (1 test)'), output);
    const number = name => Number(output.match(new RegExp(`${name}=(\\d+)`))?.[1]);
    const report = {
      hintPeriodMs: HINT_PERIOD_MS,
      otherFoundInSessionMs: number('otherFoundInSessionMs'),
      otherFoundAfterWifiOffMs: number('otherFoundAfterdisableMs'),
      otherFoundAfterWifiOnMs: number('otherFoundAfterenableMs'),
      sessionKept: /sessionKept=true/.test(output),
      lockHeldDuringSession: false,
      artifacts: root,
    };
    for (const key of ['otherFoundInSessionMs', 'otherFoundAfterWifiOffMs', 'otherFoundAfterWifiOnMs']) assert(report[key] < 5000, `${key}=${report[key]}`);
    await fs.writeFile(path.join(root, 'run.json'), JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report));
  } finally {
    if (timer) clearInterval(timer);
    if (socket) socket.close();
    if (peer && peer.exitCode === null) peer.kill();
    if (redirected) device('emu', 'redir', 'del', `udp:${REDIR_PORT}`);
  }
})().catch(error => { console.error(error); console.error('artifacts:', root); process.exitCode = 1; });
