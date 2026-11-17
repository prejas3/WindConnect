// CCS/1 PURPOSE:WN-126 an existing pairing survives updates, without pairing again: real QR pairing, then
// (A) the APK replaced in place while the PC runs, (B) the PC restarted with its kept identity and paired list,
// (C) the APK replaced while the PC is down, then the PC back. Each must end in a new authenticated session, with the
// app's main thread idle after it (WN-149),
// with no app crash (a crashed background service is restarted ever later, WN-126).
// SEC:disposable emulator only (ro.kernel.qemu), isolated peer identity in TEMP; no TLS/trust bypass, no re-pairing.
// PC_RESTARTS=n repeats (B). Needs: debug APK + androidTest APK, `cargo build -p windconnect-windows --example background_peer`,
// ANDROID_HOME, JAVA_HOME, ANDROID_SERIAL of your own emulator.
const { spawn, spawnSync } = require('node:child_process');
const fs = require('node:fs/promises');
const path = require('node:path');
const os = require('node:os');
const crypto = require('node:crypto');
const dgram = require('node:dgram');
const assert = require('node:assert/strict');
const root = path.join(os.tmpdir(), 'windconnect-wn126-' + crypto.randomUUID());
const adb = path.join(process.env.ANDROID_HOME, 'platform-tools/adb.exe');
const serial = process.env.ANDROID_SERIAL;
const pkg = 'io.github.prejas3.windconnect';
const apk = 'apps/android/app/build/outputs/apk/debug/app-debug.apk';
const redirectPort = Number(process.env.DISCOVERY_REDIRECT_PORT || 47823);
const limitMs = Number(process.env.RECONNECT_LIMIT_MS || 90000);
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
function device(...args) {
  const result = spawnSync(adb, ['-s', serial, ...args], { windowsHide: true, encoding: 'utf8', timeout: 60000 });
  assert.equal(result.status, 0, `${args.join(' ')}: ${result.stderr}${result.stdout}`); return result.stdout;
}
const read = async (name, fallback = '') => { try { return await fs.readFile(path.join(root, name), 'utf8'); } catch { return fallback; } };
let peer, peerOutput = '', socket, timer, packet, redirected = false, emulatorVerified = false;
async function startPeer() {
  await fs.rm(path.join(root, 'done'), { force: true });
  await fs.rm(path.join(root, 'port.txt'), { force: true });
  await fs.rm(path.join(root, 'sessions.txt'), { force: true });
  peer = spawn(path.resolve('target/debug/examples/background_peer.exe'), [root, '--keep'], { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
  peer.stderr.on('data', data => { peerOutput += data; });
  for (let attempt = 0; attempt < 200 && !(await read('port.txt')); attempt++) { assert.equal(peer.exitCode, null, peerOutput); await delay(50); }
  packet.writeUInt16BE(Number(await read('port.txt')), 5);
}
async function stopPeer() {
  await fs.writeFile(path.join(root, 'done'), '');
  for (let attempt = 0; attempt < 50 && peer.exitCode === null; attempt++) await delay(100);
  if (peer.exitCode === null) peer.kill();
  await fs.writeFile(path.join(root, 'connected.txt'), 'false');
}
// A new session (sessions.txt grew past `before`) that is live now.
// WN-149: after a reconnect the app's main thread is free: the reconnect loop runs there, and a wait that didn't suspend
// spun it (WN-148). Its CPU over 2 s (from /proc, ticks of 10 ms) stays under half a core, and an activity starts.
async function mainThreadIdle(label) {
  await delay(3000); // a few turns of the loop while connected
  const pid = device('shell', 'pidof', pkg).trim().split(/\s+/)[0];
  assert(pid, `${label}: app process`);
  const ticks = () => { const f = device('shell', 'run-as', pkg, 'cat', `/proc/${pid}/task/${pid}/stat`).split(') ')[1].split(' '); return Number(f[11]) + Number(f[12]); };
  const t0 = ticks(); await delay(2000); const used = (ticks() - t0) / 200;
  const start = device('shell', 'am', 'start', '-W', '-n', `${pkg}/.MainActivity`);
  device('shell', 'input', 'keyevent', 'KEYCODE_HOME');
  assert(used < 0.5, `${label}: main thread busy ${Math.round(used * 100)}% of a core after the reconnect`);
  assert(/Status: ok/.test(start), `${label}: activity start ${start}`);
  console.log(`${label}: main thread ${Math.round(used * 100)}% of a core, activity ${(start.match(/TotalTime: (\d+)/) || [])[1]} ms`);
  return Math.round(used * 100);
}
async function reconnected(label, before) {
  const started = Date.now();
  while (Date.now() - started < limitMs) {
    if (Number(await read('sessions.txt', '0')) > before && (await read('connected.txt')) === 'true') {
      const ms = Date.now() - started; console.log(`${label}: reconnected in ${ms} ms`); return ms;
    }
    await delay(250);
  }
  const services = device('shell', 'dumpsys', 'activity', 'services', pkg);
  await fs.writeFile(path.join(root, `${label}-services.txt`), services);
  await fs.writeFile(path.join(root, `${label}-logcat.txt`), device('logcat', '-d', '-t', '400'));
  const shown = device('shell', 'dumpsys', 'notification', '--noredact').split('NotificationRecord').filter(r => r.includes('pkg=' + pkg)).join('\n---\n');
  await fs.writeFile(path.join(root, `${label}-notification.txt`), shown);
  console.error(`${label}: phone shows: ${(shown.match(/android\.text=String \(([^)]*)\)/) || [])[1]}`);
  assert.fail(`${label}: no new session within ${limitMs} ms without pairing again (see ${root})`);
}
(async () => {
  try {
    assert(serial, 'set ANDROID_SERIAL to your own emulator');
    assert.equal(device('shell', 'getprop', 'ro.kernel.qemu').trim(), '1', 'requires a disposable emulator');
    emulatorVerified = true;
    await fs.mkdir(root, { recursive: true });
    device('install', '-r', apk);
    device('install', '-r', '-t', 'apps/android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk');
    device('shell', 'am', 'force-stop', pkg);
    device('shell', 'pm', 'clear', pkg);
    device('logcat', '-b', 'crash', '-c');
    device('emu', 'redir', 'add', 'udp:' + redirectPort + ':47821'); redirected = true;
    packet = Buffer.alloc(39); packet.write('WCD1'); packet[4] = 2;
    await startPeer();
    // The QR from the peer's own matrix (the desktop's production encoder), drawn without a second encoder.
    const offer = new URL(await read('offer.txt')), imageFile = path.join(root, 'desktop-qr.png'), javaFile = path.join(root, 'EncodeQr.java');
    await fs.writeFile(javaFile, `import java.nio.file.*; import java.awt.image.BufferedImage; import javax.imageio.ImageIO; class EncodeQr { public static void main(String[] a) throws Exception { var rows=Files.readAllLines(Path.of(a[0])); int w=Integer.parseInt(rows.get(0)),scale=3,size=(w+8)*scale; var bits=rows.get(1); var b=new BufferedImage(size,size,BufferedImage.TYPE_INT_RGB); for(int y=0;y<size;y++) for(int x=0;x<size;x++) { int mx=x/scale-4,my=y/scale-4; boolean dark=mx>=0&&my>=0&&mx<w&&my<w&&bits.charAt(my*w+mx)=='1'; b.setRGB(x,y,dark?0:0xffffff); } ImageIO.write(b,"png",Path.of(a[1]).toFile()); } }`);
    const encoded = spawnSync(path.join(process.env.JAVA_HOME, 'bin/java.exe'), [javaFile, path.join(root, 'matrix.txt'), imageFile], { windowsHide: true, encoding: 'utf8', timeout: 20000 });
    assert.equal(encoded.status, 0, encoded.stderr);
    crypto.createHash('sha256').update(Buffer.concat([Buffer.from('302a300506032b6570032100', 'hex'), Buffer.from(offer.searchParams.get('k'), 'base64url')])).digest().copy(packet, 7);
    socket = dgram.createSocket({ type: 'udp4', reuseAddr: true });
    await new Promise((resolve, reject) => { socket.once('error', reject); socket.bind({ port: 47821, address: '0.0.0.0', exclusive: false }, resolve); });
    timer = setInterval(() => socket.send(packet, redirectPort, '127.0.0.1'), 150);
    device('push', imageFile, '/data/local/tmp/windconnect-wn126-qr.png');
    device('shell', 'run-as', pkg, 'mkdir', '-p', 'cache');
    device('shell', `cat /data/local/tmp/windconnect-wn126-qr.png | run-as ${pkg} tee cache/desktop-qr.png >/dev/null`);
    // Async: a synchronous spawn would block the event loop, and with it the discovery packets the pairing needs.
    const paired = await new Promise((resolve, reject) => {
      const run = spawn(adb, ['-s', serial, 'shell', 'am', 'instrument', '-w', '-r', '-e', 'class', pkg + '.PairingTest#qrConnectsRealDesktop', '-e', 'desktopQr', 'true', '-e', 'backgroundService', 'true', '-e', 'doze', 'false', pkg + '.test/androidx.test.runner.AndroidJUnitRunner'], { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
      let text = ''; run.stdout.on('data', data => { text += data; }); run.stderr.on('data', data => { text += data; });
      run.on('error', reject); run.on('exit', () => resolve(text));
    });
    await fs.writeFile(path.join(root, 'pairing.log'), paired);
    assert(paired.includes('backgroundConnected=true') && paired.includes('OK (1 test)'), paired);
    const pairedFile = await read('paired.txt');
    const phoneKey = pairedFile.trim().split(/\s+/)[0];
    const report = { pairedOnce: true, phoneKey: phoneKey.slice(0, 8) + '…', pcFingerprint: packet.subarray(7, 15).toString('hex') + '…' };

    // (A) The instrumentation ended and killed the app process; a real in-place update replaces the package.
    let before = Number(await read('sessions.txt', '0'));
    device('install', '-r', apk);
    report.apkUpdateMs = await reconnected('A apk-update', before);
    report.mainThreadCpu = [await mainThreadIdle('A apk-update')];

    // (B) The PC app updated: it stops and starts again from its kept identity and paired list.
    // PC_RESTARTS repeats it (a restart that was missed once in four runs, WN-126).
    report.pcRestartMs = [];
    for (let restart = 1; restart <= Number(process.env.PC_RESTARTS || 1); restart++) {
      await stopPeer();
      await delay(3000);
      await startPeer();
      report.pcRestartMs.push(await reconnected(`B pc-restart-${restart}`, 0));
      report.mainThreadCpu.push(await mainThreadIdle(`B pc-restart-${restart}`));
    }

    // (C) Both updated, the phone first while the PC is down, as when installing the new pair of builds.
    await stopPeer();
    device('install', '-r', apk);
    await delay(Number(process.env.PC_DOWN_MS || 30000));
    await startPeer();
    report.bothUpdatedMs = await reconnected('C both-updated', 0);
    report.mainThreadCpu.push(await mainThreadIdle('C both-updated'));

    assert.equal(await read('paired.txt'), pairedFile, 'the PC paired list is unchanged: nothing paired again');
    const phonePaired = device('shell', 'run-as', pkg, 'cat', 'no_backup/paired.txt');
    assert.equal(phonePaired.trim().split(/\r?\n/).filter(line => line.trim()).length, 1, 'the phone holds the one pairing: ' + phonePaired);
    // A crash restarts the service at first, but Android puts each further restart off (30 min after the second).
    const crashes = device('logcat', '-b', 'crash', '-d');
    await fs.writeFile(path.join(root, 'crash.txt'), crashes);
    assert(!crashes.includes('Process: ' + pkg + ','), 'the app crashed during the run:\n' + crashes.slice(0, 2000));
    report.crashes = 0;
    report.artifacts = root;
    await fs.writeFile(path.join(root, 'run.json'), JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report));
  } finally {
    if (timer) clearInterval(timer); if (socket) socket.close();
    if (peer && peer.exitCode === null) { await fs.writeFile(path.join(root, 'done'), ''); for (let i = 0; i < 30 && peer.exitCode === null; i++) await delay(100); if (peer.exitCode === null) peer.kill(); }
    await fs.writeFile(path.join(root, 'peer.log'), peerOutput).catch(() => {});
    for (const name of ['offer.txt', 'matrix.txt', 'identity.key']) await fs.rm(path.join(root, name), { force: true });
    const cleanup = emulatorVerified ? [['shell', 'am', 'force-stop', pkg], ['shell', 'rm', '-f', '/data/local/tmp/windconnect-wn126-qr.png']] : [];
    if (redirected) cleanup.push(['emu', 'redir', 'del', 'udp:' + redirectPort]);
    for (const args of cleanup) { try { device(...args); } catch (error) { console.error('cleanup:', error.message); } }
  }
})().catch(error => { console.error(error); console.error('artifacts:', root); process.exitCode = 1; });
