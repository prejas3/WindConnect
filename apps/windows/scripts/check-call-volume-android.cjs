// CCS/1 PURPOSE:WN-48 real PC app (debug) + a real SMTC session (examples/smtc_player, AUMID WindConnect.TestPlayer; the app
// follows only it, WINDCONNECT_TEST_MEDIA_APP) + Android emulator: an incoming call pauses the PC's player before the second ring,
// and volume both ways (PROTOCOL.md §9.9-9.10). PairingTest#callProof holds the session and runs the script's commands. Checks:
//   (a) 5 calls (`adb emu gsm call`): ring -> paused, p95 < 1 s (the second ring is ~6 s in); hang up -> resumed;
//   (b) answered: stays paused until hung up; (c) stopped by hand during the call: not resumed;
//   (d) "Pause playback during calls" off: a call pauses nothing;
//   (g) vibrate mode: by the audio mode alone (reported) and with the call-notification signal (asserted);
//   (e) the panel sets STREAM_MUSIC; a volume key on the phone moves the panel; mute, and a level alone never unmutes;
//   (f) the phone sets the PC's volume (Core Audio); the person's level and mute are restored right after (and in `finally`).
// Restores the ringer mode, the listener access, the assistant and the PC's volume.
// The reply texts are test strings; nothing logs them. Restores the listener access and removes the phone toasts it made.
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const { spawn, spawnSync } = require('node:child_process');
const fs = require('node:fs/promises');
const path = require('node:path');
const os = require('node:os');
const crypto = require('node:crypto');
const dgram = require('node:dgram');
const assert = require('node:assert/strict');
const root = path.join(os.tmpdir(), 'windconnect-wn48-call-' + crypto.randomUUID());
const adb = path.join(process.env.ANDROID_HOME, 'platform-tools/adb.exe');
const serial = process.env.ANDROID_SERIAL || 'emulator-5590';
const port = 19330;
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

