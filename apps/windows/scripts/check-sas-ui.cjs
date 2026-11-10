// CCS/1 PURPOSE:WN-27 panel pairing by code (SAS) with a stubbed IPC: entry, waiting, code, decisions, themes | DEP:external dev-only Playwright 1.61.1.
const assert = require('node:assert/strict');
const http = require('node:http');
const fs = require('node:fs/promises');
const path = require('node:path');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const root = path.resolve(__dirname, '../frontend');
const out = process.env.UI_ARTIFACTS || path.join(require('node:os').tmpdir(), 'windconnect-wn27-ui');
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.ttf': 'font/ttf' };
const server = http.createServer(async (req, res) => {
  const p = new URL(req.url, 'http://x').pathname;
  const f = path.resolve(root, '.' + p);
  if (!f.startsWith(root + path.sep)) { res.writeHead(403).end(); return; }
  try { res.setHeader('Content-Type', types[path.extname(f)] || 'text/plain'); res.end(await fs.readFile(f)); }
  catch { res.writeHead(404).end(); }
});
const stub = () => {
  window.__calls = [];
  window.__listeners = {};
  const qr = { width: 21, dark: Array.from({ length: 441 }, (_, i) => (i * 7) % 3 === 0) };
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => {
      window.__calls.push([cmd, args]);
      if (cmd === 'pairing_snapshot') return { phase: 'waiting', phone: null, qr, message: 'Zeskanuj kod w WindConnect na telefonie', skipped_records: 0, code: null };
      if (cmd === 'files_snapshot') return { phone: null, rows: [], active: 0, activeSize: 0, activeDone: 0 };
      if (cmd === 'panel_snapshot') return { shortcut_error: null };
      return null;
    } },
    event: { listen: async (name, cb) => { window.__listeners[name] = cb; return () => {}; } },
  };
};
const state = (phase, extra = {}) => ({ phase, phone: null, qr: null, message: '', skipped_records: 0, code: null, ...extra });
(async () => {
  await fs.mkdir(out, { recursive: true });
  await new Promise(r => server.listen(0, '127.0.0.1', r));
  const browser = await chromium.launch({ headless: true });
  try {
    const page = await browser.newPage({ viewport: { width: 380, height: 540 } });
    const errors = [];
    page.on('pageerror', e => errors.push(e.message));
    await page.addInitScript(stub);
    await page.goto(`http://127.0.0.1:${server.address().port}/panel.html`);
    await page.locator('#pairing-sas:not([hidden])').waitFor();
    await page.evaluate(() => document.fonts.ready);
    await page.getByRole('button', { name: 'Paruj bez aparatu' }).click();
    const emit = s => page.evaluate(v => window.__listeners['pairing-state']({ payload: v }), s);
    await emit(state('sas-waiting', { message: 'Na telefonie wybierz „Paruj bez aparatu”, a potem SZYMON-PC z listy.' }));
    assert.equal(await page.locator('#pairing-qr').isVisible(), false);
    assert.equal(await page.locator('#pairing-code').isVisible(), false);
    assert.equal(await page.locator('#pairing-refresh').textContent(), 'Wróć do kodu QR');
    assert.equal(await page.locator('#pairing-cancel').isVisible(), true);
    await emit(state('sas-code', { phone: 'Galaxy S25 Ultra', code: '042 917', message: 'Czy Galaxy S25 Ultra pokazuje ten sam kod?' }));
    assert.equal(await page.locator('#pairing-code').textContent(), '042 917');
    assert.equal(await page.locator('#pairing-refresh').isVisible(), false);
    for (const theme of ['light', 'dark']) {
      await page.evaluate(t => { document.documentElement.dataset.theme = t; }, theme);
      await page.waitForTimeout(400); // let colour transitions finish before the screenshot
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, `overflow ${theme}`);
      await page.screenshot({ path: path.join(out, `panel-sas-${theme}.png`) });
    }
    await page.locator('#pairing-accept').focus();
    await page.keyboard.press('Enter');
    await page.getByRole('button', { name: 'Kody się różnią' }).click();
    await emit(state('sas-confirming', { phone: 'Galaxy S25 Ultra', message: 'Czekam na potwierdzenie na telefonie…' }));
    assert.equal(await page.locator('#pairing-code').isVisible(), false);
    assert.equal(await page.locator('#pairing-accept').isVisible(), false);
    const calls = await page.evaluate(() => window.__calls.filter(c => c[0] === 'pairing_action').map(c => c[1].action));
    assert.deepEqual(calls, ['sas', 'sas-accept', 'sas-reject']);
    assert.deepEqual(errors, []);
    console.log(`PASS: SAS entry, waiting, code, accept/reject, confirming, themes; screenshots: ${out}`);
  } finally { await browser.close(); server.close(); }
})().catch(e => { console.error(e); process.exit(1); });
