// CCS/1 PURPOSE:WN-50 "Dzwoń" tile with a stubbed IPC: disabled without find/1 or offline, ring -> starting -> ringing ->
// "Zatrzymaj" sends ring-stop, outcomes in the status line, keyboard, Light/Dark, 150% text | DEP:external dev-only Playwright 1.61.1.
const assert = require('node:assert/strict');
const http = require('node:http');
const fs = require('node:fs/promises');
const path = require('node:path');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const root = path.resolve(__dirname, '../frontend');
const out = process.env.UI_ARTIFACTS || path.join(require('node:os').tmpdir(), 'windconnect-wn50-ui');
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.ttf': 'font/ttf' };
const server = http.createServer(async (req, res) => {
  const p = new URL(req.url, 'http://x').pathname;
  const f = path.resolve(root, '.' + p);
  if (!f.startsWith(root + path.sep)) { res.writeHead(403).end(); return; }
  try { res.setHeader('Content-Type', types[path.extname(f)] || 'text/plain'); res.end(await fs.readFile(f)); }
  catch { res.writeHead(404).end(); }
});
const phone = { key: 'a'.repeat(64), name: 'Galaxy S25 Ultra z bardzo długą nazwą telefonu', fingerprint: 'K4TQ-9WZM-2HXP', paired_at: 1794300000, connected: true, owed: false };
const state = find => ({ phase: 'connected', phone: phone.name, qr: null, message: 'Połączono · Wi-Fi', skipped_records: 0, code: null, connected: true, devices: [phone], paired: true, battery: null, find: { available: true, state: 'idle', last: null, ring_ms: null, ...find } });
const stub = first => {
  window.__calls = [];
  window.__listeners = {};
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => {
      window.__calls.push([cmd, args]);
      if (cmd === 'pairing_snapshot') return structuredClone(first);
      if (cmd === 'files_snapshot') return { phone: null, rows: [], active: 0, activeSize: 0, activeDone: 0 };
      if (cmd === 'panel_snapshot') return { shortcut_error: null };
      return null;
    } },
    event: { listen: async (name, cb) => { window.__listeners[name] = cb; return () => {}; } },
  };
};
(async () => {
  await fs.mkdir(out, { recursive: true });
  await new Promise(r => server.listen(0, '127.0.0.1', r));
  const browser = await chromium.launch({ headless: true });
  try {
    const page = await browser.newPage({ locale: 'pl-PL', viewport: { width: 380, height: 640 } });
    const errors = [];
    page.on('pageerror', e => errors.push(e.message));
    await page.addInitScript(stub, state({}));
    await page.goto(`http://127.0.0.1:${server.address().port}/panel.html`);
    const tile = page.locator('#find-phone');
    await page.locator('#quick-actions:not([hidden])').waitFor();
    await page.evaluate(() => document.fonts.ready);
    const emit = s => page.evaluate(v => window.__listeners['pairing-state']({ payload: v }), s);
    const sent = () => page.evaluate(() => window.__calls.filter(([c]) => c === 'pairing_action').map(([, a]) => a.action));
    const look = async () => ({ disabled: await tile.isDisabled(), pressed: await tile.getAttribute('aria-pressed'), label: (await tile.textContent()).trim(), status: (await page.locator('#find-status').textContent()).trim() });

    // 1. Available and idle: "Dzwoń", enabled; a click (and the keyboard) asks to ring.
    assert.deepEqual(await look(), { disabled: false, pressed: 'false', label: 'Dzwoń', status: '' });
    await tile.click();
    assert.deepEqual(await sent(), ['ring']);
    // 2. Starting, then ringing: "Zatrzymaj", pressed; the click stops.
    await emit(state({ state: 'starting' }));
    assert.deepEqual(await look(), { disabled: false, pressed: 'true', label: 'Zatrzymaj', status: 'Łączenie z telefonem…' });
    await emit(state({ state: 'ringing', ring_ms: 180 }));
    assert.equal((await look()).status, 'Telefon dzwoni, także w trybie cichym');
    await page.screenshot({ path: path.join(out, 'panel-find-ringing.png'), clip: { x: 0, y: 0, width: 380, height: 200 } });
    await tile.focus(); await page.keyboard.press('Enter');
    assert.deepEqual(await sent(), ['ring', 'ring-stop']);
    // 3. Outcomes in the status line, the tile back to "Dzwoń".
    await emit(state({ last: 'Zatrzymano na telefonie' }));
    assert.deepEqual(await look(), { disabled: false, pressed: 'false', label: 'Dzwoń', status: 'Zatrzymano na telefonie' });
    await emit(state({ last: 'Połączenie przerwane — telefon może nadal dzwonić (do 60 s)' }));
    assert.match((await look()).status, /może nadal dzwonić/);
    // 4. Not allowed on the phone: disabled with an explanation. Offline: disabled, no line.
    await emit(state({ available: false }));
    const off = await look();
    assert.equal(off.disabled, true); assert.match(off.status, /włącz je w ustawieniach/);
    await emit({ ...state({}), phase: 'offline', connected: false });
    assert.deepEqual(await look(), { disabled: true, pressed: 'false', label: 'Dzwoń', status: '' });
    // 5. Themes and 150% text while ringing: nothing overflows.
    await emit(state({ state: 'ringing' }));
    for (const theme of ['light', 'dark']) {
      for (const text of ['100%', '150%']) {
        await page.evaluate(([t, f]) => { document.documentElement.dataset.theme = t; document.documentElement.style.fontSize = f; }, [theme, text]);
        await page.waitForTimeout(200);
        assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, `overflow ${theme} ${text}`);
        await page.screenshot({ path: path.join(out, `panel-find-${theme}-${text.replace('%', '')}.png`), clip: { x: 0, y: 0, width: 380, height: 200 } });
      }
    }
    assert.deepEqual(errors, []);
    console.log(JSON.stringify({ findUi: true, ringAndStop: true, outcomes: true, unavailable: true, themes: 2, artifacts: out }));
  } finally {
    await browser.close();
    server.close();
  }
})().catch(e => { console.error(e); process.exitCode = 1; });
