// CCS/1 PURPOSE:WC-34 first-run onboarding in the main window with a stubbed IPC: steps, QR rules (WN-94), SAS, expiry,
// "Done" with autostart, finish, focus order, Light/Dark, 150% text, no overflow | DEP:external dev-only Playwright 1.61.1.
const assert = require('node:assert/strict');
const http = require('node:http');
const fs = require('node:fs/promises');
const path = require('node:path');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const root = path.resolve(__dirname, '../frontend');
const out = process.env.UI_ARTIFACTS || path.join(require('node:os').tmpdir(), 'windconnect-wc34-ui');
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.ttf': 'font/ttf' };
const server = http.createServer(async (req, res) => {
  const p = new URL(req.url, 'http://x').pathname;
  const f = path.resolve(root, '.' + (p === '/' ? '/index.html' : p));
  if (!f.startsWith(root + path.sep)) { res.writeHead(403).end(); return; }
  try { res.setHeader('Content-Type', types[path.extname(f)] || 'text/plain'); res.end(await fs.readFile(f)); }
  catch { res.writeHead(404).end(); }
});
const LINK = 'windconnect://pair?secret=SHOULD-NEVER-BE-IN-THE-DOM';
const stub = (start = null) => {
  window.__calls = [];
  window.__listeners = {};
  window.__autostart = false;
  const qr = { width: 21, dark: Array.from({ length: 441 }, (_, i) => (i * 7) % 3 === 0) };
  window.__lastQr = null;
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => {
      window.__calls.push([cmd, args]);
      if (cmd === 'pairing_snapshot') { const q = structuredClone(qr); window.__lastQr = q; return { phase: 'waiting', phone: null, qr: q, message: 'Scan the code in WindConnect on the phone', skipped_records: 0, code: null, connected: false, devices: [], paired: false, ...start }; }
      if (cmd === 'autostart_enabled') return window.__autostart;
      if (cmd === 'set_autostart') { window.__autostart = args.enabled; return args.enabled; }
      if (cmd === 'actions_snapshot') return { builtins: [], custom: [], last: null, problems: [] };
      if (cmd === 'core_version') return '0.41.0';
      if (cmd === 'ui_dev_mode') return false;
      if (cmd === 'apply_window_theme') return false;
      if (cmd === 'clipboard_snapshot') return { enabled: true, status: '', shortcut_error: false };
      return null;
    } },
    event: { listen: async (name, cb) => { (window.__listeners[name] ||= []).push(cb); return () => {}; } },
  };
};
const base = { phone: null, qr: null, message: '', skipped_records: 0, code: null, connected: false, devices: [], paired: false };
const phone = { key: 'a'.repeat(64), name: 'Galaxy S25 Ultra', fingerprint: 'K4TQ-9WZM-2HXP', paired_at: 1794300000, connected: true, owed: false };
(async () => {
  await fs.mkdir(out, { recursive: true });
  await new Promise(r => server.listen(0, '127.0.0.1', r));
  const browser = await chromium.launch({ headless: true });
  try {
    const page = await browser.newPage({ locale: 'en-US', viewport: { width: 920, height: 720 } });
    const errors = [];
    page.on('pageerror', e => errors.push(e.message));
    await page.addInitScript(stub);
    await page.goto(`http://127.0.0.1:${server.address().port}/index.html`);
    await page.locator('#onboarding:not([hidden])').waitFor();
    await page.evaluate(() => document.fonts.ready);
    const emit = s => page.evaluate(v => window.__listeners['pairing-state'].forEach(cb => cb({ payload: v })), s);
    const calls = cmd => page.evaluate(c => window.__calls.filter(([x]) => x === c), cmd);

    // 1. Empty store: onboarding instead of the empty state, QR drawn, focus on the heading.
    assert.equal(await page.locator('#home').isVisible(), false, 'empty state replaced');
    const canvas = page.locator('#onboarding-qr');
    assert.equal(await canvas.isVisible(), true);
    assert.equal(await canvas.getAttribute('aria-label'), 'Pairing code');
    assert.equal(await page.evaluate(() => window.__lastQr.dark.every(v => v === false)), true, 'modules zeroed after drawing (WN-94)');
    assert.equal(await page.evaluate(l => document.documentElement.outerHTML.includes(l), LINK), false);
    assert.equal(await page.evaluate(() => document.activeElement?.id), 'onboarding-title');
    assert.match(await page.locator('.onboarding-steps > li.current').textContent(), /Scan this code/);
    assert.equal(await page.getByRole('button', { name: 'Pair without the camera' }).isVisible(), true);
    assert.equal(await page.locator('#actions').isVisible(), false, 'settings that need a phone wait');

    // 2. SAS: code + decision; the accept button takes focus.
    await page.getByRole('button', { name: 'Pair without the camera' }).click();
    assert.deepEqual((await calls('pairing_action')).at(-1), ['pairing_action', { action: 'sas' }]);
    await emit({ ...base, phase: 'sas-code', code: '042 917', phone: 'Galaxy S25 Ultra', message: 'Does Galaxy S25 Ultra show the same code?' });
    assert.equal(await page.locator('#onboarding-code').textContent(), '042 917');
    assert.equal(await canvas.isVisible(), false);
    assert.equal(await page.evaluate(() => document.activeElement?.id), 'onboarding-accept');
    await page.keyboard.press('Enter');
    assert.deepEqual((await calls('pairing_action')).at(-1), ['pairing_action', { action: 'sas-accept' }]);

    // 3. Expired: a message and "Show a new code", no new code on its own.
    const before = (await calls('pairing_action')).length;
    await emit({ ...base, phase: 'expired', message: 'Pairing expired or the attempt limit was reached. Start again.' });
    assert.equal(await page.getByRole('button', { name: 'Show a new code' }).isVisible(), true);
    await page.waitForTimeout(300);
    assert.equal((await calls('pairing_action')).length, before, 'nothing refreshes by itself');
    await page.getByRole('button', { name: 'Show a new code' }).click();
    assert.deepEqual((await calls('pairing_action')).at(-1), ['pairing_action', { action: 'refresh' }]);

    // 4. Paired here: "Done" with the phone, autostart, focus on the finish button.
    await emit({ ...base, phase: 'connected', phone: 'Galaxy S25 Ultra', connected: true, devices: [phone], paired: true });
    assert.match(await page.locator('#onboarding-done-title').textContent(), /Galaxy S25 Ultra connected/);
    assert.equal(await page.evaluate(() => document.activeElement?.id), 'onboarding-finish');
    assert.match(await page.locator('.onboarding-steps > li.current').textContent(), /Done/);
    for (const theme of ['light', 'dark']) {
      for (const text of ['100%', '150%']) {
        await page.evaluate(([t, f]) => { document.documentElement.dataset.theme = t; document.documentElement.style.fontSize = f; }, [theme, text]);
        await page.waitForTimeout(300);
        assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, `overflow ${theme} ${text}`);
        await page.screenshot({ path: path.join(out, `onboarding-done-${theme}-${text.replace('%', '')}.png`), fullPage: true });
      }
    }
    await page.evaluate(() => { document.documentElement.style.fontSize = ''; });
    await page.getByRole('switch', { name: 'Start with Windows' }).click();
    assert.equal(await page.getByRole('switch', { name: 'Start with Windows' }).getAttribute('aria-checked'), 'true');
    await page.getByRole('button', { name: 'Go to the app' }).click();
    await page.locator('#onboarding').waitFor({ state: 'hidden' });
    assert.equal(await page.locator('#home').isVisible(), true, 'the ordinary main window after onboarding');

    // 5. Waiting state screenshots (QR) in both themes and at 150% text.
    const fresh = await browser.newPage({ locale: 'en-US', viewport: { width: 920, height: 720 } });
    fresh.on('pageerror', e => errors.push(e.message));
    await fresh.addInitScript(stub);
    await fresh.goto(`http://127.0.0.1:${server.address().port}/index.html`);
    await fresh.locator('#onboarding-qr:not([hidden])').waitFor();
    for (const theme of ['light', 'dark']) {
      for (const text of ['100%', '150%']) {
        await fresh.evaluate(([t, f]) => { document.documentElement.dataset.theme = t; document.documentElement.style.fontSize = f; }, [theme, text]);
        await fresh.waitForTimeout(300);
        assert.equal(await fresh.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, `overflow waiting ${theme} ${text}`);
        await fresh.screenshot({ path: path.join(out, `onboarding-qr-${theme}-${text.replace('%', '')}.png`), fullPage: true });
      }
    }

    // 6. WN-113: only a phone owed its unpair notice left. The backend's rule (paired: false, as has_paired() at start)
    // opens onboarding; the owed row itself doesn't count. And the reverse: paired at start, no onboarding.
    const owedPhone = { ...phone, key: 'b'.repeat(64), name: 'Removed phone', connected: false, owed: true };
    const opened = async start => {
      const p = await browser.newPage({ locale: 'en-US', viewport: { width: 920, height: 720 } });
      p.on('pageerror', e => errors.push(e.message));
      await p.addInitScript(stub, start);
      await p.goto(`http://127.0.0.1:${server.address().port}/index.html`);
      await p.waitForFunction(() => window.__calls.some(([c]) => c === 'pairing_snapshot'));
      await p.waitForTimeout(200);
      return p;
    };
    const owedOnly = await opened({ devices: [owedPhone], paired: false });
    assert.equal(await owedOnly.locator('#onboarding').isVisible(), true, 'owed only: onboarding, as the first window decided');
    assert.equal(await owedOnly.locator('#home').isVisible(), false);
    const stillPaired = await opened({ phase: 'offline', qr: null, devices: [{ ...phone, connected: false }, owedPhone], paired: true });
    assert.equal(await stillPaired.locator('#onboarding').isVisible(), false, 'paired: no onboarding');
    assert.equal(await stillPaired.locator('#home').isVisible(), true);

    assert.deepEqual(errors, []);
    console.log(`PASS: onboarding steps, QR rules, SAS, expiry, Gotowe/autostart/finish, focus, Light/Dark, 150% text, owed-only start (WN-113); ${out}`);
  } finally {
    await browser.close();
    server.close();
  }
})().catch(e => { console.error(e); process.exitCode = 1; });
