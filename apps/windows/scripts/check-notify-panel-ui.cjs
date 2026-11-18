// CCS/1 PURPOSE:WN-44 the panel's "Powiadomienia" with a stubbed IPC: rows from pairing-state (text only), Enter sends
// notify_action reply, the field keeps its text and focus across pushes, clears only after Replied ok, keeps it after a failure,
// blocks > 1024 bytes without trimming, a toast's too-long draft lands in the field, ✕ sends dismiss, Light/Dark
// | DEP:external dev-only Playwright 1.61.1.
const assert = require('node:assert/strict');
const http = require('node:http');
const fs = require('node:fs/promises');
const path = require('node:path');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const root = path.resolve(__dirname, '../frontend');
const out = process.env.UI_ARTIFACTS || path.join(require('node:os').tmpdir(), 'windconnect-wn44-ui');
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.ttf': 'font/ttf' };
const server = http.createServer(async (req, res) => {
  const p = new URL(req.url, 'http://x').pathname;
  const f = path.resolve(root, '.' + (p === '/' ? '/panel.html' : p));
  if (!f.startsWith(root + path.sep)) { res.writeHead(403).end(); return; }
  try { res.setHeader('Content-Type', types[path.extname(f)] || 'text/plain'); res.end(await fs.readFile(f)); }
  catch { res.writeHead(404).end(); }
});
const A = 'ab'.repeat(16), B = 'cd'.repeat(16);
const row = (tag, extra = {}) => ({ tag, app: 'Signal', title: 'Ola', text: 'Będziesz o 19?', reply: 'Odpowiedz', status: null, draft: null, ...extra });
const state = notifications => ({ phase: 'connected', phone: 'Pixel 9', qr: null, message: 'Połączono · Wi-Fi', skipped_records: 0, code: null, connected: true, devices: [], paired: true, battery: null, find: { available: false, state: 'idle', last: null, ring_ms: null }, media: null, share_media: true, phone_notifications: true, notifications });
const stub = first => {
  window.__calls = [];
  window.__listeners = {};
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => {
      window.__calls.push([cmd, args]);
      if (cmd === 'pairing_snapshot') return structuredClone(first);
      if (cmd === 'files_snapshot') return { rows: [], active: 0, activeDone: 0, activeSize: 0, phone: null };
      if (cmd === 'panel_snapshot') return { shortcut_error: false };
      if (cmd === 'clipboard_snapshot') return { enabled: true, status: '', shortcut_error: false };
      return null;
    } },
    event: { listen: async (name, cb) => { (window.__listeners[name] ||= []).push(cb); return () => {}; } },
  };
};
(async () => {
  await fs.mkdir(out, { recursive: true });
  await new Promise(r => server.listen(0, '127.0.0.1', r));
  const browser = await chromium.launch({ headless: true });
  try {
    const page = await browser.newPage({ locale: 'pl-PL', viewport: { width: 380, height: 760 } });
    const errors = [];
    page.on('pageerror', e => errors.push(e.message));
    const hostile = row(B, { app: '<img src=x onerror=alert(1)>', title: '<b>x</b>', reply: null });
    await page.addInitScript(stub, state([row(A), hostile]));
    await page.goto(`http://127.0.0.1:${server.address().port}/panel.html`);
    const push = list => page.evaluate(v => window.__listeners['pairing-state'].forEach(cb => cb({ payload: v })), state(list));
    const calls = () => page.evaluate(() => window.__calls.filter(([c]) => c === 'notify_action').map(([, a]) => a));
    await page.locator('#notifications:not([hidden])').waitFor();
    assert.equal(await page.locator('#notification-list li').count(), 2);
    assert.equal(await page.locator('#notification-list img').count(), 0, 'text only');
    assert.equal(await page.locator('#notification-list li').nth(1).locator('.name').textContent(), '<b>x</b>');
    assert.equal(await page.locator('#notification-list li').nth(1).locator('input').count(), 0, 'no reply action: no field');
    const field = page.getByRole('textbox', { name: 'Odpowiedz: Ola' });
    // Typing survives a push, focus too.
    await field.fill('Tak, o 19');
    await push([row(A, { text: 'Będziesz o 19? Daj znać' }), hostile]);
    assert.equal(await field.inputValue(), 'Tak, o 19');
    assert.equal(await field.evaluate(e => e === document.activeElement), true, 'focus kept');
    await field.press('Enter');
    assert.deepEqual((await calls()).at(-1), { action: 'reply', tag: A, text: 'Tak, o 19' });
    // A stale 'sent' doesn't clear it; 'sending' then 'sent' does.
    await push([row(A, { status: 'sent' }), hostile]);
    assert.equal(await field.inputValue(), 'Tak, o 19');
    await push([row(A, { status: 'sending' }), hostile]);
    assert.equal(await page.locator('.notification-status').first().textContent(), 'Wysyłanie…');
    await push([row(A, { status: 'sent' }), hostile]);
    assert.equal(await field.inputValue(), '', 'cleared after Replied ok');
    assert.equal(await page.locator('.notification-status').first().textContent(), 'Wysłano');
    // A failure keeps the text.
    await field.fill('Druga');
    await page.getByRole('button', { name: 'Wyślij odpowiedź: Ola' }).click();
    await push([row(A, { status: 'sending' }), hostile]);
    await push([row(A, { status: 'failed' }), hostile]);
    assert.equal(await field.inputValue(), 'Druga', 'kept after Replied ok=false');
    assert.match(await page.locator('.notification-status').first().textContent(), /Nie udało się/);
    // Over 1024 bytes: blocked, not trimmed, nothing sent.
    const before = (await calls()).length;
    const long = 'ą'.repeat(513);
    await field.fill(long);
    await field.press('Enter');
    assert.equal((await calls()).length, before, 'not sent');
    assert.equal(await field.inputValue(), long, 'not trimmed');
    assert.equal(await field.getAttribute('aria-invalid'), 'true');
    assert.match(await page.locator('.notification-status').first().textContent(), /za długa \(1026 z 1024/);
    for (const theme of ['light', 'dark']) {
      await page.evaluate(t => { document.documentElement.dataset.theme = t; }, theme);
      await page.locator('#notifications').screenshot({ path: path.join(out, `notify-panel-${theme}.png`) });
    }
    // A toast's too-long reply comes as a draft into an empty field.
    await field.fill('');
    await push([row(A, { status: 'too-long', draft: 'x'.repeat(1100) }), hostile]);
    assert.equal(await field.inputValue(), 'x'.repeat(1100));
    // ✕: dismiss; the row goes when the PC says so.
    await page.getByRole('button', { name: 'Odrzuć powiadomienie: <b>x</b>' }).click();
    assert.deepEqual((await calls()).at(-1), { action: 'dismiss', tag: B });
    await push([row(A)]);
    assert.equal(await page.locator('#notification-list li').count(), 1);
    await push([]);
    assert.equal(await page.locator('#notifications').isHidden(), true);
    assert.deepEqual(errors, []);
    console.log(JSON.stringify({ notifyPanelUi: true, calls: (await calls()).map(c => c.action), themes: 2, artifacts: out }));
  } finally {
    await browser.close();
    server.close();
  }
})().catch(e => { console.error(e); process.exitCode = 1; });
