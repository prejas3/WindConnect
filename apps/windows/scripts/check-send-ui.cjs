// CCS/1 PURPOSE:WN-39 panel sending UI with a stubbed IPC (themes, transfers, drop zone, notice, keyboard) | DEP:external dev-only Playwright 1.61.1.
const assert = require('node:assert/strict');
const http = require('node:http');
const fs = require('node:fs/promises');
const path = require('node:path');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const root = path.resolve(__dirname, '../frontend');
const out = process.env.UI_ARTIFACTS || path.join(require('node:os').tmpdir(), 'windconnect-wn39-ui');
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.ttf': 'font/ttf' };
const server = http.createServer(async (req, res) => {
  const p = new URL(req.url, 'http://x').pathname;
  const f = path.resolve(root, '.' + (p === '/' ? '/panel.html' : p));
  if (!f.startsWith(root + path.sep)) { res.writeHead(403).end(); return; }
  try { res.setHeader('Content-Type', types[path.extname(f)] || 'text/plain'); res.end(await fs.readFile(f)); }
  catch { res.writeHead(404).end(); }
});
const stub = () => {
  const MB = 1024 * 1024;
  const files = {
    phone: 'Galaxy S25 Ultra', active: 3, activeSize: 900 * MB, activeDone: 310 * MB,
    rows: [
      { id: '0'.repeat(32), name: 'Wakacje 2026/IMG_2041 bardzo długa nazwa pliku z wakacji nad morzem.jpg', size: 6 * MB, done: 2 * MB, state: 'running', reason: null },
      { id: '1'.repeat(32), name: 'Prezentacja.pptx', size: 880 * MB, done: 300 * MB, state: 'running', reason: null },
      { id: '2'.repeat(32), name: 'notatki.txt', size: 14 * MB, done: 0, state: 'queued', reason: null },
      { id: '3'.repeat(32), name: 'film.mkv', size: 4000 * MB, done: 0, state: 'failed', reason: 'Brak miejsca na telefonie' },
      { id: '4'.repeat(32), name: 'a.txt', size: 5, done: 5, state: 'done', reason: null },
    ],
  };
  window.__calls = [];
  window.__listeners = {};
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => {
      window.__calls.push([cmd, args]);
      if (cmd === 'pairing_snapshot') return { phase: 'connected', phone: 'Galaxy S25 Ultra', qr: null, message: 'Połączono · Wi-Fi', skipped_records: 0 };
      if (cmd === 'files_snapshot') return files;
      if (cmd === 'panel_snapshot') return { shortcut_error: null };
      return null;
    } },
    event: { listen: async (name, cb) => { window.__listeners[name] = cb; return () => {}; } },
  };
};
(async () => {
  await fs.mkdir(out, { recursive: true });
  await new Promise(r => server.listen(0, '127.0.0.1', r));
  const url = `http://127.0.0.1:${server.address().port}/panel.html`;
  const browser = await chromium.launch({ headless: true });
  try {
    const page = await browser.newPage({ viewport: { width: 380, height: 540 } });
    const errors = [];
    page.on('pageerror', e => errors.push(e.message));
    await page.addInitScript(stub);
    await page.goto(url);
    await page.locator('#transfers:not([hidden])').waitFor();
    await page.evaluate(() => document.fonts.ready);
    assert.equal(await page.locator('#transfers-list li').count(), 4);
    assert.match(await page.locator('#transfers-title').textContent(), /Wysyłanie na Galaxy S25 Ultra/);
    assert.match(await page.locator('#transfers-summary').textContent(), /3 w toku · 310 MB z 900 MB/);
    for (const theme of ['light', 'dark']) {
      await page.evaluate(t => { document.documentElement.dataset.theme = t; }, theme);
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, `overflow ${theme}`);
      await page.screenshot({ path: path.join(out, `panel-send-${theme}.png`) });
      await page.evaluate(() => window.__listeners['files-drag']({ payload: true }));
      assert.equal(await page.locator('#drop-zone').isVisible(), true);
      assert.match(await page.locator('#drop-title').textContent(), /Galaxy S25 Ultra/);
      await page.screenshot({ path: path.join(out, `panel-drop-${theme}.png`) });
      await page.evaluate(() => window.__listeners['files-drag']({ payload: false }));
      assert.equal(await page.locator('#drop-zone').isVisible(), false);
    }
    await page.evaluate(() => window.__listeners['files-notice']({ payload: { message: 'Wysyłanie na Galaxy S25 Ultra: 3 pliki', error: false } }));
    assert.equal(await page.locator('#files-notice').isVisible(), true);
    await page.locator('#send-files').focus();
    await page.keyboard.press('Enter');
    await page.getByRole('button', { name: 'Anuluj: Prezentacja.pptx' }).click();
    await page.getByRole('button', { name: 'Ponów: film.mkv' }).click();
    await page.locator('#transfers-clear').click();
    const calls = await page.evaluate(() => window.__calls.filter(c => c[0] === 'files_action').map(c => [c[1].action, c[1].id || null]));
    assert.deepEqual(calls, [['pick', null], ['cancel', '1'.repeat(32)], ['retry', '3'.repeat(32)], ['clear', null]]);
    await page.evaluate(() => window.__listeners['files-state']({ payload: { phone: 'Galaxy S25 Ultra', active: 0, activeSize: 0, activeDone: 0, rows: [] } }));
    assert.equal(await page.locator('#transfers').isVisible(), false);
    assert.deepEqual(errors, []);
    console.log(`PASS: panel sending, themes, drop zone, notice, keyboard, actions; screenshots: ${out}`);
  } finally { await browser.close(); server.close(); }
})().catch(e => { console.error(e); process.exit(1); });
