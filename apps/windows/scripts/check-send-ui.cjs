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
      { id: '0'.repeat(32), name: 'Holidays 2026/IMG_2041 a very long file name from the holidays by the sea.jpg', size: 6 * MB, done: 2 * MB, state: 'running', reason: null },
      { id: '1'.repeat(32), name: 'Presentation.pptx', size: 880 * MB, done: 300 * MB, state: 'running', reason: null },
      { id: '2'.repeat(32), name: 'notes.txt', size: 14 * MB, done: 0, state: 'queued', reason: null },
      { id: '3'.repeat(32), name: 'film.mkv', size: 4000 * MB, done: 0, state: 'failed', reason: 'No space on the phone' },
      { id: '4'.repeat(32), name: 'a.txt', size: 5, done: 5, state: 'done', reason: null },
    ],
  };
  files.rows.forEach(r => { r.direction = 'outgoing'; r.canRetry = ['failed','cancelled'].includes(r.state); r.canOpen = true; r.rate = r.state === 'running' ? 2000000 : null; });
  window.__files = files;
  window.__calls = [];
  window.__listeners = {};
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => {
      window.__calls.push([cmd, args]);
      if (cmd === 'pairing_snapshot') return { phase: 'connected', phone: 'Galaxy S25 Ultra', qr: null, message: 'Connected · Wi-Fi', skipped_records: 0 };
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
    const page = await browser.newPage({ locale: 'en-US', viewport: { width: 380, height: 540 } });
    const errors = [];
    page.on('pageerror', e => errors.push(e.message));
    await page.addInitScript(stub);
    await page.goto(url);
    await page.locator('#transfers:not([hidden])').waitFor();
    await page.evaluate(() => document.fonts.ready);
    assert.equal(await page.locator('#transfers-list li').count(), 5, JSON.stringify({errors,html:await page.locator('#transfers-list').innerHTML()}));
    assert.match(await page.locator('#transfers-title').textContent(), /File transfers/);
    assert.match(await page.locator('#transfers-summary').textContent(), /3 in progress/);
    await page.getByRole('button', { name: 'Cancel: Presentation.pptx' }).focus();
    await page.evaluate(() => { window.__focusedTransfer = document.activeElement; for(let i=0;i<100;i++){window.__files.rows[0].done++;window.__listeners['files-state']({payload:window.__files});} });
    assert(await page.evaluate(() => document.activeElement === window.__focusedTransfer), 'progress updates preserve keyboard focus');
    await page.evaluate(() => { const fonts=[...document.querySelectorAll('body *')].map(e=>[e,parseFloat(getComputedStyle(e).fontSize)]); for(const [element,pixels] of fonts) element.style.fontSize=(pixels*1.5)+'px'; });
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
    await page.evaluate(() => window.__listeners['files-notice']({ payload: { message: 'Sending to Galaxy S25 Ultra: 3 files', error: false } }));
    assert.equal(await page.locator('#files-notice').isVisible(), true);
    await page.locator('#send-files').focus();
    await page.keyboard.press('Enter');
    await page.getByRole('button', { name: 'Cancel: Presentation.pptx' }).click();
    await page.getByRole('button', { name: 'Retry: film.mkv' }).click();
    await page.locator('#transfers-clear').click();
    const calls = await page.evaluate(() => window.__calls.filter(c => c[0] === 'files_action').map(c => [c[1].action, c[1].id || null]));
    assert.deepEqual(calls, [['pick', null], ['cancel', '1'.repeat(32)], ['retry', '3'.repeat(32)], ['clear', null]]);
    // WN-156: five finished show the newest three and "Show all (5)"; the toggle shows five, then collapses; with
    // nothing left to collapse the toggle goes away.
    await page.evaluate(() => { const rows = [...'abcde'].map(c => ({ id: c.repeat(32), direction: 'incoming', name: `${c}.txt`, size: 5, done: 5, state: 'done', reason: null, canOpen: true })); window.__listeners['files-state']({ payload: { rows } }); });
    const more = page.locator('#transfers-more');
    assert.equal(await page.locator('#transfers-list li').count(), 3);
    assert.equal(await more.textContent(), 'Show all (5)');
    assert.equal(await more.getAttribute('aria-expanded'), 'false');
    assert.equal(await more.getAttribute('aria-controls'), 'transfers-list');
    await more.click();
    assert.equal(await page.locator('#transfers-list li').count(), 5);
    assert.equal(await more.getAttribute('aria-expanded'), 'true');
    assert.equal(await more.textContent(), 'Show less');
    await more.click();
    assert.equal(await page.locator('#transfers-list li').count(), 3);
    await more.click();
    await page.evaluate(() => window.__listeners['files-state']({ payload: { rows: [{ id: 'f'.repeat(32), direction: 'incoming', name: 'f.txt', size: 5, done: 5, state: 'done', reason: null, canOpen: true }] } }));
    assert.equal(await more.isVisible(), false, 'no "Show less" with nothing to collapse');
    assert.equal(await more.getAttribute('aria-expanded'), 'false');
    await page.evaluate(() => {const row={id:'5'.repeat(32),direction:'incoming',name:'<img src=x onerror=alert(1)>',size:null,done:123,state:'running',rate:null};window.__listeners['files-state']({payload:{rows:[row]}});});
    assert.equal(await page.locator('#transfers-list img').count(),0);
    assert.equal(await page.locator('#transfers-list [role=progressbar]').getAttribute('aria-valuenow'),null);
    assert.match(await page.locator('#transfers-list small').textContent(),/123 B/);
    assert.equal(await page.getByRole('button',{name:/Retry:/}).count(),0);
    await page.evaluate(() => window.__listeners['files-state']({ payload: { phone: 'Galaxy S25 Ultra', active: 0, activeSize: 0, activeDone: 0, rows: [] } }));
    assert.equal(await page.locator('#transfers').isVisible(), false);
    assert.deepEqual(errors, []);
    console.log(`PASS: panel sending, themes, drop zone, notice, keyboard, actions; screenshots: ${out}`);
  } finally { await browser.close(); server.close(); }
})().catch(e => { console.error(e); process.exit(1); });
