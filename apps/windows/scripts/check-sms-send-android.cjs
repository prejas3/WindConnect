// CCS/1 PURPOSE:WN-56 real PC app + Android emulator: sending from the "Messages" window. Off by default (the composer
// says so), "Sending from the computer" on (SmsProof send-on, SEND_SMS granted by the script): an SMS to the emulator's own
// number is sent (Android records it as a non-default app), shows in its thread and its delivery report as "delivered"; a picture
// goes as MMS (the emulator has no MMS centre: the outcome is recorded); off again disables the composer.
// LIMIT:a real MMS and delivery reports need a phone on a network (the S25 check). No message text is printed.
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const { spawn, spawnSync } = require('node:child_process');
const fs = require('node:fs/promises');
const path = require('node:path');
const os = require('node:os');
const crypto = require('node:crypto');
const dgram = require('node:dgram');
const assert = require('node:assert/strict');
const root = path.join(os.tmpdir(), 'windconnect-wn56-send-' + crypto.randomUUID());
const adb = path.join(process.env.ANDROID_HOME, 'platform-tools/adb.exe');
const serial = process.env.ANDROID_SERIAL || 'emulator-5590';
const port = 19328;
const OWN = process.env.OWN_NUMBER || '15555215590';
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
function device(...args) { const result = spawnSync(adb, ['-s', serial, ...args], { windowsHide: true, encoding: 'utf8', timeout: 20000 }); assert.equal(result.status, 0, result.stderr); return result.stdout; }
let app, instrument, browser, socket, timer, redirected = false, output = '';
(async () => {
  try {
    await fs.mkdir(root, { recursive: true });
    device('install', '-r', 'apps/android/app/build/outputs/apk/debug/app-debug.apk');
    device('install', '-r', '-t', 'apps/android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk');
    device('shell', 'pm', 'grant', 'io.github.prejas3.windconnect', 'android.permission.POST_NOTIFICATIONS');
    for (const p of ['READ_SMS', 'READ_CONTACTS', 'SEND_SMS', 'READ_PHONE_STATE']) device('shell', 'pm', 'grant', 'io.github.prejas3.windconnect', 'android.permission.' + p);
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
    await panel.locator('#open-sms').click();
    let sms;
    await until(async () => (sms = browser.contexts().flatMap(c => c.pages()).find(p => p.url().includes('sms.html'))), 'sms window');
    await sms.waitForLoadState();
    // Sending off (the default; set again in case an earlier run left it on): the composer says so and sends nothing.
    command('send-off');
    await sms.locator('#new-message').waitFor({ state: 'visible' });
    await sms.locator('#new-message').click();
    await until(() => sms.evaluate(() => document.querySelector('#composer-note').textContent.includes('Sending from the computer')), 'sending off explained');
    assert(await sms.locator('#send').isDisabled(), 'send disabled while off');
    command('send-on');
    await until(() => sms.evaluate(() => !document.querySelector('#send').disabled), 'sending on', 15000);
    const report = { sims: await sms.evaluate(() => [...document.querySelectorAll('#sim option')].length || 1) };
    // 1. SMS to the emulator's own number: sent, then back as a received message in the same thread.
    const marker = `WN56 ${Date.now()} zażółć`; // i18n: data
    await sms.locator('#to').fill(OWN);
    await sms.locator('#body').fill(marker);
    const clicked = Date.now();
    await sms.locator('#body').press('Enter');
    await until(() => sms.evaluate(() => [...document.querySelectorAll('.bubble.local .meta')].some(m => /sending/.test(m.textContent))), 'local bubble shown', 5000);
    const outgoing = await until(() => sms.evaluate(m => [...document.querySelectorAll('.bubble.out:not(.local)')].some(b => b.textContent.includes(m)), marker), 'sent message from the phone in its thread', 30000);
    report.clickToSentShownMs = Date.now() - clicked;
    assert(outgoing);
    // The delivery report (the emulator's modem sends one) reaches the provider and the window through the watch.
    await until(() => sms.evaluate(m => [...document.querySelectorAll('.bubble.out:not(.local)')].find(b => b.textContent.includes(m))?.querySelector('.meta')?.textContent.includes('delivered'), marker), 'delivered shown', 30000);
    report.clickToDeliveredShownMs = Date.now() - clicked;
    const stored = device('shell', 'content', 'query', '--uri', 'content://sms/sent', '--projection', '_id:status', '--where', `"body='${marker}'"`);
    assert(/Row:.*status=0/.test(stored), 'Android recorded the sent SMS as delivered (non-default app)');
    await sms.screenshot({ path: path.join(root, 'pc-sms-sent.png') });
    // 2. A picture goes as MMS; the emulator has no MMS centre, so the outcome is recorded, not asserted.
    const png = path.join(root, 'picture.png');
    await fs.writeFile(png, Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAFklEQVR4nGP8z8DAwMDAxMDAwMDAAAANHQEDasKb6QAAAABJRU5ErkJggg==', 'base64'));
    await sms.locator('#attach-file').setInputFiles(png);
    await sms.locator('#attachment').waitFor({ state: 'visible' });
    assert.equal(await sms.locator('#body').getAttribute('placeholder'), 'MMS message');
    await sms.locator('#body').fill('WN56 MMS');
    await sms.locator('#send').click();
    report.mms = await until(() => sms.evaluate(() => {
      const note = document.querySelector('#composer-note').textContent;
      const meta = [...document.querySelectorAll('.bubble.local .meta')].map(m => m.textContent).pop();
      return /wasn't sent|Not sent/.test(note) ? 'failed (no MMSC on the emulator)' : meta === 'sent' ? 'sent' : null;
    }) .then(v => v) , 'MMS outcome', 150000).catch(e => 'no outcome: ' + e.message);
    // 3. Sending off on the phone: the composer turns off again.
    command('send-off');
    await until(() => sms.evaluate(() => document.querySelector('#send').disabled), 'sending off again', 15000);
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
