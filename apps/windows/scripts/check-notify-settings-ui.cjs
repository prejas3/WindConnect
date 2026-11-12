// CCS/1 PURPOSE:WN-43 main-window switch "Powiadomienia z telefonu" with a stubbed IPC: shown from pairing-state,
// reflects phone_notifications, a click (and Space) sends phone_notifications, pushed changes follow, Light/Dark | DEP:external dev-only Playwright 1.61.1.
const assert = require('node:assert/strict');
const http = require('node:http');
const fs = require('node:fs/promises');
const path = require('node:path');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const root = path.resolve(__dirname, '../frontend');
const out = process.env.UI_ARTIFACTS || path.join(require('node:os').tmpdir(), 'windconnect-wn43-ui');
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.ttf': 'font/ttf' };
const server = http.createServer(async (req, res) => {
  const p = new URL(req.url, 'http://x').pathname;
  const f = path.resolve(root, '.' + (p === '/' ? '/index.html' : p));
  if (!f.startsWith(root + path.sep)) { res.writeHead(403).end(); return; }
  try { res.setHeader('Content-Type', types[path.extname(f)] || 'text/plain'); res.end(await fs.readFile(f)); }
  catch { res.writeHead(404).end(); }
});
const phone = { key: 'a'.repeat(64), name: 'Pixel 9', fingerprint: 'K4TQ-9WZM-2HXP', paired_at: 1794300000, connected: true, owed: false };
const state = on => ({ phase: 'connected', phone: 'Pixel 9', qr: null, message: 'Połączono · Wi-Fi', skipped_records: 0, code: null, connected: true, devices: [phone], paired: true, battery: null, find: { available: false, state: 'idle', last: null, ring_ms: null }, media: null, share_media: true, phone_notifications: on });
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
    const page = await browser.newPage({ viewport: { width: 920, height: 900 } });
    const errors = [];
    page.on('pageerror', e => errors.push(e.message));
    await page.addInitScript(stub, state(true));
    await page.goto(`http://127.0.0.1:${server.address().port}/index.html`);
    const toggle = page.getByRole('switch', { name: 'Powiadomienia z telefonu' });
    await page.locator('#notify-settings:not([hidden])').waitFor();
    assert.equal(await toggle.getAttribute('aria-checked'), 'true');
    assert.equal(await toggle.isDisabled(), false);
    await toggle.click();
    assert.equal(await toggle.getAttribute('aria-checked'), 'false');
    await toggle.focus(); await page.keyboard.press('Space');
    const sent = await page.evaluate(() => window.__calls.filter(([c]) => c === 'phone_notifications').map(([, a]) => a.enabled));
    assert.deepEqual(sent, [false, true]);
    await page.evaluate(v => window.__listeners['pairing-state'].forEach(cb => cb({ payload: v })), state(false));
    assert.equal(await toggle.getAttribute('aria-checked'), 'false', 'follows the pushed state');
    for (const theme of ['light', 'dark']) {
      await page.evaluate(t => { document.documentElement.dataset.theme = t; }, theme);
      await page.locator('#notify-settings').screenshot({ path: path.join(out, `notify-settings-${theme}.png`) });
    }
    assert.deepEqual(errors, []);
    console.log(JSON.stringify({ notifySettingsUi: true, clicks: sent, themes: 2, artifacts: out }));
  } finally {
    await browser.close();
    server.close();
  }
})().catch(e => { console.error(e); process.exitCode = 1; });
