// CCS/1 PURPOSE:WN-50 real PC app + Android emulator: pair, then three rings from the panel's "Ring" tile. (1) screen off +
// ringer SILENT: PC request -> phone "ringing" (its sound already playing) < 500 ms, an alarm player started, alarm volume at
// max; stop from the PC restores it. (2) stopped on the phone from the notification, or with FIND_NOTIFICATIONS=denied
// (POST_NOTIFICATIONS refused for the run) from the app's bar. (3) stopped with a volume key. Phone side: FindProof.
// LIMIT:emulator NAT needs a UDP redirection for the public discovery hint; no trust or TLS bypass. Restores ringer mode,
// alarm volume, screen and the notification permission. The emulator needs an audio device (QEMU_AUDIO_DRV=none, not -no-audio).
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const { spawn, spawnSync } = require('node:child_process');
const fs = require('node:fs/promises');
const path = require('node:path');
const os = require('node:os');
const crypto = require('node:crypto');
const dgram = require('node:dgram');
const assert = require('node:assert/strict');
const root = path.join(os.tmpdir(), 'windconnect-wn50-find-' + crypto.randomUUID());
const adb = path.join(process.env.ANDROID_HOME, 'platform-tools/adb.exe');
const serial = process.env.ANDROID_SERIAL || 'emulator-5556';
const port = 19326;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
function device(...args) { const result = spawnSync(adb, ['-s', serial, ...args], { windowsHide: true, encoding: 'utf8', timeout: 20000 }); assert.equal(result.status, 0, result.stderr); return result.stdout; }
// FIND_NOTIFICATIONS=denied: POST_NOTIFICATIONS refused for the run; the app's bar stops ring 2.
const denied = process.env.FIND_NOTIFICATIONS === 'denied';
const stops = denied ? ['bar', 'volume'] : ['notification', 'volume'];
let app, instrument, browser, socket, timer, redirected = false, touched = false, alarmBefore = null, output = '';
(async () => {
  try {
    await fs.mkdir(root, { recursive: true });
    device('install', '-r', 'apps/android/app/build/outputs/apk/debug/app-debug.apk');
    device('install', '-r', '-t', 'apps/android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk');
    device('shell', 'pm', denied ? 'revoke' : 'grant', 'io.github.prejas3.windconnect', 'android.permission.POST_NOTIFICATIONS');
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
    instrument = spawn(adb, ['-s', serial, 'shell', 'am', 'instrument', '-w', '-r', '-e', 'class', 'io.github.prejas3.windconnect.PairingTest#qrConnectsRealDesktop', '-e', 'desktopQr', 'true', '-e', 'findProof', 'true', '-e', 'findStops', stops.join(','), ...(denied ? ['-e', 'notifications', 'denied'] : []), 'io.github.prejas3.windconnect.test/androidx.test.runner.AndroidJUnitRunner'], { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
    instrument.stdout.on('data', data => { output += data; }); instrument.stderr.on('data', data => { output += data; });
    const deadline = Date.now() + 20000;
    let state;
    do { state = await invoke('pairing_snapshot'); if (state.phase === 'connected') break; if (instrument.exitCode !== null) break; await delay(25); } while (Date.now() < deadline);
    assert.equal(state.phase, 'connected', output);
    // WN-50: rings driven from the real panel tile; each waits for the pushed snapshot.
    const PKG = 'io.github.prejas3.windconnect';
    const sh = cmd => device('shell', cmd);
    const volume = () => sh('cmd media_session volume --stream 4 --get').match(/volume is (\d+) in range \[\d+\.\.(\d+)\]/);
    const alarm = () => Number(volume()[1]);
    const alarmMax = () => Number(volume()[2]);
    const alarmPlaying = () => sh('dumpsys audio').split('\n').some(l => /USAGE_ALARM/.test(l) && /state:started/.test(l));
    const find = async (want, label) => {
      const start = Date.now();
      for (;;) {
        const now = (await invoke('pairing_snapshot')).find;
        if (want(now)) return now;
        assert(Date.now() - start < 8000, `${label}: ${JSON.stringify(now)}`);
        assert.equal(instrument.exitCode, null, 'session held: ' + output);
        await delay(10);
      }
    };
    const tile = panel.locator('#find-phone');
    await find(f => f.available, 'phone announces find/1');
    alarmBefore = alarm(); touched = true;
    const max = alarmMax();
    sh('cmd audio set-volume 4 1');
    sh('cmd audio set-ringer-mode SILENT');
    sh('input keyevent KEYCODE_SLEEP');
    await delay(1500);
    const report = { rings: [] };

    // 1. Screen off, silent: the sound starts < 500 ms after the request, on the alarm stream at max; the PC stops it.
    const clicked = Date.now();
    await tile.click();
    const rang = await find(f => f.state === 'ringing', 'ring 1');
    const clickToSnapshotMs = Date.now() - clicked;
    assert(rang.ring_ms < 500, `phone rang ${rang.ring_ms} ms after the request`);
    const playing = alarmPlaying();
    if (!playing) await fs.writeFile(path.join(root, 'dumpsys-audio.txt'), sh('dumpsys audio'));
    assert(playing, 'an alarm player is started on the phone (dumpsys-audio.txt)');
    assert.equal(alarm(), max, 'alarm volume raised to max');
    assert.equal((await tile.textContent()).trim(), 'Zatrzymaj');
    await panel.screenshot({ path: path.join(root, 'pc-find-ringing.png') });
    await tile.click();
    const stopped = await find(f => f.state === 'idle' && f.last, 'stop from the PC');
    assert.equal(stopped.last, 'Zatrzymano');
    await delay(300);
    assert(!alarmPlaying(), 'sound stopped');
    assert.equal(alarm(), 1, 'alarm volume restored');
    report.rings.push({ screenOff: true, silent: true, requestToRingingMs: rang.ring_ms, clickToSnapshotMs, stoppedBy: 'pc' });

    // 2–4. The phone stops them (FindProof): its notification action, the app's bar with notifications blocked, a volume key.
    sh('input keyevent KEYCODE_WAKEUP'); sh('wm dismiss-keyguard'); await delay(1000);
    for (const [index, stoppedBy] of stops.entries()) {
      await tile.click();
      const ringing = await find(f => f.state === 'ringing', `ring ${index + 2}`);
      const ended = await find(f => f.state === 'idle' && f.last, `ring ${index + 2} stopped by ${stoppedBy}`);
      assert.equal(ended.last, 'Zatrzymano na telefonie', stoppedBy);
      await delay(300);
      assert(!alarmPlaying(), `sound stopped (${stoppedBy})`);
      assert.equal(alarm(), 1, `alarm volume restored (${stoppedBy})`);
      report.rings.push({ requestToRingingMs: ringing.ring_ms, notificationsRefused: denied, stoppedBy });
    }
    await panel.screenshot({ path: path.join(root, 'pc-find-stopped.png') });
    for (let attempt = 0; attempt < 400 && instrument.exitCode === null; attempt++) await delay(50);
    await fs.writeFile(path.join(root, 'instrumentation.log'), output);
    assert(output.includes('OK (1 test)'), output);
    Object.assign(report, { notificationsRefused: denied, phone: state.phone, realSession: true, artifacts: root });
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
    if (touched) {
      device('shell', 'pm', 'grant', 'io.github.prejas3.windconnect', 'android.permission.POST_NOTIFICATIONS');
      device('shell', 'cmd', 'audio', 'set-ringer-mode', 'NORMAL');
      if (alarmBefore !== null) device('shell', 'cmd', 'audio', 'set-volume', '4', String(alarmBefore));
      device('shell', 'input', 'keyevent', 'KEYCODE_WAKEUP');
    }
  }
})().catch(error => { console.error(error); console.error('artifacts:', root); process.exitCode = 1; });
