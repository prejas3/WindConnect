// CCS/1 PURPOSE:WN-100 real PC app + Android emulator: remote actions both ways over a paired session. The PC's allowlist
// (actions.json in the test data dir) has lock, sleep and open-url off and one harmless custom action (whoami.exe): from the
// phone the list shows only it, lock and open-url are "refused", the custom one "done". From the panel's "Open on
// the phone": a link opens at once with the app in front, `file:` is refused before it leaves the PC, in the background the
// phone offers a notification, and with "Open links from the computer" off the phone refuses. Nothing locks or sleeps the PC.
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const { spawn, spawnSync } = require('node:child_process');
const fs = require('node:fs/promises');
const path = require('node:path');
const os = require('node:os');
const crypto = require('node:crypto');
const dgram = require('node:dgram');
const assert = require('node:assert/strict');
const root = path.join(os.tmpdir(), 'windconnect-wn100-actions-' + crypto.randomUUID());
const adb = path.join(process.env.ANDROID_HOME, 'platform-tools/adb.exe');
const serial = process.env.ANDROID_SERIAL || 'emulator-5590';
const port = 19330;

const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
function device(...args) { const result = spawnSync(adb, ['-s', serial, ...args], { windowsHide: true, encoding: 'utf8', timeout: 20000 }); assert.equal(result.status, 0, result.stderr); return result.stdout; }
let app, instrument, browser, socket, timer, redirected = false, output = '';
(async () => {
  try {
    await fs.mkdir(path.join(root, 'pc'), { recursive: true });
    // The PC's allowlist for this run: nothing that touches the session, one harmless custom action.
    await fs.writeFile(path.join(root, 'pc', 'actions.json'), JSON.stringify({ lock: false, sleep: false, openUrl: false, custom: [{ id: 'x-test', label: 'Test WN-100', enabled: true, program: path.join(process.env.SystemRoot || 'C:/Windows', 'System32', 'whoami.exe'), args: [] }] }));
    device('install', '-r', 'apps/android/app/build/outputs/apk/debug/app-debug.apk');
    device('install', '-r', '-t', 'apps/android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk');
    device('shell', 'pm', 'grant', 'io.github.prejas3.windconnect', 'android.permission.POST_NOTIFICATIONS');
    for (const p of ['READ_SMS', 'READ_CONTACTS', 'READ_PHONE_STATE', 'READ_CALL_LOG', 'ANSWER_PHONE_CALLS']) device('shell', 'pm', 'grant', 'io.github.prejas3.windconnect', 'android.permission.' + p);
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
    instrument = spawn(adb, ['-s', serial, 'shell', 'am', 'instrument', '-w', '-r', '-e', 'class', 'io.github.prejas3.windconnect.PairingTest#qrConnectsRealDesktop', '-e', 'desktopQr', 'true', '-e', 'smsProof', 'true', 'io.github.prejas3.windconnect.test/androidx.test.runner.AndroidJUnitRunner'], { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
    instrument.stdout.on('data', data => { output += data; }); instrument.stderr.on('data', data => { output += data; });
    const deadline = Date.now() + 20000;
    let state;
    do { state = await invoke('pairing_snapshot'); if (state.phase === 'connected') break; if (instrument.exitCode !== null) break; await delay(25); } while (Date.now() < deadline);
    assert.equal(state.phase, 'connected', output);
    const sh = cmd => device('shell', cmd);
    const command = word => sh(`echo ${word} | run-as io.github.prejas3.windconnect tee cache/wn55-cmd >/dev/null`);
    const until = async (what, label, ms = 10000) => {
      const start = Date.now();
      for (;;) {
        const value = await what();
        if (value) return value;
        assert(Date.now() - start < ms, label);
        assert.equal(instrument.exitCode, null, 'session held: ' + output);
        await delay(25);
      }
    };
    await until(async () => output.includes('smsProofReady') || null, 'phone proof ready', 20000);
    const ack = async line => {
      sh('run-as io.github.prejas3.windconnect rm -f cache/wn100-ack');
      command(line);
      return until(() => {
        const r = spawnSync(adb, ['-s', serial, 'shell', 'run-as', 'io.github.prejas3.windconnect', 'cat', 'cache/wn100-ack'], { windowsHide: true, encoding: 'utf8' });
        return r.status === 0 && r.stdout.startsWith(line + '=') ? { value: r.stdout.slice(line.length + 1).trim() } : null;
      }, `answer to ${line}`, 15000).then(a => a.value);
    };
    const report = {};
    // 1. Phone -> PC by the PC's allowlist (lock, sleep and open-url off here; one harmless custom action on).
    report.list = await ack('pc-list');
    assert.equal(report.list, 'x-test', 'the PC offers only what it allows');
    report.lock = await ack('pc lock');
    assert.equal(report.lock, 'refused', 'a turned-off action is refused');
    report.openUrlOnPc = await ack('pc open-url https://example.com/wn100');
    assert.equal(report.openUrlOnPc, 'refused');
    report.custom = await ack('pc x-test');
    assert.equal(report.custom, 'done', 'the PC ran its own allowed action');
    // 2. PC -> phone from the panel: in front, the link opens at once.
    await until(async () => (await invoke('pairing_snapshot')).open_on_phone || null, 'the phone takes links');
    await invoke('panel_action', { action: 'show' });
    const field = panel.locator('#phone-url');
    await field.waitFor({ state: 'visible' });
    const statusText = () => panel.locator('#open-on-phone-status').textContent();
    const url = 'https://example.com/?wn100=front';
    await field.fill(url);
    const sent = Date.now();
    await panel.locator('#open-on-phone button').click();
    await until(async () => (await statusText()) === 'Otwarto na telefonie' || null, 'opened (panel status)');
    report.panelToDoneMs = Date.now() - sent;
    const resumed = () => sh('dumpsys activity activities').split('\n').filter(l => /topResumedActivity|mResumedActivity/.test(l)).join(' ');
    report.frontActivity = await until(() => { const r = resumed(); return /chrome|browser|webview/i.test(r) ? r.trim().slice(0, 160) : null; }, 'a browser in front on the phone', 15000);
    // 3. Not a web address: refused before it leaves the PC.
    await field.fill('file:///etc/hosts');
    await panel.locator('#open-on-phone button').click();
    await until(async () => (await statusText()) === 'To nie jest adres http(s)' || null, 'bad argument');
    // 4. In the background: a notification to tap, not an activity.
    sh('input keyevent KEYCODE_HOME');
    await delay(1000);
    await field.fill('https://example.com/?wn100=background');
    await panel.locator('#open-on-phone button').click();
    await until(async () => (await statusText()) === 'Otwarto na telefonie' || null, 'offered in the background');
    report.backgroundNotification = await until(() => /links_from_pc/.test(sh('dumpsys notification --noredact')) || null, 'link notification on the phone', 10000);
    // 5. "Open links from the computer" off: refused.
    command('links-off');
    await delay(800);
    await field.fill('https://example.com/?wn100=off');
    await panel.locator('#open-on-phone button').click();
    await until(async () => (await statusText()).startsWith('Telefon odrzucił') || null, 'refused with links off');
    report.linksOffRefused = true;
    await panel.screenshot({ path: path.join(root, 'pc-panel-open-on-phone.png') });
    command('done');
    for (let attempt = 0; attempt < 200 && instrument.exitCode === null; attempt++) await delay(50);
    await fs.writeFile(path.join(root, 'instrumentation.log'), output);
    assert(output.includes('OK (1 test)'), output);
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
  }
})().catch(error => { console.error(error); console.error('artifacts:', root); process.exitCode = 1; });
