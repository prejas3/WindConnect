// CCS/1 PURPOSE:WN-57 real PC app + Android emulator: calls on the PC. "Calls on the computer" on (SmsProof calls-on, the
// phone and call-log permissions granted by the script); `adb emu gsm call` rings the phone: the PC's toast with the caller is
// timed from the call (same host clock), "Silence" mutes the ring stream, "Reject" ends the call, an old call's button leaves a
// new call alone, an unanswered call shows as a missed-call toast and in the panel, and with the setting off there is no toast.
// LIMIT:the buttons are pressed through dev_toast_activate (as WinRT delivers them); a real S25 check covers the toast UI.
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const { spawn, spawnSync } = require('node:child_process');
const fs = require('node:fs/promises');
const path = require('node:path');
const os = require('node:os');
const crypto = require('node:crypto');
const dgram = require('node:dgram');
const assert = require('node:assert/strict');
const root = path.join(os.tmpdir(), 'windconnect-wn57-calls-' + crypto.randomUUID());
const adb = path.join(process.env.ANDROID_HOME, 'platform-tools/adb.exe');
const serial = process.env.ANDROID_SERIAL || 'emulator-5590';
const port = 19329;
const CALLER = '600100200', OTHER = '600100300';
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
function device(...args) { const result = spawnSync(adb, ['-s', serial, ...args], { windowsHide: true, encoding: 'utf8', timeout: 20000 }); assert.equal(result.status, 0, result.stderr); return result.stdout; }
let app, instrument, browser, socket, timer, redirected = false, output = '';
(async () => {
  try {
    await fs.mkdir(root, { recursive: true });
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
    // SmsProof clears the command file when it starts: wait for it first.
    await until(async () => output.includes('smsProofReady') || null, 'phone proof ready', 20000);
    command('calls-on');
    await delay(1500);
    const sh2 = cmd => device('shell', cmd);
    const callState = () => Number((sh2('dumpsys telephony.registry').match(/mCallState=(\d)/) || [])[1]);
    const ringMuted = () => /Muted: true/.test((sh2('dumpsys audio').split('- STREAM_RING:')[1] || '').split('- STREAM_')[0]);
    const log = () => invoke('dev_notify_log');
    const toast = async (prefix, after, label, ms = 10000) => until(async () => (await log()).find(e => e.tag.startsWith(prefix) && e.shown_ms >= after && e.popup), label, ms);
    const report = {};
    // 1. An incoming call: the toast is up before the second ring (the emulator rings every ~4-6 s; budget 3 s).
    const t1 = Date.now();
    device('emu', 'gsm', 'call', CALLER);
    const first = await toast('call/call-', t1, 'incoming-call toast');
    report.callToToastMs = first.shown_ms - t1;
    const id = first.tag.split('call-').pop();
    // 2. "Silence" from the toast: the phone's ring stream is muted, the call goes on.
    await invoke('dev_toast_activate', { tag: first.tag.split('/')[1], arguments: `call-silence:${id}`, input: null });
    report.silenced = await until(async () => ringMuted() || null, 'ring muted on the phone', 5000).catch(() => false);
    assert.equal(callState(), 1, 'still ringing after silence');
    // 3. "Reject": the call ends on the phone.
    const rejectAt = Date.now();
    await invoke('dev_toast_activate', { tag: first.tag.split('/')[1], arguments: `call-reject:${id}`, input: null });
    await until(async () => callState() === 0 || null, 'call rejected on the phone', 8000);
    report.rejectToIdleMs = Date.now() - rejectAt;
    report.unmutedAfter = await until(async () => !ringMuted() || null, 'ring unmuted after the call', 5000).then(() => true).catch(() => false);
    // 4. A stale button (the call that already ended) does nothing to a new call.
    await delay(1500);
    const t2 = Date.now();
    device('emu', 'gsm', 'call', OTHER);
    const second = await toast('call/call-', t2, 'second call toast');
    assert.notEqual(second.tag, first.tag, 'a new call id');
    await invoke('dev_toast_activate', { tag: first.tag.split('/')[1], arguments: `call-reject:${id}`, input: null });
    await delay(1500);
    assert.equal(callState(), 1, 'an old call id leaves the new call ringing');
    // 5. Nobody answers: a missed call (toast + panel list).
    device('emu', 'gsm', 'cancel', OTHER);
    const missedAt = Date.now();
    await toast('call/missed-', missedAt - 500, 'missed-call toast', 15000);
    report.hangupToMissedToastMs = (await log()).find(e => e.tag.startsWith('call/missed-') && e.shown_ms >= missedAt - 500).shown_ms - missedAt;
    const missed = await until(async () => { const m = (await invoke('pairing_snapshot')).missed_calls; return m.length ? m : null; }, 'missed calls in the panel');
    assert(missed[0].who.replace(/\D/g, '').endsWith(OTHER.slice(-6)), 'newest missed call first');
    await invoke('panel_action', { action: 'show' });
    await panel.locator('#missed-calls').waitFor({ state: 'visible' });
    await panel.locator('#missed-calls').scrollIntoViewIfNeeded();
    await panel.locator('#missed-calls').screenshot({ path: path.join(root, 'pc-panel-missed.png') });
    // 6. WN-57 review: silenced, then the setting goes off mid-ring: our mute is undone at once (not left for good).
    await delay(1500);
    const t4 = Date.now();
    device('emu', 'gsm', 'call', CALLER);
    const fourth = await toast('call/call-', t4, 'fourth call toast');
    await invoke('dev_toast_activate', { tag: fourth.tag.split('/')[1], arguments: `call-silence:${fourth.tag.split('call-').pop()}`, input: null });
    await until(async () => ringMuted() || null, 'ring muted before the setting goes off', 5000);
    command('calls-off');
    report.unmutedWhenTurnedOff = await until(async () => !ringMuted() || null, 'ring unmuted when the setting goes off mid-ring', 8000).then(() => true);
    device('emu', 'gsm', 'cancel', CALLER);
    await until(async () => callState() === 0 || null, 'fourth call over');
    // 7. "Calls on the computer" off: a call is only a state (no toast).
    await delay(1500);
    const t3 = Date.now();
    device('emu', 'gsm', 'call', CALLER);
    await until(async () => callState() === 1 || null, 'third call rings');
    await delay(2500);
    assert(!(await log()).some(e => e.tag.startsWith('call/call-') && e.shown_ms >= t3), 'no toast with the setting off');
    device('emu', 'gsm', 'cancel', CALLER);
    command('done');
    for (let attempt = 0; attempt < 200 && instrument.exitCode === null; attempt++) await delay(50);
    await fs.writeFile(path.join(root, 'instrumentation.log'), output);
    assert(output.includes('OK (1 test)'), output);
    Object.assign(report, { phone: state.phone, realSession: true, artifacts: root });
    await fs.writeFile(path.join(root, 'run.json'), JSON.stringify(report, null, 2)); await fs.rm(offerFile);
    console.log(JSON.stringify(report));
    assert(report.callToToastMs < 3000, `toast ${report.callToToastMs} ms after the call`);
    await invoke('tray_action', { action: 'quit' }).catch(() => {});
  } finally {
    await fs.writeFile(path.join(root, 'instrumentation.log'), output).catch(() => {});
    if (timer) clearInterval(timer); if (socket) socket.close();
    try { device('emu', 'gsm', 'cancel', CALLER); device('emu', 'gsm', 'cancel', OTHER); } catch {}
    if (instrument && instrument.exitCode === null) { device('shell', 'am', 'force-stop', 'io.github.prejas3.windconnect'); instrument.kill(); }
    if (browser) await browser.close();
    if (app && app.exitCode === null) app.kill();
    if (redirected) device('emu', 'redir', 'del', 'udp:47823');
  }
})().catch(error => { console.error(error); console.error('artifacts:', root); process.exitCode = 1; });
