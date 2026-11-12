// CCS/1 PURPOSE:WN-47 real PC app + a real SMTC session (examples/smtc_player, AUMID WindConnect.TestPlayer) + Android emulator.
// The PC app follows only that player (debug WINDCONNECT_TEST_MEDIA_APP), never the person's own. The phone (PcMediaProof) sees
// it in the app and its media notification, pauses/plays it 10× (Android's media controls and the app's button), checks a
// command outside `actions`, the switch and Gone cleanup. Here: the player logs every button it got; the PC never shows our own
// mirrored session as the phone's player (no echo). Raw timings in run.json.
// LIMIT:emulator NAT needs a UDP redirection for the public discovery hint; no trust or TLS bypass. Restores listener access.
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const { spawn, spawnSync } = require('node:child_process');
const fs = require('node:fs/promises');
const path = require('node:path');
const os = require('node:os');
const crypto = require('node:crypto');
const dgram = require('node:dgram');
const assert = require('node:assert/strict');
const root = path.join(os.tmpdir(), 'windconnect-wn47-pcmedia-' + crypto.randomUUID());
const adb = path.join(process.env.ANDROID_HOME, 'platform-tools/adb.exe');
const serial = process.env.ANDROID_SERIAL || 'emulator-5556';
const port = 19326;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
function device(...args) { const result = spawnSync(adb, ['-s', serial, ...args], { windowsHide: true, encoding: 'utf8', timeout: 20000 }); assert.equal(result.status, 0, result.stderr); return result.stdout; }
const LISTENER = 'io.github.prejas3.windconnect/io.github.prejas3.windconnect.core.MediaListenerService';
let appLog = '', player, playerLog = '', app, instrument, browser, socket, timer, redirected = false, granted = false, output = '';
(async () => {
  try {
    await fs.mkdir(root, { recursive: true });
    device('install', '-r', 'apps/android/app/build/outputs/apk/debug/app-debug.apk');
    device('install', '-r', '-t', 'apps/android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk');
    device('shell', 'cmd', 'notification', 'allow_listener', LISTENER); granted = true;
    device('emu', 'redir', 'add', 'udp:47823:47821'); redirected = true;
    player = spawn(path.resolve('target/debug/examples/smtc_player.exe'), [], { windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
    player.stdout.on('data', d => { playerLog += d; });
    for (let i = 0; i < 100 && !playerLog.includes('ready'); i++) await delay(50);
    assert(playerLog.includes('ready'), 'test SMTC player ready: ' + playerLog);
    app = spawn(path.resolve('target/debug/windconnect-windows.exe'), [], { windowsHide: true, stdio: ['ignore', 'ignore', 'pipe'], env: { ...process.env, WINDCONNECT_TEST_DATA_DIR: path.join(root, 'pc'), WINDCONNECT_TEST_PAIR_IP: '10.0.2.2', WINDCONNECT_TEST_MEDIA_APP: 'WindConnect.TestPlayer', WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`, WEBVIEW2_USER_DATA_FOLDER: path.join(root, 'webview') } });
    app.stderr.on('data', d => { appLog += d; });
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
    instrument = spawn(adb, ['-s', serial, 'shell', 'am', 'instrument', '-w', '-r', '-e', 'class', 'io.github.prejas3.windconnect.PairingTest#qrConnectsRealDesktop', '-e', 'desktopQr', 'true', '-e', 'pcMediaProof', 'true', 'io.github.prejas3.windconnect.test/androidx.test.runner.AndroidJUnitRunner'], { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
    instrument.stdout.on('data', data => { output += data; }); instrument.stderr.on('data', data => { output += data; });
    const deadline = Date.now() + 20000;
    let state;
    do { state = await invoke('pairing_snapshot'); if (state.phase === 'connected') break; if (instrument.exitCode !== null) break; await delay(25); } while (Date.now() < deadline);
    assert.equal(state.phase, 'connected', output);
    // WN-47: the phone drives; here the player's log and the echo check.
    const stage = async (name, limit = 90000) => {
      const start = Date.now();
      while (!output.includes(`pcMediaStage=${name}`)) {
        assert(Date.now() - start < limit, `stage ${name}: ` + output.slice(-2000));
        assert.equal(instrument.exitCode, null, 'session held: ' + output);
        const phonePlayer = (await invoke('pairing_snapshot')).media;
        assert.equal(phonePlayer, null, `no echo: the PC must not show its own player as the phone's (${JSON.stringify(phonePlayer && phonePlayer.title)})`);
        await delay(50);
      }
    };
    const events = () => playerLog.split('\n').filter(l => / event /.test(l)).map(l => l.trim().split(' ').slice(2).join(' '));
    await stage('shown');
    await stage('toggled');
    const toggles = events().filter(e => e === 'pause' || e === 'play');
    assert(toggles.length >= 10, `10 pause/play reached the PC player: ${toggles}`);
    assert.equal(toggles[0], 'pause', 'the first was a pause from the phone');
    player.stdin.write('no-next\n');
    await stage('disallowed');
    const afterOff = playerLog.slice(playerLog.indexOf('next-disabled'));
    assert(!/ event next/.test(afterOff), 'a command outside actions reached the player: ' + afterOff);
    await stage('switched');
    player.stdin.write('quit\n');
    for (let attempt = 0; attempt < 600 && instrument.exitCode === null; attempt++) await delay(50);
    await fs.writeFile(path.join(root, 'instrumentation.log'), output);
    assert(output.includes('OK (1 test)'), output);
    const report = { roundTripMs: output.match(/roundTripMs=([^\r\n]*)/)?.[1]?.split(',').map(Number), playerEvents: events(), proof: output.match(/pcMediaProof=([^\r\n]*)/)?.[1] };
    assert(report.proof?.includes('goneCleanup'), output);
    Object.assign(report, { phone: state.phone, realSession: true, artifacts: root });
    await fs.writeFile(path.join(root, 'run.json'), JSON.stringify(report, null, 2)); await fs.rm(offerFile);
    console.log(JSON.stringify(report));
    await invoke('tray_action', { action: 'quit' }).catch(() => {});
  } finally {
    await fs.writeFile(path.join(root, 'instrumentation.log'), output).catch(() => {});
    await fs.writeFile(path.join(root, 'pc-app.log'), appLog).catch(() => {});
    await fs.writeFile(path.join(root, 'player.log'), playerLog).catch(() => {});
    if (timer) clearInterval(timer); if (socket) socket.close();
    if (instrument && instrument.exitCode === null) { device('shell', 'am', 'force-stop', 'io.github.prejas3.windconnect'); instrument.kill(); }
    if (browser) await browser.close();
    if (app && app.exitCode === null) app.kill();
    if (redirected) device('emu', 'redir', 'del', 'udp:47823');
    if (granted) device('shell', 'cmd', 'notification', 'disallow_listener', LISTENER);
    if (player && player.exitCode === null) { player.stdin.end('quit\n'); setTimeout(() => player.kill(), 1000); }
  }
})().catch(error => { console.error(error); console.error('artifacts:', root); process.exitCode = 1; });
