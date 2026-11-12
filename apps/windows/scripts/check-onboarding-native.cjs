// CCS/1 PURPOSE:WC-34 first start of the real app with an empty store: the main window (not the panel) shows the QR
// without a click; a phone stand-in pairs from the decoded canvas; step 3 "Gotowe" shows. Reports what the system adds:
// launch -> QR on screen, QR decoded -> Gotowe. The "< 60 s for a new user" verdict comes from a person, not this script.
// DEP:Playwright 1.61.1 (PLAYWRIGHT_MODULE), JAVA_HOME + ZXING_CORE_JAR, `cargo build -p windconnect-windows`,
// `cargo build -p wind-ffi --example desktop_pairing_peer`; the installed WindConnect closed (single instance).
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const { spawn, spawnSync } = require('node:child_process');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const os = require('node:os');
const crypto = require('node:crypto');
const root = path.join(os.tmpdir(), 'windconnect-wc34-' + crypto.randomUUID());
const exe = path.resolve(process.argv[2] || 'target/debug/windconnect-windows.exe');
const peerExe = path.resolve('target/debug/examples/desktop_pairing_peer.exe');
const port = 19334;
const delay = ms => new Promise(r => setTimeout(r, ms));
let app, peer, browser, main;
const invoke = (command, args) => main.evaluate(({ command, args }) => window.__TAURI__.core.invoke(command, args), { command, args });
async function waitState(predicate, timeout = 10000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) { const state = await invoke('pairing_snapshot'); if (predicate(state)) return state; await delay(25); }
  throw Error('pairing state timeout');
}
(async () => {
  try {
    await fs.mkdir(root, { recursive: true });
    const launched = performance.now();
    app = spawn(exe, [], { windowsHide: true, stdio: 'ignore', env: { ...process.env, WINDCONNECT_TEST_DATA_DIR: path.join(root, 'pc'), WINDCONNECT_TEST_PAIR_IP: '127.0.0.1', WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`, WEBVIEW2_USER_DATA_FOLDER: path.join(root, 'webview') } });
    for (let attempt = 0; attempt < 150 && !browser; attempt++) {
      try { if ((await fetch(`http://127.0.0.1:${port}/json/version`)).ok) browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`, { timeout: 1500 }); } catch {}
      if (app.exitCode !== null) throw Error('app exited before WebView2 (is the installed WindConnect still running?)');
      if (!browser) await delay(100);
    }
    assert(browser, 'WebView2 debugger connected');
    for (let attempt = 0; attempt < 100 && !main; attempt++) { main = browser.contexts().flatMap(c => c.pages()).find(p => p.url().endsWith('/index.html') || p.url().endsWith('/')); if (!main) await delay(50); }
    assert(main, 'main window page');
    await main.locator('#onboarding-qr').waitFor({ state: 'visible', timeout: 15000 });
    const launchToQrMs = performance.now() - launched;
    const visible = await invoke('tray_snapshot');
    assert.equal(visible.main_visible, true, 'the main window opened itself');
    assert.equal((await invoke('panel_snapshot')).visible, false, 'not the panel');
    assert.equal(await main.locator('#home').isVisible(), false);
    assert.equal(await main.locator('#onboarding-qr').getAttribute('aria-label'), 'Kod parowania');
    for (const theme of ['light', 'dark']) {
      await main.evaluate(t => { localStorage.setItem('windconnect-theme', t); document.documentElement.dataset.theme = t; }, theme);
      await main.screenshot({ path: path.join(root, `onboarding-qr-${theme}.png`) });
    }
    await main.evaluate(() => { document.documentElement.dataset.theme = 'light'; });
    const qrFile = path.join(root, 'qr.png'), offerFile = path.join(root, 'offer.txt');
    await main.locator('#onboarding-qr').screenshot({ path: qrFile });
    const javaFile = path.join(root, 'DecodeQr.java');
    await fs.writeFile(javaFile, `import javax.imageio.ImageIO; import java.io.File; import java.nio.file.*; import com.google.zxing.*; import com.google.zxing.common.HybridBinarizer; class DecodeQr { public static void main(String[] a) throws Exception { var i=ImageIO.read(new File(a[0])); var p=i.getRGB(0,0,i.getWidth(),i.getHeight(),null,0,i.getWidth()); var r=new MultiFormatReader().decode(new BinaryBitmap(new HybridBinarizer(new RGBLuminanceSource(i.getWidth(),i.getHeight(),p)))); Files.writeString(Path.of(a[1]),r.getText()); } }`);
    const decoded = spawnSync(path.join(process.env.JAVA_HOME, 'bin/java.exe'), ['-cp', process.env.ZXING_CORE_JAR, javaFile, qrFile, offerFile], { windowsHide: true, encoding: 'utf8', timeout: 10000 });
    assert.equal(decoded.status, 0, decoded.stderr);
    const scanned = performance.now();
    peer = spawn(peerExe, [offerFile, await invoke('dev_pairing_address'), path.join(root, 'phone')], { windowsHide: true, stdio: 'ignore' });
    await main.locator('#onboarding-done').waitFor({ state: 'visible', timeout: 15000 });
    const qrToDoneMs = performance.now() - scanned;
    assert.match(await main.locator('#onboarding-done-title').textContent(), /Testowy telefon połączony/);
    assert.equal(await main.evaluate(() => document.activeElement?.id), 'onboarding-finish');
    for (const theme of ['light', 'dark']) {
      await main.evaluate(t => { document.documentElement.dataset.theme = t; }, theme);
      await main.screenshot({ path: path.join(root, `onboarding-done-${theme}.png`) });
    }
    await main.locator('#onboarding-finish').click();
    await main.locator('#onboarding').waitFor({ state: 'hidden' });
    await fs.writeFile(path.join(root, 'phone', 'done'), 'done');
    const report = { launchToQrMs: Math.round(launchToQrMs), qrToDoneMs: Math.round(qrToDoneMs), systemAddsMs: Math.round(launchToQrMs + qrToDoneMs), mainWindowFirst: true, artifacts: root };
    await fs.writeFile(path.join(root, 'run.json'), JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report));
  } finally {
    if (main) await invoke('tray_action', { action: 'quit' }).catch(() => {});
    await delay(1500);
    if (browser) await browser.close().catch(() => {});
    if (app && app.exitCode === null) app.kill();
    if (peer && peer.exitCode === null) peer.kill();
  }
})().catch(e => { console.error(e); console.error('artifacts:', root); process.exitCode = 1; });
