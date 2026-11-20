// CCS/1 PURPOSE:WN-45 main-window "Quiet hours" with a stubbed IPC: the switch shows the times, steppers and keys move by
// 15 min / 1 h and wrap at midnight, each change sends notify_quiet_hours, pushed state follows, Light/Dark
// | DEP:external dev-only Playwright 1.61.1.
const assert = require('node:assert/strict');
const http = require('node:http');
const fs = require('node:fs/promises');
const path = require('node:path');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const root = path.resolve(__dirname, '../frontend');
const out = process.env.UI_ARTIFACTS || path.join(require('node:os').tmpdir(), 'windconnect-wn45-ui');
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.ttf': 'font/ttf' };
const server = http.createServer(async (req, res) => {
  const p = new URL(req.url, 'http://x').pathname;
  const f = path.resolve(root, '.' + (p === '/' ? '/index.html' : p));
  if (!f.startsWith(root + path.sep)) { res.writeHead(403).end(); return; }
  try { res.setHeader('Content-Type', types[path.extname(f)] || 'text/plain'); res.end(await fs.readFile(f)); }
  catch { res.writeHead(404).end(); }
});
const phone = { key: 'a'.repeat(64), name: 'Pixel 9', fingerprint: 'K4TQ-9WZM-2HXP', paired_at: 1794300000, connected: true, owed: false };
const state = on => ({ phase: 'connected', phone: 'Pixel 9', qr: null, message: 'Połączono · Wi-Fi', skipped_records: 0, code: null, connected: true, devices: [phone], paired: true, battery: null, find: { available: false, state: 'idle', last: null, ring_ms: null }, media: null, share_media: true, phone_notifications: true, quiet_hours: on });
const stub = first => {
  window.__calls = [];
  window.__listeners = {};
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => {
      window.__calls.push([cmd, args]);
      if (cmd === 'pairing_snapshot') return structuredClone(first);
      if (cmd === 'actions_snapshot') return { builtins: [], custom: [], last: null, problems: [] };
      if (cmd === 'clipboard_snapshot') return { enabled: true, status: '', shortcut_error: false };
      if (cmd === 'autostart_enabled') return false;
      if (cmd === 'core_version') return '0.49.0';
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
    const page = await browser.newPage({ locale: 'pl-PL', viewport: { width: 920, height: 900 } });
    const errors = [];
    page.on('pageerror', e => errors.push(e.message));
    await page.addInitScript(stub, state({ on: false, from: 1320, to: 420 }));
    await page.goto(`http://127.0.0.1:${server.address().port}/index.html`);
    const toggle = page.getByRole('switch', { name: 'Ciche godziny' });
    await page.locator('#notify-settings:not([hidden])').waitFor();
    assert.equal(await toggle.getAttribute('aria-checked'), 'false');
    assert.equal(await page.locator('#quiet-times').isHidden(), true);
    const sent = () => page.evaluate(() => window.__calls.filter(([c]) => c === 'notify_quiet_hours').map(([, a]) => a));
    await toggle.click();
    assert.deepEqual((await sent()).at(-1), { on: true, from: 1320, to: 420 });
    const from = page.getByRole('spinbutton', { name: 'Od' });
    const to = page.getByRole('spinbutton', { name: 'Do' });
    assert.equal(await from.textContent(), '22:00');
    assert.equal(await to.textContent(), '07:00');
    await page.getByRole('button', { name: 'Później o 15 minut (od)' }).click();
    assert.deepEqual((await sent()).at(-1), { on: true, from: 1335, to: 420 });
    await to.focus();
    await page.keyboard.press('PageUp');
    assert.deepEqual((await sent()).at(-1), { on: true, from: 1335, to: 480 });
    assert.equal(await to.getAttribute('aria-valuetext'), '08:00');
    // Wraps at midnight.
    await page.evaluate(v => window.__listeners['pairing-state'].forEach(cb => cb({ payload: v })), state({ on: true, from: 0, to: 1425 }));
    await from.focus(); await page.keyboard.press('ArrowDown');
    assert.deepEqual((await sent()).at(-1), { on: true, from: 1425, to: 1425 });
    await to.focus(); await page.keyboard.press('ArrowUp');
    assert.deepEqual((await sent()).at(-1), { on: true, from: 1425, to: 0 });
    // Pushed state follows.
    await page.evaluate(v => window.__listeners['pairing-state'].forEach(cb => cb({ payload: v })), state({ on: true, from: 1380, to: 360 }));
    assert.equal(await from.textContent(), '23:00');
    assert.equal(await to.textContent(), '06:00');
    for (const theme of ['light', 'dark']) {
      await page.evaluate(t => { document.documentElement.dataset.theme = t; }, theme);
      await page.locator('#notify-settings').screenshot({ path: path.join(out, `notify-quiet-${theme}.png`) });
    }
    await page.evaluate(v => window.__listeners['pairing-state'].forEach(cb => cb({ payload: v })), state({ on: false, from: 1380, to: 360 }));
    assert.equal(await page.locator('#quiet-times').isHidden(), true);
    assert.deepEqual(errors, []);
    console.log(JSON.stringify({ notifyQuietUi: true, calls: (await sent()).length, themes: 2, artifacts: out }));
  } finally {
    await browser.close();
    server.close();
  }
})().catch(e => { console.error(e); process.exitCode = 1; });
