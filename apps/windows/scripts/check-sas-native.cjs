// CCS/1 PURPOSE:WN-27 end to end without a camera: real PC panel -> SAS window -> real mDNS TXT pair -> FFI phone stand-in
// (same calls as Android) -> same code on both screens -> rejected pairs nobody -> accepted pairs and connects.
// DEP:Playwright 1.61.1 (PLAYWRIGHT_MODULE), `cargo build -p windconnect-windows` and
// `cargo build -p wind-ffi --example sas_pairing_peer`; a network where mDNS works (multicast on loopback/LAN).
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const { spawn } = require('node:child_process');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const os = require('node:os');
const crypto = require('node:crypto');
const root = path.join(os.tmpdir(), 'windconnect-wn27-' + crypto.randomUUID());
const exe = path.resolve(process.argv[2] || 'target/debug/windconnect-windows.exe');
const peerExe = path.resolve('target/debug/examples/sas_pairing_peer.exe');
const port = 19327;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
let app, browser, panel;
const peers = [];
const invoke = (command, args) => panel.evaluate(({ command, args }) => window.__TAURI__.core.invoke(command, args), { command, args });
async function waitState(predicate, timeout = 20000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) { const state = await invoke('pairing_snapshot'); if (predicate(state)) return state; await delay(50); }
  throw Error('pairing state timeout: ' + JSON.stringify(await invoke('pairing_snapshot')));
}
function runPeer(decision, dir) {
  const child = spawn(peerExe, [decision, dir], { windowsHide: true, stdio: ['ignore', 'pipe', 'inherit'] });
  peers.push(child);
  const lines = [];
  child.stdout.setEncoding('utf8');
  child.stdout.on('data', chunk => lines.push(...chunk.split(/\r?\n/).filter(Boolean)));
  const line = async (prefix, timeout = 30000) => {
    const deadline = Date.now() + timeout;
    while (Date.now() < deadline) { const found = lines.find(l => l.startsWith(prefix)); if (found) return found; if (child.exitCode !== null && !lines.find(l => l.startsWith(prefix))) break; await delay(50); }
    throw Error(`peer: no ${prefix} in ${JSON.stringify(lines)}`);
  };
  return { child, line, exited: () => new Promise(r => child.exitCode !== null ? r(child.exitCode) : child.once('exit', r)) };
}
(async () => {
  try {
    await fs.mkdir(root, { recursive: true });
    app = spawn(exe, [], { windowsHide: true, stdio: 'ignore', env: { ...process.env, WINDCONNECT_TEST_DATA_DIR: path.join(root, 'pc'), WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`, WEBVIEW2_USER_DATA_FOLDER: path.join(root, 'webview') } });
    for (let attempt = 0; attempt < 150; attempt++) {
      try { if ((await fetch(`http://127.0.0.1:${port}/json/version`)).ok) { browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`, { timeout: 1500 }); break; } } catch {}
      if (app.exitCode !== null) throw Error('app exited before WebView2');
      await delay(100);
    }
    assert(browser, 'WebView2 debugger connected');
    for (let attempt = 0; attempt < 100 && !panel; attempt++) { panel = browser.contexts().flatMap(c => c.pages()).find(p => p.url().includes('panel.html')); if (!panel) await delay(50); }
    assert(panel, 'panel');
    await waitState(s => s.phase === 'waiting');
    await panel.locator('#pairing-sas').click();
    await waitState(s => s.phase === 'sas-waiting');

    // 1. The person on the PC sees a different code (or doesn't trust it): nobody is paired.
    const first = runPeer('accept', path.join(root, 'phone-1'));
    const firstCode = (await first.line('CODE ')).split(' ').slice(1, 3).join(' ');
    const asked = await waitState(s => s.phase === 'sas-code');
    assert.equal(asked.code, firstCode, 'same code on both screens');
    assert.equal(await panel.locator('#pairing-code').textContent(), firstCode);
    assert.equal(asked.phone, 'Testowy telefon');
    await panel.locator('#pairing-reject').click();
    await first.line('NOT PAIRED');
    await first.exited();
    await waitState(s => s.phase === 'sas-waiting');
    assert.equal(await fs.readFile(path.join(root, 'pc', 'paired.txt'), 'utf8').catch(() => ''), '', 'PC recorded nothing');

    // 2. Both accept the same code: paired on both sides, then the phone connects.
    const second = runPeer('accept', path.join(root, 'phone-2'));
    const code = (await second.line('CODE ')).split(' ').slice(1, 3).join(' ');
    assert.match(code, /^\d{3} \d{3}$/);
    await waitState(s => s.phase === 'sas-code' && s.code === code);
    await panel.locator('#pairing-accept').click();
    await second.line('PAIRED');
    await second.line('SESSION');
    const connected = await waitState(s => s.phase === 'connected');
    assert.equal(connected.phone, 'Testowy telefon');
    // §12.8: the name is stored percent-encoded (a space is %20).
    assert.match(await fs.readFile(path.join(root, 'pc', 'paired.txt'), 'utf8'), /Testowy%20telefon/);
    await panel.screenshot({ path: path.join(root, 'connected.png') });
    await fs.writeFile(path.join(root, 'phone-2', 'done'), 'done');
    await second.exited();
    console.log(`PASS: SAS without a camera end to end (reject pairs nobody; accept pairs and connects); output: ${root}`);
  } finally {
    for (const p of peers) if (p.exitCode === null) p.kill();
    if (panel) await invoke('tray_action', { action: 'quit' }).catch(() => {});
    if (browser) await browser.close().catch(() => {});
    await delay(500);
    if (app && app.exitCode === null) app.kill();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
