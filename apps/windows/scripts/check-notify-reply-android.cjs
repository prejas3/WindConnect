// CCS/1 PURPOSE:WN-44 real PC app (debug) + Android emulator + a real app with a RemoteInput (Google Messages, fed by
// `adb emu sms send`): quick reply and dismissal sync both ways (PROTOCOL.md §9.7–9.8). Pairs by QR as check-notify-toasts-android
// does, then PairingTest#notifyReplyProof holds the session with notifications on. Checks:
//   - an incoming SMS -> a toast with a reply field (dev_notify_log.reply) and a row with a field in the panel;
//   (a) a reply typed in the panel (Playwright, Enter) -> the SMS goes out (content://sms/sent) and the panel says "Sent";
//   (b) a reply from the toast: dev_toast_activate calls the same function as the WinRT handler -> SMS sent;
//   (c) ✕ in the panel -> the Messages notification is gone from the phone (dumpsys notification);
//   (d) dismissed on the phone (snooze) -> the toast and the row are gone on the PC;
//   (e) closing a popup (dev_toast_dismissed reason 0) -> gone on the phone; reason 2 (timed out) -> stays.
// Reports PC -> phone reply latency (invoke -> "Sent" in the panel: the PendingIntent sent and Replied back).
// LIMIT:the real WinRT toast click/close path is not driven here (system UI): the short manual test on a real phone covers it.
// The reply texts are test strings; nothing logs them. Restores the listener access and removes the phone toasts it made.
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const { spawn, spawnSync } = require('node:child_process');
const fs = require('node:fs/promises');
const path = require('node:path');
const os = require('node:os');
const crypto = require('node:crypto');
const dgram = require('node:dgram');
const assert = require('node:assert/strict');
const root = path.join(os.tmpdir(), 'windconnect-wn44-reply-' + crypto.randomUUID());
const adb = path.join(process.env.ANDROID_HOME, 'platform-tools/adb.exe');
const serial = process.env.ANDROID_SERIAL || 'emulator-5590';
const port = 19328;
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

