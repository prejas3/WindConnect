// CCS/1 PURPOSE:WN-45 real PC app (debug) + Android emulator: the app filter, Do not disturb on the phone and quiet hours on the PC
// (PROTOCOL.md §9.7 field 8). Pairs by QR as check-notify-reply-android does; PairingTest#notifyFilterProof holds the session and
// runs filter commands from the script (cache/wn45-cmd). Senders: the shell (`cmd notification post`, another app) and Google
// Messages (`adb emu sms send`). Checks:
//   (a) the shell blocked: 10 of its notifications hand nothing to the core (no NotifySend: nothing over the network), an SMS
//       gets through; blocking removed the shell's notification already shown on the PC (it stays on the phone);
//   (b) "Chosen" with nothing chosen: nothing at all;
//   (c) blocking an app removes exactly its notifications from this session on the PC (panel and Windows), others stay;
//   (d) DND on the phone -> no popup on the PC, the notification stays on the phone; DND off -> popup again;
//   (e) quiet hours on the PC: inside -> no popup (silent), outside -> popup; the silent one stays on the phone.
// The queued-before-blocking case (the send point's re-check) is a unit test (NotifyAppsTest). Restores DND, the quiet hours, the
// listener access and the assistant; removes the phone toasts it made.
// The reply texts are test strings; nothing logs them. Restores the listener access and removes the phone toasts it made.
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const { spawn, spawnSync } = require('node:child_process');
const fs = require('node:fs/promises');
const path = require('node:path');
const os = require('node:os');
const crypto = require('node:crypto');
const dgram = require('node:dgram');
const assert = require('node:assert/strict');
const root = path.join(os.tmpdir(), 'windconnect-wn45-filter-' + crypto.randomUUID());
const adb = path.join(process.env.ANDROID_HOME, 'platform-tools/adb.exe');
const serial = process.env.ANDROID_SERIAL || 'emulator-5590';
const port = 19329;
const runner = 'io.github.prejas3.windconnect.test/androidx.test.runner.AndroidJUnitRunner';
const LISTENER = 'io.github.prejas3.windconnect/io.github.prejas3.windconnect.core.NotifyListenerService';
const MESSAGES = 'com.google.android.apps.messaging';
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
function device(...args) { const result = spawnSync(adb, ['-s', serial, ...args], { windowsHide: true, encoding: 'utf8', timeout: 60000 }); assert.equal(result.status, 0, result.stderr + result.stdout); return result.stdout; }
async function until(what, check, ms = 20000) {
  const end = Date.now() + ms;
  for (;;) { const value = await check(); if (value) return value; if (Date.now() > end) throw Error(`timed out: ${what}`); await delay(150); }
}
// Messages' notifications on the phone (keys), from dumpsys.
function messagesKeys() {
  return [...device('shell', 'dumpsys', 'notification').matchAll(/NotificationRecord\(0x[0-9a-f]+: pkg=([^ ]+) user=\S+ id=(-?\d+) tag=(\S+)/g)]
    .filter(m => m[1] === MESSAGES).map(m => `${m[1]}|${m[2]}|${m[3]}`);
}
// Whether a Messages notification with this text is on the phone (the test SMS texts only).
function phoneHas(text) {
  return device('shell', 'dumpsys', 'notification', '--noredact').includes(`android.text=String (${text})`);
}
// The key of the Messages notification with this text (not its group summary: snoozing that holds back later ones too).
function keyWithText(text) {
  const block = device('shell', 'dumpsys', 'notification', '--noredact').split('NotificationRecord(').find(b => b.includes(`pkg=${MESSAGES} `) && b.includes(`android.text=String (${text})`));
  return block && block.match(/key=(.+?): Notification\(/)[1];
}
function sentSms() {
  return device('shell', 'content', 'query', '--uri', 'content://sms/sent', '--projection', 'address:body').split(/\r?\n/).filter(l => l.startsWith('Row:'));
}

let app, browser, socket, timer, redirected = false, granted = false, invoke = null, instrument = null, assistant = null, dnd = false, quietSet = null;
(async () => {
  try {
    await fs.mkdir(root, { recursive: true });
    device('install', '-r', 'apps/android/app/build/outputs/apk/debug/app-debug.apk');
    device('install', '-r', '-t', 'apps/android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk');
    assert(device('shell', 'pm', 'list', 'packages', MESSAGES).includes(MESSAGES), 'Google Messages on the emulator');
    device('shell', 'cmd', 'role', 'add-role-holder', 'android.app.role.SMS', MESSAGES, '0'); // it sends the replies
    assert(device('shell', 'cmd', 'role', 'get-role-holders', 'android.app.role.SMS').includes(MESSAGES), 'Messages is the SMS app');
    device('shell', 'pm', 'grant', MESSAGES, 'android.permission.POST_NOTIFICATIONS');
    device('shell', 'cmd', 'notification', 'allow_listener', LISTENER); granted = true;
    // Android 15's assistant hides SMS texts from listeners as sensitive; off for the test so rows can be told apart, restored after.
    assistant = device('shell', 'cmd', 'notification', 'get_approved_assistant').trim();
    if (!assistant.includes('/')) assistant = null; else device('shell', 'cmd', 'notification', 'disallow_assistant', assistant);
    device('emu', 'redir', 'add', 'udp:47823:47821'); redirected = true;
    app = spawn(path.resolve('target/debug/windconnect-windows.exe'), [], { windowsHide: true, stdio: 'ignore', env: { ...process.env, WINDCONNECT_TEST_DATA_DIR: path.join(root, 'pc'), WINDCONNECT_TEST_PAIR_IP: '10.0.2.2', WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`, WEBVIEW2_USER_DATA_FOLDER: path.join(root, 'webview') } });
    for (let index = 0; index < 100; index++) { try { browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`, { timeout: 1000 }); break; } catch {} if (app.exitCode !== null) throw Error('PC app exited'); await delay(100); }
    assert(browser);
    let panel;
    for (let index = 0; index < 100; index++) { panel = browser.contexts().flatMap(c => c.pages()).find(p => p.url().includes('panel.html')); if (panel) break; await delay(100); }
    assert(panel);
    invoke = (command, args) => panel.evaluate(({ command, args }) => window.__TAURI__.core.invoke(command, args), { command, args });
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
    device('shell', 'run-as', 'io.github.prejas3.windconnect', 'rm', '-f', 'cache/wn45-cmd', 'cache/wn45-ack');
    device('shell', 'am', 'force-stop', 'com.android.shell'); // at most 50 notifications per app: clear earlier runs'

    // The phone: pair, notifications on, run the filter commands until "done".
    let output = '';
    const finished = new Promise((resolve, reject) => {
      instrument = spawn(adb, ['-s', serial, 'shell', 'am', 'instrument', '-w', '-r', '-e', 'class', 'io.github.prejas3.windconnect.PairingTest#qrConnectsRealDesktop',
        '-e', 'desktopQr', 'true', '-e', 'notifyFilterProof', 'true', runner], { windowsHide: true });
      const killer = setTimeout(() => instrument.kill(), 400000);
      instrument.stdout.on('data', d => { output += d; }); instrument.stderr.on('data', d => { output += d; });
      instrument.on('error', reject);
      instrument.on('close', () => { clearTimeout(killer); resolve(output); });
    });
    await until('the phone holds the session', () => output.includes('notifyFilterReady=true') || (instrument.exitCode !== null && Promise.reject(Error(output))), 120000);
    clearInterval(timer); timer = null;
    const report = { artifacts: root };
    const state = () => invoke('pairing_snapshot');
    const run = Array.from(crypto.randomBytes(4), b => String.fromCharCode(97 + b % 26)).join('');
    const SHELL = 'com.android.shell';
    const prefix = String(100 + crypto.randomInt(900));
    const sms = (from, text) => device('emu', 'sms', 'send', `${prefix}${from}`, text);
    // A shell notification (another app, as a real one would): tag wn45<name>, text <name>_<run>.
    const post = name => device('shell', `cmd notification post -t T_${run} wn45${name} ${name}_${run}`);
    const rowWith = async text => (await state()).notifications.find(n => n.text === text);
    const onPhone = name => device('shell', 'cmd', 'notification', 'list').includes(`|wn45${name}|`);
    let commands = 0;
    // A filter command on the phone; its answer lists every NotifySend so far ("package:outcome").
    const command = async line => {
      device('shell', `run-as io.github.prejas3.windconnect sh -c 'echo ${line} > cache/wn45-cmd'`);
      commands++;
      const ack = await until(`the phone ran "${line}"`, () => {
        try { const a = device('shell', 'run-as', 'io.github.prejas3.windconnect', 'cat', 'cache/wn45-ack').trim(); return a.startsWith(`${commands} `) && a; } catch { return false; }
      });
      return ack.slice(ack.indexOf(' ') + 1).split(',').filter(Boolean);
    };
    const sendsOf = (sends, pkg) => sends.filter(s => s.startsWith(`${pkg}:`));
    const logOf = async tag => (await invoke('dev_notify_log')).filter(e => e.tag === tag);

    // Baseline: the shell gets through.
    post('base');
    const base = await until('the baseline in the panel', () => rowWith(`base_${run}`), 20000);

    // (a) Blocked: 10 from the shell never reach the core (no NotifySend: nothing over the network); an SMS from Messages does.
    let sends = await command(`block ${SHELL}`);
    const shellBefore = sendsOf(sends, SHELL).length;
    for (let i = 1; i <= 10; i++) post(`a${i}`);
    sms('1001', `Przechodzi ${run}`);
    await until('the SMS gets through', () => rowWith(`Przechodzi ${run}`), 30000);
    await delay(1500);
    sends = await command('noop');
    assert.equal(sendsOf(sends, SHELL).length, shellBefore, 'a blocked app hands nothing to the core');
    for (let i = 1; i <= 10; i++) assert(!(await rowWith(`a${i}_${run}`)), `a${i} not on the PC`);
    // Blocking removed the baseline from the PC (sent in this session); it stays on the phone.
    await until('the baseline gone from the PC', async () => !(await state()).notifications.some(n => n.tag === base.tag));
    await until('its toast gone', async () => !(await invoke('dev_notify_history')).includes(base.tag));
    assert(onPhone('base'), 'blocking removes it from the PC only');
    report.blocked = { shellPosts: 10, shellSends: 0, smsThrough: true, removedOnBlock: 1 };

    // (b) "Chosen" with nothing chosen: nothing at all.
    const all = sends.length;
    const through = await rowWith(`Przechodzi ${run}`);
    await command('mode only');
    // Messages isn't chosen either: its SMS from (a) goes away on the PC.
    await until('the SMS from (a) gone from the PC', async () => !(await state()).notifications.some(n => n.tag === through.tag));
    post('b1');
    sms('1002', `Nic ${run}`);
    await delay(4000);
    sends = await command('noop');
    assert.equal(sends.length, all, 'nothing handed to the core');
    assert(!(await rowWith(`b1_${run}`)) && !(await rowWith(`Nic ${run}`)), 'nothing on the PC');
    report.onlyChosenEmpty = { sends: 0 };

    // (c) Back to all, the shell on; three sent; blocking removes exactly those from the PC.
    await command('mode all');
    await command(`allow ${SHELL}`);
    const tags = [];
    for (const name of ['c1', 'c2', 'c3']) { post(name); tags.push((await until(`${name} on the PC`, () => rowWith(`${name}_${run}`), 20000)).tag); }
    await until('their toasts in Windows', async () => { const h = await invoke('dev_notify_history'); return tags.every(t => h.includes(t)); });
    sms('1003', `Zostaje ${run}`);
    const other = await until('another app on the PC', () => rowWith(`Zostaje ${run}`), 30000);
    await command(`block ${SHELL}`);
    await until('the three gone from the panel', async () => { const s = await state(); return tags.every(t => !s.notifications.some(n => n.tag === t)); });
    await until('and from Windows', async () => { const h = await invoke('dev_notify_history'); return tags.every(t => !h.includes(t)); });
    assert((await state()).notifications.some(n => n.tag === other.tag), 'another app stays');
    assert(['c1', 'c2', 'c3'].every(onPhone), 'still on the phone');
    report.removedOnBlock = tags.length;
    await command(`allow ${SHELL}`);

    // (d) Do not disturb on the phone: quiet on the PC (no popup, silent), and it stays on the phone.
    device('shell', 'cmd', 'notification', 'set_dnd', 'priority'); dnd = true;
    await delay(500);
    post('d1');
    const d1 = await until('d1 on the PC', () => rowWith(`d1_${run}`), 20000);
    const d1log = await until('d1 shown', async () => (await logOf(d1.tag))[0]);
    assert.equal(d1log.popup, false, 'quiet on the phone -> no popup');
    device('shell', 'cmd', 'notification', 'set_dnd', 'off'); dnd = false;
    await delay(2000);
    assert(onPhone('d1'), 'a silent toast never dismisses it on the phone');
    post('d2');
    const d2 = await until('d2 on the PC', () => rowWith(`d2_${run}`), 20000);
    assert.equal((await until('d2 shown', async () => (await logOf(d2.tag))[0])).popup, true, 'DND off -> popup again');
    report.phoneDnd = { quiet: true, staysOnPhone: true };

    // (e) Quiet hours on the PC (the main window's command): inside -> silent; outside -> popup.
    await invoke('panel_action', { action: 'settings' }); // opens the main window
    const main = await until('the main window page', () => browser.contexts().flatMap(c => c.pages()).find(p => /^https?:/.test(p.url()) && !/panel\.html|tray-menu/.test(p.url())));
    await main.waitForFunction(() => !!window.__TAURI__);
    const mainInvoke = (command, args) => main.evaluate(({ command, args }) => window.__TAURI__.core.invoke(command, args), { command, args });
    const now = new Date();
    const floor = Math.floor((now.getHours() * 60 + now.getMinutes()) / 15) * 15;
    await mainInvoke('notify_quiet_hours', { on: true, from: floor, to: (floor + 60) % 1440 }); quietSet = mainInvoke;
    await until('quiet hours set', async () => (await state()).quiet_hours.on);
    post('e1');
    const e1 = await until('e1 on the PC', () => rowWith(`e1_${run}`), 20000);
    assert.equal((await until('e1 shown', async () => (await logOf(e1.tag))[0])).popup, false, 'inside the quiet hours -> no popup');
    await mainInvoke('notify_quiet_hours', { on: true, from: (floor + 120) % 1440, to: (floor + 180) % 1440 });
    await until('quiet hours moved', async () => (await state()).quiet_hours.from === (floor + 120) % 1440);
    post('e2');
    const e2 = await until('e2 on the PC', () => rowWith(`e2_${run}`), 20000);
    assert.equal((await until('e2 shown', async () => (await logOf(e2.tag))[0])).popup, true, 'outside -> popup');
    await delay(1500);
    assert(onPhone('e1'), 'the silent one stays on the phone');
    report.quietHours = { inside: 'silent', outside: 'popup' };

    await command('done').catch(() => {});
    const done = await finished;
    await fs.writeFile(path.join(root, 'instrumentation.log'), done);
    assert(done.includes('OK (1 test)'), done);
    await fs.writeFile(path.join(root, 'run.json'), JSON.stringify(report, null, 2)); await fs.rm(offerFile);
    console.log(JSON.stringify(report, null, 2));
  } catch (error) {
    // Diagnostics (titles/texts of test SMS only).
    if (invoke) await fs.writeFile(path.join(root, 'failure.json'), JSON.stringify({ state: await invoke('pairing_snapshot').catch(e => String(e)), log: await invoke('dev_notify_log').catch(e => String(e)), phone: (() => { try { return messagesKeys(); } catch (e) { return String(e); } })() }, null, 2)).catch(() => {});
    try { await fs.writeFile(path.join(root, 'logcat.txt'), spawnSync(adb, ['-s', serial, 'logcat', '-d', '-t', '400'], { encoding: 'utf8', windowsHide: true }).stdout); } catch {}
    throw error;
  } finally {
    try { device('shell', `run-as io.github.prejas3.windconnect sh -c 'echo done > cache/wn45-cmd'`); } catch {}
    if (dnd) try { device('shell', 'cmd', 'notification', 'set_dnd', 'off'); } catch {}
    if (quietSet) await quietSet('notify_quiet_hours', { on: false, from: 1320, to: 420 }).catch(() => {});
    if (invoke) await invoke('phone_notifications', { enabled: false }).catch(() => {}); // removes the phone toasts it made
    await delay(300);
    if (invoke) await invoke('tray_action', { action: 'quit' }).catch(() => {});
    if (timer) clearInterval(timer); if (socket) socket.close();
    if (browser) await browser.close();
    if (app && app.exitCode === null) { await delay(500); if (app.exitCode === null) app.kill(); }
    if (instrument && instrument.exitCode === null) { await delay(2000); if (instrument.exitCode === null) instrument.kill(); }
    if (redirected) device('emu', 'redir', 'del', 'udp:47823');
    if (assistant) device('shell', 'cmd', 'notification', 'allow_assistant', assistant);
    if (granted) device('shell', 'cmd', 'notification', 'disallow_listener', LISTENER);
  }
})().catch(error => { console.error(error); console.error('artifacts:', root); process.exitCode = 1; });
