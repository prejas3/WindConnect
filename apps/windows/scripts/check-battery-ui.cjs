// CCS/1 PURPOSE:WN-49 phone battery in the panel header with a stubbed IPC: level, charging, low, hidden offline, pushed updates
// without any extra IPC call (no polling), label not a live region, Light/Dark, 150% text, no overflow | DEP:external dev-only Playwright 1.61.1.
const assert = require('node:assert/strict');
const http = require('node:http');
const fs = require('node:fs/promises');
const path = require('node:path');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const root = path.resolve(__dirname, '../frontend');
const out = process.env.UI_ARTIFACTS || path.join(require('node:os').tmpdir(), 'windconnect-wn49-ui');
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.ttf': 'font/ttf' };
const server = http.createServer(async (req, res) => {
  const p = new URL(req.url, 'http://x').pathname;
  const f = path.resolve(root, '.' + p);
  if (!f.startsWith(root + path.sep)) { res.writeHead(403).end(); return; }
  try { res.setHeader('Content-Type', types[path.extname(f)] || 'text/plain'); res.end(await fs.readFile(f)); }
  catch { res.writeHead(404).end(); }
});
const phone = { key: 'a'.repeat(64), name: 'Galaxy S25 Ultra z bardzo długą nazwą telefonu', fingerprint: 'K4TQ-9WZM-2HXP', paired_at: 1794300000, connected: true, owed: false };
const state = battery => ({ phase: 'connected', phone: phone.name, qr: null, message: 'Połączono · Wi-Fi', skipped_records: 0, code: null, connected: true, devices: [phone], paired: true, battery });
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
    await page.addInitScript(stub, state({ level: 82, charging: false }));
    await page.goto(`http://127.0.0.1:${server.address().port}/panel.html`);
    const box = page.locator('#phone-battery');
    await box.waitFor();
    await page.evaluate(() => document.fonts.ready);
    const emit = s => page.evaluate(v => window.__listeners['pairing-state']({ payload: v }), s);
    const read = () => box.evaluate(b => ({ text: b.textContent.trim(), label: b.getAttribute('aria-label'), low: b.classList.contains('low'), charging: b.classList.contains('charging'), hidden: b.hidden, width: Number(b.querySelector('.level').getAttribute('width')) }));
    const snapshots = () => page.evaluate(() => window.__calls.filter(([c]) => c === 'pairing_snapshot').length);

    // 1. From the snapshot: 82%, not charging.
    assert.deepEqual(await read(), { text: '82%', label: 'Bateria telefonu 82%', low: false, charging: false, hidden: false, width: 16 * 0.82 });
    assert.equal(await box.getAttribute('role'), 'img');
    assert.equal(await box.evaluate(b => b.closest('[aria-live],[role="status"]')), null, 'no announcement on each change');
    const before = await snapshots();

    // 2. Pushed changes (no IPC call): charging, then low, then full.
    await emit(state({ level: 81, charging: true }));
    assert.deepEqual(await read(), { text: '81%', label: 'Bateria telefonu 81%, ładuje się', low: false, charging: true, hidden: false, width: 16 * 0.81 });
    await emit(state({ level: 15, charging: false }));
    const low = await read();
    assert.equal(low.low, true); assert.equal(low.text, '15%');
    await page.screenshot({ path: path.join(out, 'panel-battery-low.png'), clip: { x: 0, y: 0, width: 380, height: 120 } });
    await emit(state({ level: 15, charging: true }));
    assert.equal((await read()).low, false, 'charging at 15%: not low');
    await emit(state({ level: 100, charging: true }));
    assert.equal((await read()).text, '100%');
    assert.equal(await snapshots(), before, 'updates come pushed, never by asking');

    // 3. No value or no session: hidden.
    await emit(state(null));
    assert.equal((await read()).hidden, true);
    await emit({ ...state({ level: 50, charging: false }), phase: 'offline', connected: false });
    assert.equal((await read()).hidden, true, 'offline hides a stale value');

    // 4. Themes and 150% text with a long phone name: nothing overflows, the level stays whole.
    await emit(state({ level: 9, charging: false }));
    for (const theme of ['light', 'dark']) {
      for (const text of ['100%', '150%']) {
        await page.evaluate(([t, f]) => { document.documentElement.dataset.theme = t; document.documentElement.style.fontSize = f; }, [theme, text]);
        await page.waitForTimeout(200);
        assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, `overflow ${theme} ${text}`);
        const r = await box.boundingBox();
        assert(r && r.x + r.width <= 380, `battery inside the panel ${theme} ${text}`);
        await page.screenshot({ path: path.join(out, `panel-battery-${theme}-${text.replace('%', '')}.png`), clip: { x: 0, y: 0, width: 380, height: 120 } });
      }
    }
    assert.deepEqual(errors, []);
    console.log(JSON.stringify({ batteryUi: true, pushedNoPolling: true, lowAndCharging: true, hiddenOffline: true, themes: 2, artifacts: out }));
  } finally {
    await browser.close();
    server.close();
  }
})().catch(e => { console.error(e); process.exitCode = 1; });
