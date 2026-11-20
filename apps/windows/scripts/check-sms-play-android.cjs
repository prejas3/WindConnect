// CCS/1 PURPOSE:WN-124 (decision D2) the Google Play build on a phone: real PC app + Android emulator with the Play APK
// (-PwindStore=play), paired by real QR. The phone never offers sms/1 (no READ_SMS in its manifest, no source), and the
// PC's "Messages" window says the phone doesn't share SMS. Also runs StoreVariantTest on the phone (no SMS/calls
// settings in the Play build).
// LIMIT:emulator NAT needs a UDP redirection for the public discovery hint; no trust or TLS bypass. Disposable emulator.
// Needs: cargo build -p windconnect-windows; in apps/android: ./gradlew :app:assembleDebug :app:assembleDebugAndroidTest
// -PwindStore=play; ANDROID_HOME, ANDROID_SERIAL, JAVA_HOME, ZXING_CORE_JAR, PLAYWRIGHT_MODULE.
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const { spawn, spawnSync } = require('node:child_process');
const fs = require('node:fs/promises');
const path = require('node:path');
const os = require('node:os');
const crypto = require('node:crypto');
const dgram = require('node:dgram');
const assert = require('node:assert/strict');
const root = path.join(os.tmpdir(), 'windconnect-wn124-play-' + crypto.randomUUID());
const adb = path.join(process.env.ANDROID_HOME, 'platform-tools/adb.exe');
const serial = process.env.ANDROID_SERIAL;
const pkg = 'io.github.prejas3.windconnect';
const apks = 'apps/android/app/build-play/outputs/apk';
const port = 19328;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
function device(...args) { const result = spawnSync(adb, ['-s', serial, ...args], { windowsHide: true, encoding: 'utf8', timeout: 60000 }); assert.equal(result.status, 0, result.stderr + result.stdout); return result.stdout; }
let app, instrument, browser, socket, timer, redirected = false, output = '';
(async () => {
  try {
    assert(serial, 'set ANDROID_SERIAL to your own emulator');
    assert.equal(device('shell', 'getprop', 'ro.kernel.qemu').trim(), '1', 'requires a disposable emulator');
    await fs.mkdir(root, { recursive: true });
    // The full build may be installed: the Play one has other permissions, so it replaces it cleanly.
    spawnSync(adb, ['-s', serial, 'uninstall', pkg]);
    device('install', '-r', path.join(apks, 'debug/app-play-debug.apk'));
    device('install', '-r', '-t', path.join(apks, 'androidTest/debug/app-play-debug-androidTest.apk'));
    const perms = device('shell', 'dumpsys', 'package', pkg);
    for (const p of ['READ_SMS', 'SEND_SMS', 'READ_CALL_LOG', 'READ_CONTACTS']) assert(!perms.includes('android.permission.' + p), `Play build requests ${p}`);
    const variant = device('shell', 'am', 'instrument', '-w', '-r', '-e', 'class', pkg + '.StoreVariantTest', '-e', 'windStore', 'play', pkg + '.test/androidx.test.runner.AndroidJUnitRunner');
    await fs.writeFile(path.join(root, 'store-variant.log'), variant);
    assert(variant.includes('storeVariant=play') && variant.includes('OK (1 test)'), variant);
    device('emu', 'redir', 'add', 'udp:47823:47821'); redirected = true;
    app = spawn(path.resolve('target/debug/windconnect-windows.exe'), [], { windowsHide: true, stdio: 'ignore', env: { ...process.env, WINDCONNECT_TEST_DATA_DIR: path.join(root, 'pc'), WINDCONNECT_TEST_PAIR_IP: '10.0.2.2', WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`, WEBVIEW2_USER_DATA_FOLDER: path.join(root, 'webview') } });
    for (let index = 0; index < 150; index++) { try { browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`, { timeout: 1000 }); break; } catch {} if (app.exitCode !== null) throw Error('PC app exited'); await delay(100); }
    assert(browser);
    let panel;
    for (let index = 0; index < 100; index++) { panel = browser.contexts().flatMap(c => c.pages()).find(p => p.url().includes('panel.html')); if (panel) break; await delay(100); }
    assert(panel);
    const invoke = (command, args) => panel.evaluate(({ command, args }) => window.__TAURI__.core.invoke(command, args), { command, args });
    await panel.waitForFunction(async () => (await window.__TAURI__.core.invoke('pairing_snapshot')).phase === 'waiting');
    await invoke('panel_action', { action: 'show' });
    await panel.locator('#pairing-qr').waitFor({ state: 'visible' });
    const imageFile = path.join(root, 'desktop-qr.png'), offerFile = path.join(root, 'offer.txt'), javaFile = path.join(root, 'DecodeQr.java');
    await panel.locator('#pairing-qr').screenshot({ path: imageFile });
    await fs.writeFile(javaFile, `import javax.imageio.ImageIO; import java.io.File; import java.nio.file.*; import com.google.zxing.*; import com.google.zxing.common.HybridBinarizer; class DecodeQr { public static void main(String[] a) throws Exception { var i=ImageIO.read(new File(a[0])); var p=i.getRGB(0,0,i.getWidth(),i.getHeight(),null,0,i.getWidth()); var r=new MultiFormatReader().decode(new BinaryBitmap(new HybridBinarizer(new RGBLuminanceSource(i.getWidth(),i.getHeight(),p)))); Files.writeString(Path.of(a[1]),r.getText()); } }`);
    const decoded = spawnSync(path.join(process.env.JAVA_HOME, 'bin/java.exe'), ['-cp', process.env.ZXING_CORE_JAR, javaFile, imageFile, offerFile], { windowsHide: true, encoding: 'utf8', timeout: 20000 });
    assert.equal(decoded.status, 0, decoded.stderr);
    const link = new URL(await fs.readFile(offerFile, 'utf8'));
    const fingerprint = crypto.createHash('sha256').update(Buffer.concat([Buffer.from('302a300506032b6570032100', 'hex'), Buffer.from(link.searchParams.get('k'), 'base64url')])).digest();
    const mainPort = Number((await invoke('dev_pairing_address')).split(':').pop());
    const packet = Buffer.alloc(39); packet.write('WCD1'); packet[4] = 2; packet.writeUInt16BE(mainPort, 5); fingerprint.copy(packet, 7);
    socket = dgram.createSocket({ type: 'udp4', reuseAddr: true });
    await new Promise((resolve, reject) => { socket.once('error', reject); socket.bind({ port: 47821, address: '0.0.0.0', exclusive: false }, resolve); });
    timer = setInterval(() => socket.send(packet, 47823, '127.0.0.1'), 150);
    device('push', imageFile, '/data/local/tmp/windconnect-wn124-qr.png');
    device('shell', 'run-as', pkg, 'mkdir', '-p', 'cache');
    device('shell', `cat /data/local/tmp/windconnect-wn124-qr.png | run-as ${pkg} tee cache/desktop-qr.png >/dev/null`);
    instrument = spawn(adb, ['-s', serial, 'shell', 'am', 'instrument', '-w', '-r', '-e', 'class', pkg + '.PairingTest#qrConnectsRealDesktop', '-e', 'desktopQr', 'true', '-e', 'holdMs', '60000', pkg + '.test/androidx.test.runner.AndroidJUnitRunner'], { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
    instrument.stdout.on('data', data => { output += data; }); instrument.stderr.on('data', data => { output += data; });
    const until = async (what, label, ms = 15000) => {
      const start = Date.now();
      for (;;) { const value = await what(); if (value) return value; assert(Date.now() - start < ms, label); await delay(50); }
    };
    await until(async () => (await invoke('pairing_snapshot')).phase === 'connected', 'connected: ' + output, 30000);
    await delay(3000); // capabilities settle; sms/1 would be in the first list
    const snapshot = await invoke('pairing_snapshot');
    assert(!snapshot.sms, 'the Play build never offers sms/1');
    assert.deepEqual(await invoke('sms_status'), { connected: true, available: false });
    await invoke('sms_open');
    let sms;
    await until(async () => (sms = browser.contexts().flatMap(c => c.pages()).find(p => p.url().includes('sms.html'))), 'sms window');
    await sms.waitForLoadState();
    const text = await until(() => sms.evaluate(() => document.querySelector('#threads-status')?.textContent || ''), 'status text');
    assert(text.includes('nie udostępnia SMS') && text.includes('Google Play'), text);
    await sms.screenshot({ path: path.join(root, 'pc-sms-window-play.png') });
    const report = { playPermissions: 'no SMS, Call Log, contacts', storeVariantTest: 'play', smsOffered: false, smsStatus: { connected: true, available: false }, windowText: text, artifacts: root };
    await fs.writeFile(path.join(root, 'run.json'), JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report));
  } finally {
    await fs.writeFile(path.join(root, 'instrumentation.log'), output).catch(() => {});
    await fs.rm(path.join(root, 'offer.txt'), { force: true });
    if (timer) clearInterval(timer); if (socket) socket.close();
    if (instrument && instrument.exitCode === null) { spawnSync(adb, ['-s', serial, 'shell', 'am', 'force-stop', pkg]); instrument.kill(); }
    if (redirected) spawnSync(adb, ['-s', serial, 'emu', 'redir', 'del', 'udp:47823']);
    try { await browser?.close(); } catch {}
    if (app && app.exitCode === null) app.kill();
  }
})().catch(error => { console.error(error); console.error('artifacts:', root); process.exitCode = 1; });
