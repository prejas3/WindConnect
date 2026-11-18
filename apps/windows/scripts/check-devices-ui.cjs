// CCS/1 PURPOSE:WC-28 panel device list with a stubbed IPC: rows, rename, unpair confirmation, disconnect, add, themes | DEP:external dev-only Playwright 1.61.1.
const assert = require('node:assert/strict');
const http = require('node:http');
const fs = require('node:fs/promises');
const path = require('node:path');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const root = path.resolve(__dirname, '../frontend');
const out = process.env.UI_ARTIFACTS || path.join(require('node:os').tmpdir(), 'windconnect-wc28-ui');
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.ttf': 'font/ttf' };
const server = http.createServer(async (req, res) => {
  const p = new URL(req.url, 'http://x').pathname;
  const f = path.resolve(root, '.' + p);
  if (!f.startsWith(root + path.sep)) { res.writeHead(403).end(); return; }
  try { res.setHeader('Content-Type', types[path.extname(f)] || 'text/plain'); res.end(await fs.readFile(f)); }
  catch { res.writeHead(404).end(); }
});
const KEY_A = 'a'.repeat(64), KEY_B = 'b'.repeat(64), KEY_C = 'c'.repeat(64);
const devices = [
  { key: KEY_A, name: 'Galaxy S25 Ultra', fingerprint: 'K4TQ-9WZM-2HXP', paired_at: 1794300000, connected: true, owed: false },
  { key: KEY_B, name: 'Pixel 9 służbowy z bardzo długą nazwą urządzenia', fingerprint: 'M7RC-1QDN-8LVA', paired_at: 1794390000, connected: false, owed: false },
  { key: KEY_C, name: 'Usunięty telefon', fingerprint: 'Z2PE-6YHT-4KUB', paired_at: 1794400000, connected: false, owed: true },
];
const stub = rows => {
  window.__calls = [];
  window.__listeners = {};
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => {
      window.__calls.push([cmd, args]);
      if (cmd === 'pairing_snapshot') return { phase: 'connected', phone: 'Galaxy S25 Ultra', qr: null, message: 'Połączono · Wi-Fi', skipped_records: 0, code: null, connected: true, devices: structuredClone(rows) };
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
    await page.addInitScript(stub, devices);
    await page.goto(`http://127.0.0.1:${server.address().port}/panel.html`);
    await page.locator('#devices:not([hidden])').waitFor();
    await page.evaluate(() => document.fonts.ready);
    const calls = () => page.evaluate(() => window.__calls.filter(([c]) => c === 'device_action' || c === 'pairing_action'));
    const rows = page.locator('#device-list > li');
    assert.equal(await rows.count(), 3);
    assert.match(await rows.nth(0).textContent(), /Połączony/);
    assert.match(await rows.nth(2).textContent(), /dowie się przy następnym połączeniu/);
    assert.equal(await rows.nth(2).locator('button').count(), 0, 'an owed row has no actions');
    assert.equal(await rows.nth(1).getByRole('button', { name: /^Rozłącz/ }).count(), 0, 'disconnect only for the connected phone');

    // Rename: Enter saves the trimmed name; an empty name is refused in place.
    await rows.nth(1).getByRole('button', { name: /^Zmień nazwę/ }).click();
    const input = rows.nth(1).locator('input');
    await input.fill('   ');
    await input.press('Enter');
    assert.equal(await input.getAttribute('aria-invalid'), 'true');
    await input.fill('  Telefon Oli ');
    // A state update while typing keeps what was typed.
    await page.evaluate(v => window.__listeners['pairing-state']({ payload: { phase: 'connected', phone: 'Galaxy S25 Ultra', qr: null, message: '', skipped_records: 0, code: null, connected: true, devices: v } }), devices);
    assert.equal(await rows.nth(1).locator('input').inputValue(), '  Telefon Oli ');
    await rows.nth(1).locator('input').press('Enter');
    assert.deepEqual((await calls()).at(-1), ['device_action', { action: 'rename', key: KEY_B, name: 'Telefon Oli' }]);

    // Unpair asks inline first; Cancel sends nothing.
    await rows.nth(0).getByRole('button', { name: /^Usuń parowanie/ }).click();
    assert.match(await rows.nth(0).locator('.confirm').textContent(), /Telefon też je usunie\./);
    for (const theme of ['light', 'dark']) {
      await page.evaluate(t => { document.documentElement.dataset.theme = t; }, theme);
      await page.waitForTimeout(400);
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, `overflow ${theme}`);
      await page.screenshot({ path: path.join(out, `panel-devices-${theme}.png`), fullPage: true });
    }
    const before = (await calls()).length;
    await rows.nth(0).getByRole('button', { name: /^Anuluj/ }).click();
    assert.equal((await calls()).length, before);
    await rows.nth(1).getByRole('button', { name: /^Usuń parowanie/ }).click();
    assert.match(await rows.nth(1).locator('.confirm').textContent(), /przy następnym połączeniu/);
    await rows.nth(1).getByRole('button', { name: /^Usuń parowanie: / }).last().click();
    assert.deepEqual((await calls()).at(-1), ['device_action', { action: 'unpair', key: KEY_B }]);

    await rows.nth(0).getByRole('button', { name: /^Rozłącz/ }).click();
    assert.deepEqual((await calls()).at(-1), ['device_action', { action: 'disconnect', key: KEY_A }]);
    await page.getByRole('button', { name: 'Dodaj telefon' }).click();
    assert.deepEqual((await calls()).at(-1), ['pairing_action', { action: 'refresh' }]);
    // While another phone is being paired, the connected one keeps its quick actions.
    await page.evaluate(v => window.__listeners['pairing-state']({ payload: { phase: 'waiting', phone: 'Galaxy S25 Ultra', qr: { width: 21, dark: Array(441).fill(false) }, message: 'Zeskanuj kod', skipped_records: 0, code: null, connected: true, devices: v } }), devices);
    assert.equal(await page.locator('#quick-actions').isVisible(), true);
    assert.equal(await page.locator('#pairing-qr').isVisible(), true);
    await page.screenshot({ path: path.join(out, 'panel-devices-adding.png'), fullPage: true });
    assert.deepEqual(errors, []);
    console.log(JSON.stringify({ devicesUi: true, rename: true, unpairConfirm: true, disconnect: true, add: true, themes: 2, artifacts: out }));
  } finally {
    await browser.close();
    server.close();
  }
})().catch(e => { console.error(e); process.exitCode = 1; });