let app, browser, socket, timer, redirected = false, granted = false, invoke = null, instrument = null, assistant = null;
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
    device('shell', 'run-as', 'io.github.prejas3.windconnect', 'rm', '-f', 'cache/wn44-done');

    // The phone: pair, notifications on, hold the session until cache/wn44-done.
    let output = '';
    const finished = new Promise((resolve, reject) => {
      instrument = spawn(adb, ['-s', serial, 'shell', 'am', 'instrument', '-w', '-r', '-e', 'class', 'io.github.prejas3.windconnect.PairingTest#qrConnectsRealDesktop',
        '-e', 'desktopQr', 'true', '-e', 'notifyReplyProof', 'true', runner], { windowsHide: true });
      const killer = setTimeout(() => instrument.kill(), 400000);
      instrument.stdout.on('data', d => { output += d; }); instrument.stderr.on('data', d => { output += d; });
      instrument.on('error', reject);
      instrument.on('close', () => { clearTimeout(killer); resolve(output); });
    });
    await until('the phone holds the session', () => output.includes('notifyReplyReady=true') || (instrument.exitCode !== null && Promise.reject(Error(output))), 120000);
    clearInterval(timer); timer = null;
    const report = { artifacts: root };
    const state = () => invoke('pairing_snapshot');
    // The panel row of a notification, by its tag (rows are newest first, as in the state).
    const rowOf = async tag => panel.locator('#notification-list li').nth((await state()).notifications.findIndex(n => n.tag === tag));
    // Each run's texts are unique (Messages keeps conversations, and re-posts them, across runs): rows match exactly.
    const run = Array.from(crypto.randomBytes(4), b => String.fromCharCode(97 + b % 26)).join('');
    const T = text => `${text} ${run}`;
    const rowFor = async text => (await state()).notifications.find(n => n.text === text);

    // An incoming SMS: a toast with a reply field and a row in the panel.
    // A new conversation per step and run: earlier runs' conversations on the phone don't mix in.
    const prefix = String(100 + crypto.randomInt(900));
    const sms = (from, text) => device('emu', 'sms', 'send', `${prefix}${from.slice(-4)}`, text);
    sms('5551001', T('Czy bedziesz o 19'));
    const first = await until('the SMS in the panel', () => rowFor(T('Czy bedziesz o 19')), 30000);
    assert.ok(first.reply, `a reply action: ${JSON.stringify(first)}`);
    const log = await invoke('dev_notify_log');
    const toast = log.find(e => e.tag === first.tag);
    assert(toast && toast.reply, `the toast carries a reply field: ${JSON.stringify(log)}`);
    report.replyLabel = first.reply;

    // (a) From the panel: typed, Enter.
    const sentBefore = sentSms().length;
    const field = (await rowOf(first.tag)).getByRole('textbox', { name: new RegExp(`^${first.reply}: `) });
    await field.fill('Tak, z panelu');
    const t0 = Date.now();
    await field.press('Enter');
    // Replied ok: "Sent" (or Messages already took the notification away after the reply: the row is gone).
    await until('Replied ok in the panel', async () => { const row = (await state()).notifications.find(n => n.tag === first.tag); return !row || row.status === 'sent'; });
    const panelLatency = Date.now() - t0;
    await until('the SMS from the panel went out', () => sentSms().some(r => r.includes('Tak, z panelu')), 20000);
    if ((await state()).notifications.some(n => n.tag === first.tag)) assert.equal(await (await rowOf(first.tag)).locator('input').inputValue(), '', 'the field is cleared after Replied ok');
    await panel.locator('#notifications').screenshot({ path: path.join(root, 'panel-after-reply.png') }).catch(() => {});
    report.panelReply = { latencyMs: panelLatency, sentBefore, sentAfter: sentSms().length };

    // (b) From the toast (the WinRT handler's function): the next SMS from another number.
    sms('5551002', T('Kupisz chleb'));
    const second = await until('the second SMS', () => rowFor(T('Kupisz chleb')), 30000);
    const t1 = Date.now();
    await invoke('dev_toast_activate', { tag: second.tag, arguments: 'reply', input: 'Tak, z toastu' });
    await until('the SMS from the toast went out', () => sentSms().some(r => r.includes('Tak, z toastu')), 20000);
    report.toastReply = { latencyMs: Date.now() - t1 };
    // An empty reply from the toast sends nothing; the body opens the panel.
    await invoke('dev_toast_activate', { tag: second.tag, arguments: 'reply', input: '  ' });

    // (c) ✕ in the panel -> gone on the phone.
    sms('5551003', T('Do usuniecia z komputera'));
    const third = await until('the third SMS', () => rowFor(T('Do usuniecia z komputera')), 30000);
    // Messages posts an incoming SMS's notification a few times while it processes it; a cancel in between is undone by its next
    // post (the PC then shows it again, as the phone does). Dismiss once it has settled.
    await delay(2500);
    assert(phoneHas(T('Do usuniecia z komputera')), 'Messages shows it');
    const t2 = Date.now();
    await (await rowOf(third.tag)).getByRole('button', { name: /^Dismiss the notification: / }).click();
    await until('the row is gone on the PC', async () => !(await state()).notifications.some(n => n.tag === third.tag));
    await until('gone on the phone', () => !phoneHas(T('Do usuniecia z komputera')), 10000);
    report.panelDismiss = { latencyMs: Date.now() - t2 };
    await until('its toast is gone too', async () => !(await invoke('dev_notify_history')).includes(third.tag), 5000);
    assert(!(await state()).notifications.some(n => n.tag === third.tag), 'not back after the Removed from the phone');

    // (d) Dismissed on the phone -> gone on the PC.
    sms('5551004', T('Do usuniecia z telefonu'));
    const fourth = await until('the fourth SMS', () => rowFor(T('Do usuniecia z telefonu')), 30000);
    await until('its toast in Windows', async () => (await invoke('dev_notify_history')).includes(fourth.tag));
    const phoneKey = keyWithText(T('Do usuniecia z telefonu'));
    assert(phoneKey, 'its key on the phone');
    const t3 = Date.now();
    device('shell', `cmd notification snooze --for 60000 '${phoneKey}'`); // the key has '|': quoted for the phone's shell
    await until('the row is gone on the PC', async () => !(await state()).notifications.some(n => n.tag === fourth.tag));
    await until('the toast is gone on the PC', async () => !(await invoke('dev_notify_history')).includes(fourth.tag));
    report.phoneDismiss = { latencyMs: Date.now() - t3 };

    // (e) A popup closed by the person dismisses on the phone; timed out doesn't.
    sms('5551005', T('Wygasnie'));
    const fifth = await until('the fifth SMS', () => rowFor(T('Wygasnie')), 30000);
    await delay(2500); // settled, as above
    assert(phoneHas(T('Wygasnie')));
    await invoke('dev_toast_dismissed', { tag: fifth.tag, reason: 2 });
    await delay(1500);
    assert(phoneHas(T('Wygasnie')), 'timed out: stays on the phone');
    assert((await state()).notifications.some(n => n.tag === fifth.tag), 'and in the panel');
    await invoke('dev_toast_dismissed', { tag: fifth.tag, reason: 0 });
    await until('closed by the person: gone on the phone', () => !phoneHas(T('Wygasnie')), 10000);
    report.toastClose = true;

    await panel.locator('body').screenshot({ path: path.join(root, 'pc-panel.png') }).catch(() => {});
    device('shell', 'run-as', 'io.github.prejas3.windconnect', 'touch', 'cache/wn44-done');
    const done = await finished;
    await fs.writeFile(path.join(root, 'instrumentation.log'), done);
    assert(done.includes('OK (1 test)'), done);
    report.latencyNote = 'panelReply: PC invoke -> Replied ok in the panel (the PendingIntent sent on the phone and back). toastReply: invoke -> SMS in content://sms/sent, panelDismiss: click -> PC row gone, phoneDismiss: snooze -> PC row and toast gone; the last three include adb polling (~0.2-1 s).';
    await fs.writeFile(path.join(root, 'run.json'), JSON.stringify(report, null, 2)); await fs.rm(offerFile);
    console.log(JSON.stringify(report, null, 2));
  } catch (error) {
    // Diagnostics (titles/texts of test SMS only).
    if (invoke) await fs.writeFile(path.join(root, 'failure.json'), JSON.stringify({ state: await invoke('pairing_snapshot').catch(e => String(e)), log: await invoke('dev_notify_log').catch(e => String(e)), phone: (() => { try { return messagesKeys(); } catch (e) { return String(e); } })() }, null, 2)).catch(() => {});
    try { await fs.writeFile(path.join(root, 'logcat.txt'), spawnSync(adb, ['-s', serial, 'logcat', '-d', '-t', '400'], { encoding: 'utf8', windowsHide: true }).stdout); } catch {}
    throw error;
  } finally {
    try { device('shell', 'run-as', 'io.github.prejas3.windconnect', 'touch', 'cache/wn44-done'); } catch {}
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
