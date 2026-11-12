// CCS/1 PURPOSE:WN-43 real PC app (debug) + Android emulator: the phone's notifications as native toasts (PROTOCOL.md §9.7).
// Pairs by QR as check-media-android does, then NotifyProof on the phone posts 20 shell notifications, a chat, a media one, an
// unchanged re-post and snoozes one. Here: the PC showed exactly the 21 (dev_notify_log), Windows holds 20 phone toasts afterwards
// (dev_notify_history: the snoozed one removed), at most 5 popups in any 2 s. Latency per notification, wall clocks corrected by
// the emulator/host offset: the phone's listener callback -> ToastNotifier.Show returned on the PC, p95 < 250 ms (asserted);
// Android's postTime -> Show reported with the notification assistant on (first pass) and off (second pass, paired reconnect).
// LIMIT:emulator NAT needs a UDP redirection for the discovery hint; no trust or TLS bypass. Restores the listener access and the
// assistant, and removes the phone toasts it made (they are PowerShell's in debug builds).
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const { spawn, spawnSync } = require('node:child_process');
const fs = require('node:fs/promises');
const path = require('node:path');
const os = require('node:os');
const crypto = require('node:crypto');
const dgram = require('node:dgram');
const assert = require('node:assert/strict');
const root = path.join(os.tmpdir(), 'windconnect-wn43-toasts-' + crypto.randomUUID());
const adb = path.join(process.env.ANDROID_HOME, 'platform-tools/adb.exe');
const serial = process.env.ANDROID_SERIAL || 'emulator-5590';
const port = 19327;
const runner = 'io.github.prejas3.windconnect.test/androidx.test.runner.AndroidJUnitRunner';
const LISTENER = 'io.github.prejas3.windconnect/io.github.prejas3.windconnect.core.NotifyListenerService';
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
function device(...args) { const result = spawnSync(adb, ['-s', serial, ...args], { windowsHide: true, encoding: 'utf8', timeout: 60000 }); assert.equal(result.status, 0, result.stderr + result.stdout); return result.stdout; }
const sorted = a => [...a].sort((x, y) => x - y);
const pct = (a, q) => sorted(a)[Math.min(a.length - 1, Math.floor(q * a.length))];
const stats = a => ({ p50: pct(a, 0.5), p95: pct(a, 0.95), max: Math.max(...a) });

// The emulator's wall clock minus the host's, ms (midpoint of the adb round trip; best of 5).
function clockOffset() {
  let best = null;
  for (let i = 0; i < 5; i++) {
    const t0 = Date.now();
    const out = device('shell', 'echo $EPOCHREALTIME');
    const t1 = Date.now();
    const phone = Math.round(parseFloat(out.trim()) * 1000);
    if (!best || t1 - t0 < best.rtt) best = { rtt: t1 - t0, offset: phone - (t0 + t1) / 2 };
  }
  return best;
}