let app, browser, socket, timer, redirected = false, granted = false, invoke = null, instrument = null, assistant = null, player = null, vibrate = false, original = null, command = null;
const playerEvents = [];
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
    // The test player first: Playing, under its own AUMID.
    player = spawn(path.resolve('target/debug/examples/smtc_player.exe'), [], { windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
    let buffered = '';
    player.stdout.on('data', d => { const at = Date.now(); buffered += d; const lines = buffered.split(/\r?\n/); buffered = lines.pop(); for (const l of lines) { const m = l.match(/^\d+ event (\w+)/); if (m) playerEvents.push({ word: m[1], at }); } });
    await until('the test player', () => playerEvents !== null && buffered !== undefined, 1000).catch(() => {});
    await delay(1500);
    app = spawn(path.resolve('target/debug/windconnect-windows.exe'), [], { windowsHide: true, stdio: 'ignore', env: { ...process.env, WINDCONNECT_TEST_DATA_DIR: path.join(root, 'pc'), WINDCONNECT_TEST_PAIR_IP: '10.0.2.2', WINDCONNECT_TEST_MEDIA_APP: 'WindConnect.TestPlayer', WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`, WEBVIEW2_USER_DATA_FOLDER: path.join(root, 'webview') } });
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
    device('shell', 'run-as', 'io.github.prejas3.windconnect', 'rm', '-f', 'cache/wn48-cmd', 'cache/wn48-ack');

    // The phone: pair, hold the session and run the script's commands until "done".
    let output = '';
    const finished = new Promise((resolve, reject) => {
      instrument = spawn(adb, ['-s', serial, 'shell', 'am', 'instrument', '-w', '-r', '-e', 'class', 'io.github.prejas3.windconnect.PairingTest#qrConnectsRealDesktop',
        '-e', 'desktopQr', 'true', '-e', 'callProof', 'true', runner], { windowsHide: true });
      const killer = setTimeout(() => instrument.kill(), 400000);
      instrument.stdout.on('data', d => { output += d; }); instrument.stderr.on('data', d => { output += d; });
      instrument.on('error', reject);
      instrument.on('close', () => { clearTimeout(killer); resolve(output); });
    });
    await until('the phone holds the session', () => output.includes('callProofReady=true') || (instrument.exitCode !== null && Promise.reject(Error(output))), 120000);
    clearInterval(timer); timer = null;
    const report = { artifacts: root };
    const state = () => invoke('pairing_snapshot');
    let commands = 0;
    // A command for the phone; its answer: the PC's volume as the phone got it, and STREAM_MUSIC.
    command = async line => {
      device('shell', `run-as io.github.prejas3.windconnect sh -c 'echo ${line} > cache/wn48-cmd'`);
      commands++;
      const ack = await until(`the phone ran "${line}"`, () => {
        try { const a = device('shell', 'run-as', 'io.github.prejas3.windconnect', 'cat', 'cache/wn48-ack').trim(); return a.startsWith(`${commands} `) && a; } catch { return false; }
      }, 15000);
      const pick = name => { const v = ack.match(new RegExp(`${name}=(\\S+)`))[1]; if (v === 'none') return null; const [l, m] = v.split(','); return { level: Number(l), muted: m === 'true' }; };
      return { pc: pick('pc'), phone: pick('phone') };
    };
    // The test player's events as they arrive here (host wall clock).
    const eventsSince = (word, since) => playerEvents.filter(e => e.word === word && e.at >= since);
    const waitEvent = (word, since, ms = 6000) => until(`player ${word}`, () => eventsSince(word, since)[0], ms);
    const gsm = (...args) => device('emu', 'gsm', ...args);
    const mainPage = async () => {
      await invoke('panel_action', { action: 'settings' });
      const main = await until('the main window page', () => browser.contexts().flatMap(c => c.pages()).find(p => /^https?:/.test(p.url()) && !/panel\.html|tray-menu/.test(p.url())));
      await main.waitForFunction(() => !!window.__TAURI__);
      return (command, args) => main.evaluate(({ command, args }) => window.__TAURI__.core.invoke(command, args), { command, args });
    };
    await delay(1500);

    // (a) Five calls: ring -> the PC's player paused (before the second ring, ~6 s), hang up -> resumed after the debounce.
    const pauses = [], resumes = [];
    for (let i = 0; i < 5; i++) {
      const number = `55512${i}`;
      const t0 = Date.now();
      gsm('call', number);
      const paused = await waitEvent('pause', t0);
      pauses.push(paused.at - t0);
      const t1 = Date.now();
      gsm('cancel', number);
      const played = await waitEvent('play', t1);
      resumes.push(played.at - t1);
      await delay(800);
    }
    const p95 = a => [...a].sort((x, y) => x - y)[Math.min(a.length - 1, Math.floor(0.95 * a.length))];
    report.ringToPausedMs = { all: pauses, p95: p95(pauses) };
    report.hangUpToResumedMs = { all: resumes, p95: p95(resumes) };
    assert(p95(pauses) < 1000, `ring -> paused p95 < 1 s: ${pauses}`);

    // (b) Answered: still paused; hung up: resumed.
    let t = Date.now();
    gsm('call', '5551300');
    await waitEvent('pause', t);
    gsm('accept', '5551300');
    await delay(2500);
    assert.equal(eventsSince('play', t).length, 0, 'answered: still paused');
    t = Date.now();
    gsm('cancel', '5551300');
    await waitEvent('play', t);
    report.answered = 'paused until hung up';
    await delay(800);

    // (c) Stopped by hand during the call: not resumed.
    t = Date.now();
    gsm('call', '5551301');
    await waitEvent('pause', t);
    player.stdin.write('stop\n');
    await delay(500);
    gsm('cancel', '5551301');
    await delay(3500);
    assert.equal(eventsSince('play', t).length, 0, 'stopped by hand -> not resumed');
    player.stdin.write('play\n');
    await delay(800);
    report.byHand = 'not resumed';

    // (d) The PC's setting off: a call pauses nothing (the phone's call/1 is refused).
    const mainInvoke = await mainPage();
    await mainInvoke('call_volume_settings', { pauseOnCall: false });
    await until('setting off', async () => !(await state()).pause_on_call);
    t = Date.now();
    gsm('call', '5551302');
    await delay(3000);
    assert.equal(eventsSince('pause', t).length, 0, 'setting off -> no pause');
    gsm('cancel', '5551302');
    await mainInvoke('call_volume_settings', { pauseOnCall: true });
    await until('setting on', async () => (await state()).pause_on_call);
    await delay(2000);
    report.settingOff = 'no pause';

    // (g) Vibrate mode: by the audio mode alone, then with the call-notification signal (notification access on).
    device('shell', 'cmd', 'audio', 'set-ringer-mode', 'VIBRATE'); vibrate = true;
    const vib = {};
    for (const [label, notify] of [['modeOnly', false], ['withListener', true]]) {
      if (notify) { device('shell', 'cmd', 'notification', 'allow_listener', LISTENER); await command('notify on'); await delay(1500); }
      t = Date.now();
      gsm('call', `555140${notify ? 1 : 0}`);
      const got = await waitEvent('pause', t, 5000).catch(() => null);
      vib[label] = got ? got.at - t : null;
      gsm('cancel', `555140${notify ? 1 : 0}`);
      if (got) await waitEvent('play', Date.now() - 100, 8000).catch(() => {});
      await delay(1500);
    }
    await command('notify off');
    device('shell', 'cmd', 'audio', 'set-ringer-mode', 'NORMAL'); vibrate = false;
    report.vibrate = vib;
    assert(vib.modeOnly !== null || vib.withListener !== null, `vibrate: paused by some signal: ${JSON.stringify(vib)}`);
    assert(vib.withListener !== null && vib.withListener < 1000, `vibrate with notification access: ${JSON.stringify(vib)}`);

    // (e) The phone's volume from the panel, and the phone's own change back to the panel.
    await until('the phone volume in the panel', async () => (await state()).phone_volume?.controllable);
    await invoke('phone_volume', { level: 60, muted: null });
    let ack = await until('STREAM_MUSIC at 60 %', async () => { const a = await command('noop'); return Math.abs(a.phone.level - 60) <= 4 && a; }, 15000);
    report.panelToPhone = ack.phone;
    const before = (await state()).phone_volume.level;
    // Changed on the phone by someone else (no music plays on the emulator, so a volume key would move the ringer instead).
    const steps = Number(device('shell', 'cmd', 'media_session', 'volume', '--stream', '3', '--get').match(/volume is (\d+) in range \[0\.\.(\d+)\]/)[1]);
    device('shell', 'cmd', 'audio', 'set-volume', '3', String(steps >= 12 ? steps - 3 : steps + 2));
    await until('the panel follows the phone', async () => (await state()).phone_volume.level !== before, 5000);
    await invoke('phone_volume', { level: null, muted: true });
    await until('muted on the phone', async () => (await command('noop')).phone.muted, 10000);
    await invoke('phone_volume', { level: 30, muted: null });
    await delay(800);
    assert((await command('noop')).phone.muted, 'a level alone never unmutes');
    await invoke('phone_volume', { level: null, muted: false });
    await until('unmuted', async () => !(await command('noop')).phone.muted, 10000);
    report.phoneVolume = { set: 'ok', followsPhone: true, levelAloneKeepsMute: true };

    // (f) The PC's volume from the phone (Core Audio); the person's level and mute are put back right after.
    const start = (await command('noop')).pc;
    if (!start) {
      report.pcVolume = 'no default output device on this PC: skipped';
    } else {
      original = start;
      const target = start.level === 33 ? 34 : 33;
      ack = await command(`pcvol ${target}`);
      const took = await until('Core Audio took it', async () => (await command('noop')).pc.level === target, 6000).catch(() => false);
      // The Set reached Core Audio either way (the COM thread's log); a device with a fixed volume (some HDMI/S/PDIF/USB
      // outputs) keeps its level: then mute shows the change round trip instead.
      const log = await invoke('dev_volume_log');
      assert(log.some(l => l.startsWith(`set Some(${target}) None attached=true`)), `the phone's Set reached Core Audio: ${log}`);
      let muteRoundTrip = null;
      if (!took) {
        await command(`pcmute ${start.muted ? 0 : 1}`);
        muteRoundTrip = await until('the mute came back to the phone', async () => (await command('noop')).pc.muted !== start.muted, 6000).then(() => true).catch(() => false);
      }
      await command(`pcvol ${start.level}`);
      await command(`pcmute ${start.muted ? 1 : 0}`);
      await until('restored', async () => { const pc = (await command('noop')).pc; return pc.level === start.level && pc.muted === start.muted; }, 8000);
      original = null;
      report.pcVolume = took ? { from: start.level, to: target, restored: true }
        : { from: start.level, fixedLevelDevice: true, setReachedCoreAudio: true, muteRoundTrip, restored: true };
      assert(took || muteRoundTrip, `the PC's volume changed from the phone: ${JSON.stringify(report.pcVolume)}`);
    }

    report.callLog = await invoke('dev_call_log');
    await command('done').catch(() => {});
    const done = await finished;
    await fs.writeFile(path.join(root, 'instrumentation.log'), done);
    assert(done.includes('OK (1 test)'), done);
    await fs.writeFile(path.join(root, 'run.json'), JSON.stringify(report, null, 2)); await fs.rm(offerFile);
    console.log(JSON.stringify({ ...report, callLog: report.callLog.length }, null, 2));
  } catch (error) {
    if (invoke) await fs.writeFile(path.join(root, 'volume-log.json'), JSON.stringify(await invoke('dev_volume_log').catch(e => String(e)))).catch(() => {});
    // Diagnostics (titles/texts of test SMS only).
    if (invoke) await fs.writeFile(path.join(root, 'failure.json'), JSON.stringify({ state: await invoke('pairing_snapshot').catch(e => String(e)), log: await invoke('dev_notify_log').catch(e => String(e)), phone: (() => { try { return messagesKeys(); } catch (e) { return String(e); } })() }, null, 2)).catch(() => {});
    try { await fs.writeFile(path.join(root, 'logcat.txt'), spawnSync(adb, ['-s', serial, 'logcat', '-d', '-t', '400'], { encoding: 'utf8', windowsHide: true }).stdout); } catch {}
    throw error;
  } finally {
    if (original && command) { await command(`pcvol ${original.level}`).catch(() => {}); await command(`pcmute ${original.muted ? 1 : 0}`).catch(() => {}); }
    try { device('shell', `run-as io.github.prejas3.windconnect sh -c 'echo done > cache/wn48-cmd'`); } catch {}
    if (vibrate) try { device('shell', 'cmd', 'audio', 'set-ringer-mode', 'NORMAL'); } catch {}
    for (const n of ['5551300', '5551301', '5551302', '5551400', '5551401']) try { device('emu', 'gsm', 'cancel', n); } catch {}
    if (player && player.exitCode === null) { player.stdin.end('quit\n'); setTimeout(() => player.kill(), 1000); }
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
