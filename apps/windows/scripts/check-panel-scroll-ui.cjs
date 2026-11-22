// CCS/1 PURPOSE:WN-145 the panel opens at the top (it used to keep its last scroll and could open mid-way, looking cut off
// under a top taskbar), and a pairing QR, near the bottom, is brought into view: when it appears and when the panel opens
// during a pairing. Stubbed IPC in headless Chromium, no app or screen needed. DEP:external dev-only Playwright.
const assert = require('node:assert/strict');
const http = require('node:http');
const fs = require('node:fs/promises');
const path = require('node:path');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const root = path.resolve(__dirname, '../frontend');
const out = process.env.UI_ARTIFACTS || path.join(require('node:os').tmpdir(), 'windconnect-wn145-ui');
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.ttf': 'font/ttf' };
const server = http.createServer(async (req, res) => {
  const p = new URL(req.url, 'http://x').pathname;
  const f = path.resolve(root, '.' + p);
  if (!f.startsWith(root + path.sep)) { res.writeHead(403).end(); return; }
  try { res.setHeader('Content-Type', types[path.extname(f)] || 'text/plain'); res.end(await fs.readFile(f)); }
  catch { res.writeHead(404).end(); }
});
// Enough phones for the panel to scroll in its 540 px window.
const devices = Array.from({ length: 8 }, (_, i) => ({ key: String(i).repeat(64).slice(0, 64), name: `Telefon ${i + 1}`, fingerprint: 'K4TQ-9WZM-2HXP', paired_at: 1794300000 + i, connected: i === 0, owed: false }));
const connected = { phase: 'connected', phone: 'Telefon 1', qr: null, message: '', skipped_records: 0, code: null, connected: true, devices };
const width = 21, qr = { width, dark: Array.from({ length: width * width }, (_, i) => (i * 7) % 3 === 0) };
const waiting = { ...connected, phase: 'waiting', connected: true, qr };
const stub = state => {
  window.__state = state;
  window.__listeners = {};
  window.__TAURI__ = {
    core: { invoke: async cmd => {
      if (cmd === 'pairing_snapshot') return structuredClone(window.__state);
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
    const page = await browser.newPage({ locale: 'en-US', viewport: { width: 380, height: 540 } });
    const errors = [];
    page.on('pageerror', e => errors.push(e.message));
    await page.addInitScript(stub, connected);
    await page.goto(`http://127.0.0.1:${server.address().port}/panel.html`);
    await page.locator('#devices:not([hidden])').waitFor();
    await page.waitForFunction(() => window.__listeners['panel-opened']);
    const top = () => page.evaluate(() => Math.max(document.scrollingElement.scrollTop, document.body.scrollTop));
    const open = async state => {
      await page.evaluate(s => { window.__state = s; return window.__listeners['panel-opened']({ payload: 1 }); }, state);
      await page.waitForTimeout(150);
    };
    const pairingInView = () => page.evaluate(() => {
      const r = document.querySelector('#pairing-qr').closest('section').getBoundingClientRect();
      return r.top >= 0 && r.top < innerHeight;
    });
    const max = await page.evaluate(() => document.scrollingElement.scrollHeight - innerHeight);
    assert(max > 200, `the panel scrolls (${max} px)`);
    // Left scrolled to the bottom, then opened again: it starts at the top.
    await page.evaluate(() => { document.scrollingElement.scrollTop = 1e6; });
    assert((await top()) > 200);
    await open(connected);
    assert.equal(await top(), 0, 'opens at the top');
    await page.screenshot({ path: path.join(out, 'panel-opened-top.png') });
    // "Add a phone" with the panel open: the QR appears and is brought into view.
    await page.evaluate(s => window.__listeners['pairing-state']({ payload: s }), waiting);
    await page.waitForTimeout(150);
    assert.equal(await page.locator('#pairing-qr').isHidden(), false, 'QR shown');
    assert(await pairingInView(), 'the QR card in view when it appears');
    // Scrolled away and opened again during the pairing: the QR card, not the top.
    await page.evaluate(() => { document.scrollingElement.scrollTop = 0; });
    await open(waiting);
    assert(await pairingInView(), 'the QR card in view when opened during a pairing');
    await page.screenshot({ path: path.join(out, 'panel-opened-pairing.png') });
    // Pairing over: back to the top on the next opening.
    await page.evaluate(s => window.__listeners['pairing-state']({ payload: s }), connected);
    await open(connected);
    assert.equal(await top(), 0, 'top again after the pairing');
    assert.deepEqual(errors, []);
    console.log(JSON.stringify({ scrollable: max, opensAtTop: true, qrInViewOnAppear: true, qrInViewOnOpen: true, artifacts: out }));
  } finally {
    await browser.close();
    server.close();
  }
})().catch(e => { console.error(e); process.exitCode = 1; });
