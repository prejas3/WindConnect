// CCS/1 PURPOSE:WN-46 real PC app + Android emulator: notification-listener access granted (cmd notification allow_listener),
// pair, a test MediaSession on the phone (MediaProof) stands in for Spotify. The PC panel must show its title, artist and art;
// 10 clicks each on play/pause and next, and a seek, are measured as PC click -> the player's changed state back in the PC
// snapshot (a round trip: an upper bound of click -> player). Then NEXT leaves `actions` and the PC's further "next" must get
// no reaction (MediaProof checks). Raw ms in run.json.
// LIMIT:emulator NAT needs a UDP redirection for the public discovery hint; no trust or TLS bypass. Spotify itself isn't on
// the emulator. Restores the listener access.
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const { spawn, spawnSync } = require('node:child_process');
const fs = require('node:fs/promises');
const path = require('node:path');
const os = require('node:os');
const crypto = require('node:crypto');
const dgram = require('node:dgram');
const assert = require('node:assert/strict');
const root = path.join(os.tmpdir(), 'windconnect-wn46-media-' + crypto.randomUUID());
const adb = path.join(process.env.ANDROID_HOME, 'platform-tools/adb.exe');
const serial = process.env.ANDROID_SERIAL || 'emulator-5556';
const port = 19326;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
function device(...args) { const result = spawnSync(adb, ['-s', serial, ...args], { windowsHide: true, encoding: 'utf8', timeout: 20000 }); assert.equal(result.status, 0, result.stderr); return result.stdout; }
const LISTENER = 'io.github.prejas3.windconnect/io.github.prejas3.windconnect.core.MediaListenerService';
let app, instrument, browser, socket, timer, redirected = false, granted = false, output = '';
(async () => {
  try {
    await fs.mkdir(root, { recursive: true });
    device('install', '-r', 'apps/android/app/build/outputs/apk/debug/app-debug.apk');
    device('install', '-r', '-t', 'apps/android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk');
    device('shell', 'cmd', 'notification', 'allow_listener', LISTENER); granted = true;
    device('emu', 'redir', 'add', 'udp:47823:47821'); redirected = true;
    app = spawn(path.resolve('target/debug/windconnect-windows.exe'), [], { windowsHide: true, stdio: 'ignore', env: { ...process.env, WINDCONNECT_TEST_DATA_DIR: path.join(root, 'pc'), WINDCONNECT_TEST_PAIR_IP: '10.0.2.2', WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`, WEBVIEW2_USER_DATA_FOLDER: path.join(root, 'webview') } });
    for (let index = 0; index < 100; index++) { try { browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`, { timeout: 1000 }); break; } catch {} if (app.exitCode !== null) throw Error('PC app exited'); await delay(100); }
    assert(browser);
    let panel;
    for (let index = 0; index < 100; index++) { panel = browser.contexts().flatMap(c => c.pages()).find(p => p.url().includes('panel.html')); if (panel) break; await delay(100); }
    assert(panel);
    const invoke = (command, args) => panel.evaluate(({command,args}) => window.__TAURI__.core.invoke(command,args), {command,args});
    await panel.waitForFunction(async () => (await window.__TAURI__.core.invoke('pairing_snapshot')).phase === 'waiting');
    // WC-34: an empty store opens onboarding in the main window; the panel (same pairing window) is opened explicitly.
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
    timer = setInterval(() => socket.send(packet, 47823, '127.0.0.1'), 150);
    device('push', imageFile, '/data/local/tmp/windconnect-wn25-qr.png');
    device('shell', 'run-as', 'io.github.prejas3.windconnect', 'mkdir', '-p', 'cache');
    device('shell', 'cat /data/local/tmp/windconnect-wn25-qr.png | run-as io.github.prejas3.windconnect tee cache/desktop-qr.png >/dev/null');
    instrument = spawn(adb, ['-s', serial, 'shell', 'am', 'instrument', '-w', '-r', '-e', 'class', 'io.github.prejas3.windconnect.PairingTest#qrConnectsRealDesktop', '-e', 'desktopQr', 'true', '-e', 'mediaProof', 'true', 'io.github.prejas3.windconnect.test/androidx.test.runner.AndroidJUnitRunner'], { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
    instrument.stdout.on('data', data => { output += data; }); instrument.stderr.on('data', data => { output += data; });
    const deadline = Date.now() + 20000;
    let state;
    do { state = await invoke('pairing_snapshot'); if (state.phase === 'connected') break; if (instrument.exitCode !== null) break; await delay(25); } while (Date.now() < deadline);
    assert.equal(state.phase, 'connected', output);
    // WN-46: the phone's (test) player in the real panel; each click waits for the player's changed state to come back.
    const media = async (want, label, limit = 8000) => {
      const start = Date.now();
      for (;;) {
        const now = (await invoke('pairing_snapshot')).media;
        if (now && want(now)) return now;
        assert(Date.now() - start < limit, `${label}: ${JSON.stringify(now && { ...now, art: !!now.art })}`);
        assert.equal(instrument.exitCode, null, 'session held: ' + output);
        await delay(2);
      }
    };
    const shown = await media(m => m.title === 'Proof song 1' && m.art, 'the player shows up with its art', 20000);
    assert.equal(shown.artist, 'WindConnect test');
    assert.equal(shown.duration_ms, 200000);
    assert.equal(shown.actions, 31);
    await panel.locator('#media:not([hidden])').waitFor();
    assert.equal(await panel.locator('#media-title').textContent(), 'Proof song 1');
    assert.equal(await panel.locator('#media-art').evaluate(i => i.complete && i.naturalWidth > 0 && i.naturalWidth <= 256), true, 'art ≤ 256 px decoded');
    await panel.locator('#media').screenshot({ path: path.join(root, 'pc-media.png') });
    const report = { toggleMs: [], nextMs: [], seekMs: null };
    for (let i = 0; i < 10; i++) {
      const before = (await invoke('pairing_snapshot')).media.playing;
      const t = Date.now();
      await panel.locator('#media-toggle').click();
      await media(m => m.playing !== before, `toggle ${i}`);
      report.toggleMs.push(Date.now() - t);
    }
    for (let i = 0; i < 10; i++) {
      const before = (await invoke('pairing_snapshot')).media.title;
      const t = Date.now();
      await panel.locator('#media-next').click();
      await media(m => m.title !== before, `next ${i}`);
      report.nextMs.push(Date.now() - t);
    }
    {
      const t = Date.now();
      const bar = await panel.locator('#media-progress').boundingBox();
      await panel.mouse.click(bar.x + bar.width * 0.75, bar.y + bar.height / 2);
      await media(m => Math.abs(m.position_ms - 150000) < 5000, 'seek to 3/4');
      report.seekMs = Date.now() - t;
    }
    // NEXT leaves the player's actions: the panel disables it, and a direct "next" must not reach the player.
    await media(m => (m.actions & 4) === 0, 'next no longer allowed');
    assert.equal(await panel.locator('#media-next').isDisabled(), true);
    for (let i = 0; i < 3; i++) { await invoke('media_action', { action: 'next' }); await delay(200); }
    const sorted = a => [...a].sort((x, y) => x - y);
    const p = (a, q) => sorted(a)[Math.min(a.length - 1, Math.floor(q * a.length))];
    Object.assign(report, { toggleP50: p(report.toggleMs, 0.5), toggleMax: Math.max(...report.toggleMs), nextP50: p(report.nextMs, 0.5), nextMax: Math.max(...report.nextMs) });
    assert(report.toggleMax < 100 && report.nextMax < 100, `round trips under 100 ms: ${JSON.stringify(report)}`);
    for (let attempt = 0; attempt < 600 && instrument.exitCode === null; attempt++) await delay(50);
    await fs.writeFile(path.join(root, 'instrumentation.log'), output);
    assert(output.includes('OK (1 test)'), output);
    report.proof = output.match(/mediaProof=([^\n]*)/)?.[1];
    assert(output.includes('disallowedIgnored=true'), output);
    Object.assign(report, { phone: state.phone, realSession: true, artifacts: root });
    await fs.writeFile(path.join(root, 'run.json'), JSON.stringify(report, null, 2)); await fs.rm(offerFile);
    console.log(JSON.stringify(report));
    await invoke('tray_action', { action: 'quit' }).catch(() => {});
  } finally {
    await fs.writeFile(path.join(root, 'instrumentation.log'), output).catch(() => {});
    if (timer) clearInterval(timer); if (socket) socket.close();
    if (instrument && instrument.exitCode === null) { device('shell', 'am', 'force-stop', 'io.github.prejas3.windconnect'); instrument.kill(); }
    if (browser) await browser.close();
    if (app && app.exitCode === null) app.kill();
    if (redirected) device('emu', 'redir', 'del', 'udp:47823');
    if (granted) device('shell', 'cmd', 'notification', 'disallow_listener', LISTENER);
  }
})().catch(error => { console.error(error); console.error('artifacts:', root); process.exitCode = 1; });
