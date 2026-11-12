// CCS/1 PURPOSE:decode actual PC canvas, pair over real QUIC, verify truthful UI/tray, persistence and replay refusal.
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const { spawn, spawnSync } = require('node:child_process');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const os = require('node:os');
const crypto = require('node:crypto');
const root = path.join(os.tmpdir(), 'windconnect-wn25-' + crypto.randomUUID());
const exe = path.resolve(process.argv[2] || 'target/debug/windconnect-windows.exe');
const peerExe = path.resolve('target/debug/examples/desktop_pairing_peer.exe');
const port = 19325;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
let app, peer, browser, panel;
const invoke = (command, args) => panel.evaluate(({ command, args }) => window.__TAURI__.core.invoke(command, args), { command, args });
async function waitState(predicate, timeout = 10000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) { const state = await invoke('pairing_snapshot'); if (predicate(state)) return state; await delay(25); }
  throw Error('pairing state timeout');
}
async function launch() {
  app = spawn(exe, [], { windowsHide: true, stdio: 'ignore', env: { ...process.env, WINDCONNECT_TEST_DATA_DIR: path.join(root, 'pc'), WINDCONNECT_TEST_PAIR_IP: '127.0.0.1', WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`, WEBVIEW2_USER_DATA_FOLDER: path.join(root, 'webview') } });
  for (let attempt = 0; attempt < 100; attempt++) {
    try { if ((await fetch(`http://127.0.0.1:${port}/json/version`)).ok) { browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`, { timeout: 1500 }); break; } } catch {}
    if (app.exitCode !== null) throw Error('app exited before WebView2');
    await delay(100);
  }
  assert(browser, 'WebView2 debugger connected');
  for (let attempt = 0; attempt < 100; attempt++) { panel = browser.contexts().flatMap(c => c.pages()).find(p => p.url().includes('panel.html')); if (panel) return; await delay(50); }
  throw Error('panel unavailable');
}
async function quit() {
  if (app && app.exitCode === null) { const done = new Promise(resolve => app.once('exit', resolve)); await invoke('tray_action', { action: 'quit' }).catch(() => {}); await Promise.race([done, delay(5000)]); }
  if (browser) await browser.close(); browser = null;
  if (app && app.exitCode === null) app.kill();
}
async function shot(name, theme) {
  await panel.evaluate(theme => { localStorage.setItem('windconnect-theme', theme); window.dispatchEvent(new Event('storage')); }, theme);
  await panel.screenshot({ path: path.join(root, `${name}-${theme}.png`) });
}
(async () => {
  try {
    await fs.mkdir(root, { recursive: true });
    await launch();
    await waitState(s => s.phase === 'waiting');
    // WC-34: with no paired phone the first window is the main one (onboarding); open the panel for this check.
    assert.equal((await invoke('panel_snapshot')).visible, false, 'first start opens onboarding, not the panel');
    await invoke('panel_action', { action: 'show' });
    await panel.waitForFunction(async () => (await window.__TAURI__.core.invoke('panel_snapshot')).visible);
    await panel.locator('#pairing-qr').waitFor({ state: 'visible' });
    await shot('waiting', 'light'); await shot('waiting', 'dark');
    const qrFile = path.join(root, 'qr.png'), offerFile = path.join(root, 'offer.txt');
    await panel.locator('#pairing-qr').screenshot({ path: qrFile });
    const javaFile = path.join(root, 'DecodeQr.java');
    await fs.writeFile(javaFile, `import javax.imageio.ImageIO; import java.io.File; import java.nio.file.*; import com.google.zxing.*; import com.google.zxing.common.HybridBinarizer; class DecodeQr { public static void main(String[] a) throws Exception { var i=ImageIO.read(new File(a[0])); var p=i.getRGB(0,0,i.getWidth(),i.getHeight(),null,0,i.getWidth()); var r=new MultiFormatReader().decode(new BinaryBitmap(new HybridBinarizer(new RGBLuminanceSource(i.getWidth(),i.getHeight(),p)))); Files.writeString(Path.of(a[1]),r.getText()); } }`);
    const start = performance.now();
    const decoded = spawnSync(path.join(process.env.JAVA_HOME, 'bin/java.exe'), ['-cp', process.env.ZXING_CORE_JAR, javaFile, qrFile, offerFile], { windowsHide: true, encoding: 'utf8', timeout: 10000 });
    assert.equal(decoded.status, 0, decoded.stderr);
    const address = await invoke('dev_pairing_address');
    const peerDir = path.join(root, 'phone');
    peer = spawn(peerExe, [offerFile, address, peerDir], { windowsHide: true, stdio: 'ignore' });
    const connected = await waitState(s => s.phase === 'connected', 10000);
    const decodeToConnectedMs = performance.now() - start;
    assert(decodeToConnectedMs < 10000);
    assert.equal(connected.phone, 'Testowy telefon');
    await panel.waitForFunction(async () => (await window.__TAURI__.core.invoke('tray_snapshot')).phase === 'connected');
    assert.equal(await panel.locator('#pairing-qr').isVisible(), false);
    assert.equal(await panel.locator('#phone-name').textContent(), 'Testowy telefon');
    await shot('connected', 'light'); await shot('connected', 'dark');
    await fs.writeFile(path.join(peerDir, 'done'), 'done');
    await waitState(s => s.phase === 'offline');
    await quit();
    await launch();
    const restored = await waitState(s => s.phase === 'offline');
    assert.equal(restored.phone, 'Testowy telefon');
    assert.equal(await panel.locator('#pairing-qr').isVisible(), false);
    await invoke('pairing_action', { action: 'refresh' });
    await waitState(s => s.phase === 'waiting');
    const replay = spawnSync(peerExe, [offerFile, await invoke('dev_pairing_address'), path.join(root, 'replay')], { windowsHide: true, stdio: 'ignore', timeout: 7000 });
    assert.notEqual(replay.status, 0, 'old QR must be refused after restart/new window');
    await invoke('pairing_action', { action: 'close' });
    await waitState(s => s.phase === 'offline');
    assert.equal(await panel.locator('#pairing-qr').isVisible(), false);
    const report = { decodeToConnectedMs, pairToConnectedMs: connected.pair_to_connected_ms, phone: connected.phone, actualSession: true, trayConnected: true, restartPreservesTrust: true, replayRefused: true, cancelHidesQr: true, artifacts: root };
    await fs.writeFile(path.join(root, 'run.json'), JSON.stringify(report, null, 2));
    await fs.rm(offerFile); // CCS/1 SEC:remove one-time test secret; QR PNGs remain local review artifacts.
    console.log(JSON.stringify(report));
  } finally {
    if (peer && peer.exitCode === null) peer.kill();
    await quit();
  }
})().catch(error => { console.error(error); console.error('artifacts:', root); process.exitCode = 1; });