let app, browser, socket, timer, redirected = false, granted = false, assistant = null, invoke = null;
// The phone's test turns the assistant off and on itself; restored here too if it was cut short.
(async () => {
  try {
    await fs.mkdir(root, { recursive: true });
    device('install', '-r', 'apps/android/app/build/outputs/apk/debug/app-debug.apk');
    device('install', '-r', '-t', 'apps/android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk');
    device('shell', 'cmd', 'notification', 'allow_listener', LISTENER); granted = true;
    assistant = device('shell', 'cmd', 'notification', 'get_approved_assistant').trim();
    if (!assistant.includes('/')) assistant = null;
    device('shell', 'am', 'force-stop', 'com.android.shell'); // at most 50 notifications per app: clear earlier runs'
    device('emu', 'redir', 'add', 'udp:47823:47821'); redirected = true;
    app = spawn(path.resolve('target/debug/windconnect-windows.exe'), [], { windowsHide: true, stdio: 'ignore', env: { ...process.env, WINDCONNECT_TEST_DATA_DIR: path.join(root, 'pc'), WINDCONNECT_TEST_PAIR_IP: '10.0.2.2', WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`, WEBVIEW2_USER_DATA_FOLDER: path.join(root, 'webview') } });
    for (let index = 0; index < 100; index++) { try { browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`, { timeout: 1000 }); break; } catch {} if (app.exitCode !== null) throw Error('PC app exited'); await delay(100); }
    assert(browser);
    let panel;
    for (let index = 0; index < 100; index++) { panel = browser.contexts().flatMap(c => c.pages()).find(p => p.url().includes('panel.html')); if (panel) break; await delay(100); }
    assert(panel);
    invoke = (command, args) => panel.evaluate(({command,args}) => window.__TAURI__.core.invoke(command,args), {command,args});
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
    timer = setInterval(() => socket.send(packet, 47823, '127.0.0.1'), 150);
    device('push', imageFile, '/data/local/tmp/windconnect-wn25-qr.png');
    device('shell', 'run-as', 'io.github.prejas3.windconnect', 'mkdir', '-p', 'cache');
    device('shell', 'cat /data/local/tmp/windconnect-wn25-qr.png | run-as io.github.prejas3.windconnect tee cache/desktop-qr.png >/dev/null');
    const clock = clockOffset();
    const report = { clockOffsetMs: Math.round(clock.offset), clockRttMs: clock.rtt, passes: [], artifacts: root };

    // One instrumentation: QR pairing, then NotifyProof twice on that session (assistant on: "listener"; off: "post").
    const tag = `t${Date.now() % 1000000}x`;
    // Async: the discovery hint timer must keep running while the phone connects.
    const output = await new Promise((resolve, reject) => {
      const child = spawn(adb, ['-s', serial, 'shell', 'am', 'instrument', '-w', '-r', '-e', 'class', 'io.github.prejas3.windconnect.PairingTest#qrConnectsRealDesktop',
        '-e', 'desktopQr', 'true', '-e', 'notifyProof', 'true', '-e', 'tag', tag, runner], { windowsHide: true });
      let out = '';
      const killer = setTimeout(() => child.kill(), 300000);
      child.stdout.on('data', d => { out += d; }); child.stderr.on('data', d => { out += d; });
      child.on('error', reject);
      child.on('close', () => { clearTimeout(killer); resolve(out); });
    });
    await fs.writeFile(path.join(root, 'instrumentation.log'), output);
    assert(output.includes('OK (1 test)'), output);
    await delay(500);
    const log = await invoke('dev_notify_log');
    const phones = [...output.matchAll(/notifyProof=([^\r\n]*)/g)].map(m => m[1]);
    const timeLists = [...output.matchAll(/notifyTimes=([^\r\n]*)/g)].map(m => m[1].split(',').map(p => p.split(':').map(Number)));
    assert.equal(timeLists.length, 2, 'two passes on the phone');
    assert.equal(log.length, 42, `the PC showed the 20 and the chat of each pass, nothing else: ${log.length}`);
    const history = await invoke('dev_notify_history');
    await panel.locator('body').screenshot({ path: path.join(root, 'pc-panel.png') }).catch(() => {});
    for (const [index, [label, measure]] of [['assistant', 'listener'], ['no-assistant', 'post']].entries()) {
      const part = log.slice(index * 21, index * 21 + 21);
      const times = timeLists[index];
      assert.equal(times.length, 21);
      // In order on both sides (one stream, one worker): the phone's i-th sent is the PC's i-th shown.
      const listenerToShown = part.map((e, i) => e.shown_ms - (times[i][0] - clock.offset));
      const postToShown = part.map((e, i) => e.shown_ms - (times[i][1] - clock.offset));
      const receivedToShown = part.map(e => e.show_us / 1000);
      // At most 5 popups in any 2 s.
      const popups = part.filter(e => e.popup).map(e => e.shown_ms);
      for (const t of popups) assert(popups.filter(u => u >= t && u - t < 2000).length <= 5, 'popup burst limit');
      const ours = part.map(e => e.tag);
      assert(!history.includes(part[1].tag), 'Removed took the snoozed one (the second) off');
      assert.equal(ours.filter(t => history.includes(t)).length, 20, 'Windows holds the other 20');
      const result = { label, phone: phones[index], listenerToShown: stats(listenerToShown), postToShown: stats(postToShown), receivedToShownMs: stats(receivedToShown), popups: popups.length, raw: { listenerToShown, postToShown } };
      assert(result.listenerToShown.p95 < 250, `listener -> toast p95 < 250 ms: ${JSON.stringify(result)}`);
      report.passes.push(result);
    }
    // The icon: one app, one file named by its key, re-encoded by Windows as a 64 x 64 PNG (IHDR width/height).
    const iconDir = path.join(root, 'pc', 'notify-icons');
    const icons = (await fs.readdir(iconDir)).filter(f => f.endsWith('.png'));
    assert.equal(icons.length, 1, `one icon for the shell: ${icons}`);
    assert.match(icons[0], /^[0-9a-f]{16}\.png$/);
    const png = await fs.readFile(path.join(iconDir, icons[0]));
    assert(png.subarray(0, 4).equals(Buffer.from([0x89, 0x50, 0x4e, 0x47])) && png.readUInt32BE(16) === 64 && png.readUInt32BE(20) === 64, 'a 64 px PNG');
    report.icon = { file: icons[0], bytes: png.length };
    // The setting off removes every phone toast from Windows and the icon cache (WN-43 review, point 2).
    await invoke('phone_notifications', { enabled: false });
    for (let i = 0; i < 50 && (await invoke('dev_notify_history')).length; i++) await delay(100);
    assert.deepEqual(await invoke('dev_notify_history'), [], 'no phone toasts left in Windows');
    await assert.rejects(fs.access(iconDir), 'the icon cache is gone');
    report.clearedOnSettingOff = true;
    await fs.writeFile(path.join(root, 'run.json'), JSON.stringify(report, null, 2)); await fs.rm(offerFile);
    console.log(JSON.stringify({ ...report, passes: report.passes.map(({ raw, ...p }) => p) }, null, 2));
  } finally {
    if (invoke) await invoke('phone_notifications', { enabled: false }).catch(() => {}); // removes the phone toasts it made
    await delay(300);
    if (invoke) await invoke('tray_action', { action: 'quit' }).catch(() => {});
    if (timer) clearInterval(timer); if (socket) socket.close();
    if (browser) await browser.close();
    if (app && app.exitCode === null) { await delay(500); if (app.exitCode === null) app.kill(); }
    if (assistant && !device('shell', 'cmd', 'notification', 'get_approved_assistant').includes(assistant)) device('shell', 'cmd', 'notification', 'allow_assistant', assistant);
    if (redirected) device('emu', 'redir', 'del', 'udp:47823');
    if (granted) device('shell', 'cmd', 'notification', 'disallow_listener', LISTENER);
  }
})().catch(error => { console.error(error); console.error('artifacts:', root); process.exitCode = 1; });
