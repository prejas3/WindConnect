// CCS/1 PURPOSE:WN-55 real PC app + Android emulator: 1000 SMS seeded in one thread (adb emu sms send), paired, "SMS on
// the computer" on (SmsProof). The panel's "Messages" opens the window; opening the 1000-message thread is timed from click
// to painted (window.__smsOpenMs): cold (from the phone) and warm (memory + incremental `since`), budget < 300 ms. Then a
// new SMS reaches the open thread through the watch, and the phone's setting off/on is shown in the window.
// LIMIT:emulator NAT needs a UDP redirection for the public discovery hint; no trust or TLS bypass. Seeding is skipped for
// messages already there (by count). No message text is printed.
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const { spawn, spawnSync } = require('node:child_process');
const fs = require('node:fs/promises');
const path = require('node:path');
const os = require('node:os');
const crypto = require('node:crypto');
const dgram = require('node:dgram');
const assert = require('node:assert/strict');
const root = path.join(os.tmpdir(), 'windconnect-wn55-sms-' + crypto.randomUUID());
const adb = path.join(process.env.ANDROID_HOME, 'platform-tools/adb.exe');
const serial = process.env.ANDROID_SERIAL || 'emulator-5590';
const port = 19327;
const SENDER = '+48600100200', OTHER = '+48600100300';
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
function device(...args) { const result = spawnSync(adb, ['-s', serial, ...args], { windowsHide: true, encoding: 'utf8', timeout: 20000 }); assert.equal(result.status, 0, result.stderr); return result.stdout; }
let app, instrument, browser, socket, timer, redirected = false, output = '';
(async () => {
  try {
    await fs.mkdir(root, { recursive: true });
    device('install', '-r', 'apps/android/app/build/outputs/apk/debug/app-debug.apk');
    device('install', '-r', '-t', 'apps/android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk');
    device('shell', 'pm', 'grant', 'io.github.prejas3.windconnect', 'android.permission.POST_NOTIFICATIONS');
    for (const p of ['READ_SMS', 'READ_CONTACTS']) device('shell', 'pm', 'grant', 'io.github.prejas3.windconnect', 'android.permission.' + p);
    const seeded = seed();
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
    await until(async () => (await invoke('pairing_snapshot')).sms, 'phone announces sms/1', 15000);
    const opened = Date.now();
    await panel.locator('#open-sms').click();
    let sms;
    await until(async () => (sms = browser.contexts().flatMap(c => c.pages()).find(p => p.url().includes('sms.html'))), 'sms window');
    await sms.waitForLoadState();
    const threadOf = number => sms.evaluate(number => [...document.querySelectorAll('.thread')].find(b => b.querySelector('.thread-title')?.textContent.replace(/\D/g, '').endsWith(number.slice(-9)))?.dataset.id, number);
    const big = await until(() => threadOf(SENDER), 'the 1000-message thread listed', 20000);
    const threadsMs = Date.now() - opened;
    const small = await until(() => threadOf(OTHER), 'a second thread');
    const open = async id => {
      await sms.evaluate(() => { window.__smsOpenMs = null; });
      await sms.locator(`.thread[data-id="${id}"]`).click();
      return until(() => sms.evaluate(id => window.__smsOpenMs?.id === Number(id) && window.__smsOpenMs, id), `open ${id}`);
    };
    const cold = await open(big);
    assert(cold.count >= 1000, `cold open showed ${cold.count} messages`);
    await sms.screenshot({ path: path.join(root, 'pc-sms-thread.png') });
    const warm = [];
    for (let i = 0; i < 6; i++) { await open(small); warm.push((await open(big)).ms); }
    warm.sort((a, b) => a - b);
    const warmP95 = warm[Math.ceil(warm.length * .95) - 1];
    // Incremental: a new SMS reaches the open thread (watch -> sms_thread with since).
    const count = () => sms.evaluate(() => document.querySelectorAll('.bubble').length);
    const before = await count();
    const sent = Date.now();
    device('emu', 'sms', 'send', SENDER, 'WN55 nowa wiadomosc');
    await until(async () => (await count()) === before + 1, 'new SMS shown in the open thread', 15000);
    const incomingMs = Date.now() - sent;
    // The phone's setting: off -> the window says so, on -> available again.
    command('off');
    await until(() => sms.evaluate(() => document.querySelector('#threads-status').textContent.includes("doesn't share")), 'setting off shown');
    command('on');
    await until(() => sms.evaluate(() => document.querySelector('#threads-status').textContent === ''), 'setting on again');
    await sms.screenshot({ path: path.join(root, 'pc-sms-window.png') });
    command('done');
    for (let attempt = 0; attempt < 200 && instrument.exitCode === null; attempt++) await delay(50);
    await fs.writeFile(path.join(root, 'instrumentation.log'), output);
    assert(output.includes('OK (1 test)'), output);
    const report = { seeded, threadsListedMs: threadsMs, coldOpen: cold, warmOpenMs: warm, warmP95, incomingToShownMs: incomingMs, phone: state.phone, realSession: true, artifacts: root };
    await fs.writeFile(path.join(root, 'run.json'), JSON.stringify(report, null, 2)); await fs.rm(offerFile);
    console.log(JSON.stringify(report));
    assert(cold.ms < 300, `cold open of ${cold.count} messages took ${cold.ms} ms`);
    assert(warmP95 < 300, `warm open p95 ${warmP95} ms`);
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

// 1000 messages from SENDER (one thread, a minute apart, every tenth sent) and 3 from OTHER, inserted into the provider by the
// shell (WRITE_SMS app-op; the emulator's modem delivers only ~1 SMS/s); only what is missing. The live SMS later goes
// through the modem.
function seed() {
  const have = number => {
    const out = spawnSync(adb, ['-s', serial, 'shell', 'content', 'query', '--uri', 'content://sms', '--projection', '_id', '--where', `"address='${number}'"`], { windowsHide: true, encoding: 'utf8', timeout: 60000 });
    return (out.stdout.match(/^Row:/gm) || []).length;
  };
  device('shell', 'appops', 'set', 'com.android.shell', 'WRITE_SMS', 'allow');
  const sent = {};
  const now = Date.now();
  for (const [number, want] of [[OTHER, 3], [SENDER, 1000]]) {
    const missing = want - have(number);
    if (missing > 0) {
      const loop = `i=0; while [ $i -lt ${missing} ]; do if [ $((i % 10)) -eq 9 ]; then box=sent; else box=inbox; fi; ` +
        `content insert --uri content://sms/$box --bind address:s:${number} --bind body:s:"WN55 $i test" --bind date:l:$((${now - want * 60000} + i * 60000)) --bind read:i:1; i=$((i+1)); done`;
      const out = spawnSync(adb, ['-s', serial, 'shell', loop], { windowsHide: true, encoding: 'utf8', timeout: 3600000 });
      assert.equal(out.status, 0, out.stderr);
    }
    assert(have(number) >= want, `seeded ${number}`);
    sent[number === SENDER ? 'thread' : 'other'] = Math.max(0, missing);
  }
  return sent;
}
