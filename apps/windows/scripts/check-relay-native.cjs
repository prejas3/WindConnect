// CCS/1 PURPOSE:WN-92/WN-139 relay switch in the real app: visible, off by default, a refused URL shows its error,
// on writes relay-url and survives a restart, off deletes it. Screenshots in RELAY_ARTIFACTS.
// Run: cargo build -p windconnect-windows; node apps/windows/scripts/check-relay-native.cjs [exe]
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const { spawn, spawnSync } = require('node:child_process');
const assert = require('node:assert/strict');
const path = require('node:path');
const fs = require('node:fs/promises');
const executable = path.resolve(process.argv[2] || 'target/debug/windconnect-windows.exe');
const port = 19331;
const artifacts = process.env.RELAY_ARTIFACTS || path.join(process.env.TEMP, 'windconnect-wn139');
const data = path.join(artifacts, 'pc-' + process.pid);
const file = path.join(data, 'relay-url');
const url = 'wss://windconnect-relay.example.workers.dev';
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const exists = p => fs.access(p).then(() => true, () => false);

function showMain() {
  const script = `Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public class RelayProbe { [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string cls,string title); [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd); [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hwnd,int command); }'; $w=[RelayProbe]::FindWindow("WindConnect.MainWindow",$null); [RelayProbe]::ShowWindow($w,9) | Out-Null; [RelayProbe]::SetForegroundWindow($w) | Out-Null`;
  spawnSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', script], { windowsHide: true, timeout: 15000 });
}

async function launch() {
  const child = spawn(executable, ['--autostart'], { windowsHide: true, stdio: 'pipe', env: { ...process.env,
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`, WEBVIEW2_USER_DATA_FOLDER: path.join(artifacts, 'webview'),
    WINDCONNECT_TEST_DATA_DIR: data } });
  let exited = false, stderr = '';
  child.once('exit', () => { exited = true; });
  child.stderr.on('data', d => { stderr += d; });
  for (let i = 0; i < 100; i++) { try { if ((await fetch(`http://127.0.0.1:${port}/json/version`)).ok) break; } catch {} if (exited) throw Error(stderr); await delay(200); }
  const browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
  let main;
  for (let i = 0; i < 100 && !main; i++) {
    main = browser.contexts().flatMap(c => c.pages()).find(p => p.url().includes('tauri.localhost') && !/panel.html|tray-menu.html/.test(p.url()));
    if (!main) await delay(100);
  }
  assert(main, 'main webview: ' + stderr);
  await main.waitForFunction(() => !document.querySelector('#relay-settings').hidden && !document.querySelector('#relay-enabled').disabled, null, { timeout: 30000 });
  showMain();
  await main.locator('#relay-settings').scrollIntoViewIfNeeded();
  return { child, browser, main, stderr: () => stderr };
}

async function stop(app) {
  await app.browser.close().catch(() => {});
  app.child.kill();
  await new Promise(resolve => app.child.exitCode !== null ? resolve() : app.child.once('exit', resolve));
  await delay(500);
}

(async () => {
  await fs.mkdir(data, { recursive: true });
  let app = await launch();
  const log = [];
  try {
    const m = app.main;
    assert.equal(await m.locator('#relay-settings').isVisible(), true, 'section visible');
    assert.equal(await m.locator('#relay-enabled').getAttribute('aria-checked'), 'false', 'off by default');
    assert.equal(await exists(file), false, 'no relay-url file by default');
    log.push('visible, off by default, no relay-url');
    await m.locator('#relay-url').fill('http://relay.example');
    await m.locator('#relay-enabled').click();
    await m.waitForFunction(() => document.querySelector('#relay-error').textContent.length > 0);
    const error = await m.locator('#relay-error').textContent();
    assert.match(error, /^Nieprawidłowy adres: /);
    assert.equal(await m.locator('#relay-enabled').getAttribute('aria-checked'), 'false', 'a refused URL leaves it off');
    assert.equal(await exists(file), false);
    await m.screenshot({ path: path.join(artifacts, 'relay-refused.png') });
    log.push('refused URL: ' + error);
    await m.locator('#relay-url').fill(url);
    await m.locator('#relay-enabled').click();
    await m.waitForFunction(() => document.querySelector('#relay-enabled').getAttribute('aria-checked') === 'true');
    assert.equal((await fs.readFile(file, 'utf8')).trim(), url, 'relay-url written');
    await m.screenshot({ path: path.join(artifacts, 'relay-on.png') });
    log.push('on: relay-url = ' + url);
    await stop(app);
    app = await launch();
    assert.equal(await app.main.locator('#relay-enabled').getAttribute('aria-checked'), 'true', 'on survives a restart');
    assert.equal(await app.main.locator('#relay-url').inputValue(), url);
    log.push('restart: still on with the same URL');
    await app.main.locator('#relay-enabled').click();
    await app.main.waitForFunction(() => document.querySelector('#relay-enabled').getAttribute('aria-checked') === 'false');
    assert.equal(await exists(file), false, 'off deletes relay-url');
    await app.main.screenshot({ path: path.join(artifacts, 'relay-off.png') });
    log.push('off: relay-url deleted');
    await stop(app);
    app = await launch();
    assert.equal(await app.main.locator('#relay-enabled').getAttribute('aria-checked'), 'false', 'off survives a restart');
    log.push('restart: still off');
    console.log(JSON.stringify({ ok: true, artifacts, log }, null, 1));
  } finally { await stop(app); }
})().catch(error => { console.error(error); process.exit(1); });
