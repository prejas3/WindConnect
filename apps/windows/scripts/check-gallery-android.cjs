// CCS/1 PURPOSE:WN-58 real PC app + Android emulator: paired over QR, "Zdjęcia na komputerze" on (GalleryProof). Three new
// 4000×3000 JPEGs are saved on the phone the way a camera does (pending -> bytes -> published); each is timed from the
// script's command to its thumbnail painted in the panel (a MutationObserver in the page), budget < 1000 ms. Then a
// screenshot is marked as one, the original of a photo is fetched (as a press does) and compared by SHA-256 with the file on
// the phone, and the phone's setting off/on hides and shows the section.
// LIMIT:the measured time is an upper bound: it includes adb and the proof's 20 ms command poll. The native drag (OLE
// DoDragDrop) is not driven here: it needs a real mouse (the S25 + PC short test). No name is printed beyond the test's own.
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const { spawn, spawnSync } = require('node:child_process');
const fs = require('node:fs/promises');
const path = require('node:path');
const os = require('node:os');
const crypto = require('node:crypto');
const dgram = require('node:dgram');
const assert = require('node:assert/strict');
const root = path.join(os.tmpdir(), 'windconnect-wn58-gallery-' + crypto.randomUUID());
const adb = path.join(process.env.ANDROID_HOME, 'platform-tools/adb.exe');
const serial = process.env.ANDROID_SERIAL || 'emulator-5590';
const port = 19328;
const PKG = 'io.github.prejas3.windconnect';
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
function device(...args) { const result = spawnSync(adb, ['-s', serial, ...args], { windowsHide: true, encoding: 'utf8', timeout: 20000 }); assert.equal(result.status, 0, result.stderr); return result.stdout; }
let app, instrument, browser, socket, timer, redirected = false, output = '';
(async () => {
  try {
    await fs.mkdir(root, { recursive: true });
    device('install', '-r', 'apps/android/app/build/outputs/apk/debug/app-debug.apk');
    device('install', '-r', '-t', 'apps/android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk');
    device('shell', 'pm', 'grant', PKG, 'android.permission.POST_NOTIFICATIONS');
    device('shell', 'pm', 'grant', PKG, 'android.permission.READ_MEDIA_IMAGES');
    device('emu', 'redir', 'add', 'udp:47824:47821'); redirected = true;
    app = spawn(path.resolve('target/debug/windconnect-windows.exe'), [], { windowsHide: true, stdio: 'ignore', env: { ...process.env, WINDCONNECT_TEST_DATA_DIR: path.join(root, 'pc'), WINDCONNECT_TEST_PAIR_IP: '10.0.2.2', WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`, WEBVIEW2_USER_DATA_FOLDER: path.join(root, 'webview') } });
    for (let index = 0; index < 100; index++) { try { browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`, { timeout: 1000 }); break; } catch {} if (app.exitCode !== null) throw Error('PC app exited'); await delay(100); }
    assert(browser);
    let panel;
    for (let index = 0; index < 100; index++) { panel = browser.contexts().flatMap(c => c.pages()).find(p => p.url().includes('panel.html')); if (panel) break; await delay(100); }
    assert(panel);
    const invoke = (command, args) => panel.evaluate(({command,args}) => window.__TAURI__.core.invoke(command,args), {command,args});
    await panel.waitForFunction(async () => (await window.__TAURI__.core.invoke('pairing_snapshot')).phase === 'waiting');
    await invoke('panel_action', { action: 'show' });
    await panel.locator('#pairing-qr').waitFor({ state: 'visible' });
    const imageFile = path.join(root, 'desktop-qr.png'), offerFile = path.join(root, 'offer.txt'), javaFile = path.join(root, 'DecodeQr.java');
    await panel.locator('#pairing-qr').screenshot({ path: imageFile });
    await fs.writeFile(javaFile, `import javax.imageio.ImageIO; import java.io.File; import java.nio.file.*; import com.google.zxing.*; import com.google.zxing.common.HybridBinarizer; class DecodeQr { public static void main(String[] a) throws Exception { var i=ImageIO.read(new File(a[0])); var p=i.getRGB(0,0,i.getWidth(),i.getHeight(),null,0,i.getWidth()); var r=new MultiFormatReader().decode(new BinaryBitmap(new HybridBinarizer(new RGBLuminanceSource(i.getWidth(),i.getHeight(),p)))); Files.writeString(Path.of(a[1]),r.getText()); } }`);
    const decoded = spawnSync(path.join(process.env.JAVA_HOME, 'bin/java.exe'), ['-cp', process.env.ZXING_CORE_JAR, javaFile, imageFile, offerFile], { windowsHide: true, encoding: 'utf8', timeout: 10000 });
    assert.equal(decoded.status, 0, decoded.stderr);
    const link = new URL(await fs.readFile(offerFile, 'utf8'));
    const key = Buffer.from(link.searchParams.get('k'), 'base64url');
    const fingerprint = crypto.createHash('sha256').update(Buffer.concat([Buffer.from('302a300506032b6570032100', 'hex'), key])).digest();
    const mainPort = Number((await invoke('dev_pairing_address')).split(':').pop());
    const packet = Buffer.alloc(39); packet.write('WCD1'); packet[4] = 2; packet.writeUInt16BE(mainPort, 5); fingerprint.copy(packet, 7);
    socket = dgram.createSocket({ type: 'udp4', reuseAddr: true });
    await new Promise((resolve, reject) => { socket.once('error', reject); socket.bind({ port: 47821, address: '0.0.0.0', exclusive: false }, resolve); });
    timer = setInterval(() => socket.send(packet, 47824, '127.0.0.1'), 150);
    device('push', imageFile, '/data/local/tmp/windconnect-wn58-qr.png');
    device('shell', 'run-as', PKG, 'mkdir', '-p', 'cache');
    device('shell', `cat /data/local/tmp/windconnect-wn58-qr.png | run-as ${PKG} tee cache/desktop-qr.png >/dev/null`);
    instrument = spawn(adb, ['-s', serial, 'shell', 'am', 'instrument', '-w', '-r', '-e', 'class', `${PKG}.PairingTest#qrConnectsRealDesktop`, '-e', 'desktopQr', 'true', '-e', 'galleryProof', 'true', `${PKG}.test/androidx.test.runner.AndroidJUnitRunner`], { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
    instrument.stdout.on('data', data => { output += data; }); instrument.stderr.on('data', data => { output += data; });
    const until = async (what, label, ms = 10000) => {
      const start = Date.now();
      for (;;) {
        const value = await what();
        if (value) return value;
        assert(Date.now() - start < ms, label);
        assert.equal(instrument.exitCode, null, 'session held: ' + output);
        await delay(10);
      }
    };
    await until(async () => (await invoke('pairing_snapshot')).phase === 'connected', 'connected', 20000);
    await until(async () => (await invoke('gallery_state')).available, 'phone announces gallery/1', 15000);
    await invoke('panel_action', { action: 'show' });
    // The page notes when each photo's thumbnail is first painted.
    await panel.evaluate(() => {
      window.__shown = {};
      const mark = () => document.querySelectorAll('#gallery-list .photo').forEach(b => { if (b.querySelector('img[src]') && !window.__shown[b.dataset.id]) window.__shown[b.dataset.id] = Date.now(); });
      new MutationObserver(mark).observe(document.querySelector('#gallery-list'), { subtree: true, childList: true, attributes: true });
      mark();
    });
    const command = word => device('shell', `echo ${word} | run-as ${PKG} tee cache/wn58-cmd >/dev/null`);
    const readAck = () => { const r = spawnSync(adb, ['-s', serial, 'shell', 'run-as', PKG, 'cat', 'cache/wn58-ack'], { windowsHide: true, encoding: 'utf8' }); return r.status === 0 && r.stdout.trim() ? r.stdout.trim() : null; };
    const save = async (kind, n) => {
      device('shell', 'run-as', PKG, 'rm', '-f', 'cache/wn58-ack');
      const sent = Date.now();
      command(`${kind} ${n}`);
      const adbMs = Date.now() - sent;
      const ack = await until(async () => { const a = readAck(); return a && a.startsWith(`${n} `) ? a : null; }, `${kind} ${n} saved`);
      const [, id, insertMs] = ack.split(' ');
      const shown = await until(() => panel.evaluate(id => window.__shown[id], id), `${kind} ${n} shown`, 5000);
      return { n, id: Number(id), insertMs: Number(insertMs), adbMs, commandToShownMs: shown - sent };
    };
    const photos = [];
    for (const n of [1, 2, 3]) { photos.push(await save('photo', n)); await delay(500); }
    const shot = await save('screenshot', 4);
    const state = await invoke('gallery_state');
    assert.equal(state.photos[0].id, shot.id, 'newest first');
    assert.equal(state.photos[0].screenshot, true, 'screenshot marked');
    assert.equal(state.photos.find(p => p.id === photos[0].id).screenshot, false);
    await panel.locator('#gallery').screenshot({ path: path.join(root, 'pc-gallery.png') });
    // Full quality: what a press fetches is the phone's file, byte for byte.
    const first = photos[0];
    await invoke('gallery_prefetch', { id: first.id });
    const local = path.join(root, 'pc', 'gallery-temp', String(first.id), 'WN58_1.jpg');
    const bytes = await fs.readFile(local);
    const pcHash = crypto.createHash('sha256').update(bytes).digest('hex');
    const phoneHash = device('shell', 'sha256sum', '/sdcard/Pictures/WindConnectTest/WN58_1.jpg').trim().split(/\s+/)[0];
    assert.equal(pcHash, phoneHash, 'original intact');
    // The phone's setting: off hides the section, on brings it back.
    command('off');
    await until(() => panel.evaluate(() => document.querySelector('#gallery').hidden), 'setting off hides the photos');
    command('on');
    await until(() => panel.evaluate(() => !document.querySelector('#gallery').hidden && document.querySelectorAll('#gallery-list .photo').length >= 4), 'setting on shows them again');
    command('done');
    for (let attempt = 0; attempt < 200 && instrument.exitCode === null; attempt++) await delay(50);
    await fs.writeFile(path.join(root, 'instrumentation.log'), output);
    assert(output.includes('OK (1 test)'), output);
    const worst = Math.max(...[...photos, shot].map(p => p.commandToShownMs));
    const report = { photos, screenshot: shot, worstCommandToShownMs: worst, original: { bytes: bytes.length, sha256Match: true }, realSession: true, artifacts: root };
    await fs.writeFile(path.join(root, 'run.json'), JSON.stringify(report, null, 2)); await fs.rm(offerFile);
    console.log(JSON.stringify(report));
    assert(worst < 1000, `a new photo took ${worst} ms to show`);
    await invoke('tray_action', { action: 'quit' }).catch(() => {});
  } finally {
    await fs.writeFile(path.join(root, 'instrumentation.log'), output).catch(() => {});
    if (timer) clearInterval(timer); if (socket) socket.close();
    if (instrument && instrument.exitCode === null) { device('shell', 'am', 'force-stop', PKG); instrument.kill(); }
    if (browser) await browser.close();
    if (app && app.exitCode === null) app.kill();
    if (redirected) device('emu', 'redir', 'del', 'udp:47824');
  }
})().catch(error => { console.error(error); console.error('artifacts:', root); process.exitCode = 1; });
