// CCS/1 PURPOSE:real WebView2 panel, Win+Shift+W, Esc/blur, preloading and request-to-frame timings.
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const { spawn, spawnSync } = require('node:child_process');
const assert = require('node:assert/strict');
const path = require('node:path');
const fs = require('node:fs/promises');
const executable = path.resolve(process.argv[2] || 'target/debug/windconnect-windows.exe');
const port = 19324;
const artifacts = process.env.PANEL_ARTIFACTS || path.join(process.env.TEMP, 'windconnect-wn24');
const child = spawn(executable, ['--autostart'], { windowsHide: true, stdio: 'pipe', env: { ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`, WEBVIEW2_USER_DATA_FOLDER: path.join(artifacts, 'webview'), WINDCONNECT_TEST_DATA_DIR: path.join(artifacts, 'pc-' + process.pid) } });
let exited = false, stderr = '';
child.once('exit', () => { exited = true; });
child.stderr.on('data', data => { stderr += data; });
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
function native(action) {
  const script = `Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public class PanelProbe { [DllImport("user32.dll")] public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra); [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string cls,string title); [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd); [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hwnd,int command); }'; ${action === 'shortcut' ? '[PanelProbe]::keybd_event(0x5B,0,0,[UIntPtr]::Zero); [PanelProbe]::keybd_event(0x10,0,0,[UIntPtr]::Zero); [PanelProbe]::keybd_event(0x57,0,0,[UIntPtr]::Zero); [PanelProbe]::keybd_event(0x57,0,2,[UIntPtr]::Zero); [PanelProbe]::keybd_event(0x10,0,2,[UIntPtr]::Zero); [PanelProbe]::keybd_event(0x5B,0,2,[UIntPtr]::Zero)' : '$window=[PanelProbe]::FindWindow("WindConnect.MainWindow",$null); [PanelProbe]::ShowWindow($window,9); [PanelProbe]::SetForegroundWindow($window)'}`;
  const result = spawnSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', script], { windowsHide: true, encoding: 'utf8', timeout: 15000 });
  assert.equal(result.status, 0, result.stderr);
}
(async () => {
  let browser;
  try {
    await fs.mkdir(artifacts, { recursive: true });
    let ready = false;
    for (let attempt = 0; attempt < 100; attempt++) { try { if ((await fetch(`http://127.0.0.1:${port}/json/version`)).ok) { ready = true; break; } } catch {} if (exited) throw Error(stderr); await delay(200); }
    assert(ready, `WebView2 debugger started: ${stderr}`);
    browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
    let panel, main;
    for (let attempt = 0; attempt < 60; attempt++) {
      const pages = browser.contexts().flatMap(context => context.pages());
      panel = pages.find(page => page.url().includes('panel.html'));
      main = pages.find(page => page.url().includes('tauri.localhost') && !/panel.html|tray-menu.html/.test(page.url()));
      if (panel && main) break;
      await delay(100);
    }
    assert(panel && main, `panel and main webviews precreated: ${JSON.stringify(browser.contexts().flatMap(context => context.pages()).map(page => page.url()))}; ${stderr}`);
    const errors = []; panel.on('pageerror', error => errors.push(error.message));
    const invoke = (command, args) => panel.evaluate(({command,args}) => window.__TAURI__.core.invoke(command,args), {command,args});
    await panel.waitForFunction(async () => { try { await window.__TAURI__.core.invoke('tray_snapshot'); return true; } catch { return false; } });
    await panel.waitForFunction(async () => (await window.__TAURI__.core.invoke('panel_snapshot')).ready);
    assert.equal((await invoke('panel_snapshot')).visible, false, 'preloaded without login flash');
    const samples = [];
    for (let index = 0; index < 20; index++) {
      await invoke('panel_action', { action: 'show' });
      let state;
      const deadline = Date.now() + 3000;
      do {
        state = await invoke('panel_snapshot');
        if (Number.isFinite(state.painted_us)) break;
        await delay(5);
      } while (Date.now() < deadline);
      assert(Number.isFinite(state.painted_us), `missing frame acknowledgement: ${JSON.stringify(state)}`);
      assert.equal(state.visible, true);
      samples.push(state.painted_us);
      await panel.keyboard.press('Escape');
      await panel.waitForFunction(async () => !(await window.__TAURI__.core.invoke('panel_snapshot')).visible);
    }
    const shortcutError = (await invoke('panel_snapshot')).shortcut_error;
    if (!shortcutError) {
      native('shortcut');
      await panel.waitForFunction(async () => (await window.__TAURI__.core.invoke('panel_snapshot')).visible);
    } else {
      assert.equal(await panel.locator('#shortcut-error').isVisible(), true, 'shortcut conflict is explained');
      await invoke('panel_action', { action: 'show' });
    }
    native('blur');
    await panel.waitForFunction(async () => !(await window.__TAURI__.core.invoke('panel_snapshot')).visible);
    await invoke('panel_action', { action: 'show' });
    await panel.locator('#settings').click();
    assert.equal((await invoke('panel_snapshot')).visible, false);
    await invoke('panel_action', { action: 'show' });
    for (const theme of ['light','dark']) {
      await panel.evaluate(theme => { localStorage.setItem('windconnect-theme', theme); window.dispatchEvent(new Event('storage')); }, theme);
      assert.equal(await panel.evaluate(() => document.fonts.check('13px "DM Sans"')), true);
      assert.equal(await panel.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
      await panel.screenshot({ path: path.join(artifacts, `${theme}.png`) });
    }
    await panel.emulateMedia({ reducedMotion: 'reduce' });
    assert.equal(await panel.locator('.panel').evaluate(element => getComputedStyle(element).animationName), 'none');
    // WN-109: unpaired, the whole group is hidden (panel.js, WC-28), so Wyślij and Schowek can't be reached;
    // once connected they are live (their modules report failures). Only the not-yet-built ones stay disabled.
    // Debug builds start with an empty store (WINDCONNECT_TEST_DATA_DIR); a release build reads the real one,
    // so with a phone connected there the group is shown and only the disabled list is checked.
    if ((await invoke('pairing_snapshot')).phase !== 'connected') assert.equal(await panel.locator('#quick-actions').isHidden(), true, 'unpaired: no quick actions');
    assert.deepEqual(await panel.locator('.quick-actions button:disabled').allTextContents(), ['Dzwoń', 'Touchpad'], 'only unbuilt actions are disabled');
    assert.equal(await panel.locator('.quick-actions button:not(:disabled)').count(), 2, 'Wyślij and Schowek are live once connected');
    assert.deepEqual(errors, []);
    const report = { executable, samplesUs: samples, maxUs: Math.max(...samples), preloaded: true, escape: true, shortcut: !shortcutError, shortcutError, blur: true, themes: true, reducedMotion: true, stderr };
    await fs.writeFile(path.join(artifacts,'run.json'), JSON.stringify(report,null,2));
    console.log(JSON.stringify(report));
    assert(report.maxUs < 80000, `panel frame acknowledgement took ${report.maxUs}us`);
    await invoke('tray_action', { action: 'quit' }).catch(() => {});
    for (let attempt = 0; attempt < 40 && !exited; attempt++) await delay(50);
    assert(exited, 'Quit stops app and hotkey worker');
  } finally {
    if (browser) await browser.close();
    if (!exited) child.kill();
    if (!exited) await Promise.race([new Promise(resolve => child.once('exit', resolve)), delay(5000)]);
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
