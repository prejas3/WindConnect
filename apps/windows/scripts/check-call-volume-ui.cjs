// CCS/1 PURPOSE:WN-48 with a stubbed IPC: the panel's "Głośność telefonu" (keys, mute, at most every 100 ms, no bounce-back from
// an echoed level while dragging or within 500 ms of a local change, then the pushed level wins; read-only when the phone doesn't
// take volume/1) and the main window's two switches (call_volume_settings). Light/Dark | DEP:external dev-only Playwright 1.61.1.
const assert = require('node:assert/strict');
const http = require('node:http');
const fs = require('node:fs/promises');
const path = require('node:path');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const root = path.resolve(__dirname, '../frontend');
const out = process.env.UI_ARTIFACTS || path.join(require('node:os').tmpdir(), 'windconnect-wn48-ui');
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.ttf': 'font/ttf' };
const server = http.createServer(async (req, res) => {
  const p = new URL(req.url, 'http://x').pathname;
  const f = path.resolve(root, '.' + (p === '/' ? '/index.html' : p));
  if (!f.startsWith(root + path.sep)) { res.writeHead(403).end(); return; }
  try { res.setHeader('Content-Type', types[path.extname(f)] || 'text/plain'); res.end(await fs.readFile(f)); }
  catch { res.writeHead(404).end(); }
});
const phone = { key: 'a'.repeat(64), name: 'Pixel 9', fingerprint: 'K4TQ-9WZM-2HXP', paired_at: 1794300000, connected: true, owed: false };
const state = (volume, extra = {}) => ({ phase: 'connected', phone: 'Pixel 9', qr: null, message: 'Połączono · Wi-Fi', skipped_records: 0, code: null, connected: true, devices: [phone], paired: true, battery: null, find: { available: false, state: 'idle', last: null, ring_ms: null }, media: null, share_media: true, phone_notifications: true, quiet_hours: { on: false, from: 1320, to: 420 }, notifications: [], pause_on_call: true, volume_remote: true, phone_volume: volume, ...extra });
const stub = first => {
  window.__calls = [];
  window.__listeners = {};
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => {
      window.__calls.push([cmd, args, performance.now()]);
      if (cmd === 'pairing_snapshot') return structuredClone(first);
      if (cmd === 'files_snapshot') return { rows: [], active: 0, activeDone: 0, activeSize: 0, phone: null };
      if (cmd === 'panel_snapshot') return { shortcut_error: false };
      if (cmd === 'clipboard_snapshot') return { enabled: true, status: '', shortcut_error: false };
      if (cmd === 'actions_snapshot') return { builtins: [], custom: [], last: null, problems: [] };
      if (cmd === 'autostart_enabled') return false;
      if (cmd === 'core_version') return '0.52.0';
      return null;
    } },
    event: { listen: async (name, cb) => { (window.__listeners[name] ||= []).push(cb); return () => {}; } },
  };
};
const push = (page, v) => page.evaluate(v => window.__listeners['pairing-state'].forEach(cb => cb({ payload: v })), v);
(async () => {
  await fs.mkdir(out, { recursive: true });
  await new Promise(r => server.listen(0, '127.0.0.1', r));
  const browser = await chromium.launch({ headless: true });
  try {
    // The panel.
    const page = await browser.newPage({ locale: 'pl-PL', viewport: { width: 380, height: 760 } });
    const errors = [];
    page.on('pageerror', e => errors.push(e.message));
    await page.addInitScript(stub, state({ level: 40, muted: false, controllable: true }));
    await page.goto(`http://127.0.0.1:${server.address().port}/panel.html`);
    const slider = page.getByRole('slider', { name: 'Głośność telefonu' });
    await page.locator('#phone-volume:not([hidden])').waitFor();
    assert.equal(await slider.getAttribute('aria-valuenow'), '40');
    const sent = () => page.evaluate(() => window.__calls.filter(([c]) => c === 'phone_volume').map(([, a]) => a));
    // Keys: 5 % and 10 %; sends coalesce to at most one per 100 ms (the latest wins).
    await slider.focus();
    for (let i = 0; i < 4; i++) await page.keyboard.press('ArrowRight');
    await page.keyboard.press('PageUp');
    await page.waitForTimeout(250);
    assert.equal(await slider.getAttribute('aria-valuenow'), '70');
    const calls = await sent();
    assert(calls.length <= 3, `coalesced: ${calls.length}`);
    assert.deepEqual(calls.at(-1), { level: 70, muted: null });
    // An echo of an older level within 500 ms doesn't move the slider back.
    await push(page, state({ level: 45, muted: false, controllable: true }));
    assert.equal(await slider.getAttribute('aria-valuenow'), '70', 'no bounce-back');
    // After 500 ms the pushed level wins.
    await page.waitForTimeout(600);
    assert.equal(await slider.getAttribute('aria-valuenow'), '45', 'then the latest pushed level wins');
    // While dragging, pushed levels don't move it either.
    const box = await slider.boundingBox();
    await page.mouse.move(box.x + box.width * 0.2, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width * 0.9, box.y + box.height / 2, { steps: 5 });
    await push(page, state({ level: 10, muted: false, controllable: true }));
    await page.waitForTimeout(700);
    assert.equal(await slider.getAttribute('aria-valuenow'), '90', 'held while dragging');
    await page.mouse.up();
    await page.waitForTimeout(600);
    assert.equal(await slider.getAttribute('aria-valuenow'), '10', 'released: the latest pushed wins');
    // Mute sends only muted (a level alone never unmutes; mute never touches the level).
    await page.getByRole('button', { name: 'Wycisz telefon' }).click();
    await page.waitForTimeout(150);
    assert.deepEqual((await sent()).at(-1), { level: null, muted: true });
    for (const theme of ['light', 'dark']) {
      await page.evaluate(t => { document.documentElement.dataset.theme = t; }, theme);
      await page.locator('#phone-volume').screenshot({ path: path.join(out, `phone-volume-${theme}.png`) });
    }
    // The phone doesn't take volume/1: shown, read-only, with a note.
    await push(page, state({ level: 30, muted: false, controllable: false }));
    await page.waitForTimeout(600);
    assert.equal(await slider.getAttribute('aria-disabled'), 'true');
    assert.equal(await page.locator('#phone-volume-note').isVisible(), true);
    const before = (await sent()).length;
    await slider.focus(); await page.keyboard.press('ArrowRight');
    await page.waitForTimeout(150);
    assert.equal((await sent()).length, before, 'nothing sent');
    await push(page, state(null));
    assert.equal(await page.locator('#phone-volume').isHidden(), true);

    // The main window's switches.
    const main = await browser.newPage({ locale: 'pl-PL', viewport: { width: 920, height: 900 } });
    main.on('pageerror', e => errors.push(e.message));
    await main.addInitScript(stub, state(null));
    await main.goto(`http://127.0.0.1:${server.address().port}/index.html`);
    const pause = main.getByRole('switch', { name: 'Wstrzymaj odtwarzanie przy połączeniu' });
    const volume = main.getByRole('switch', { name: 'Głośność z telefonu' });
    await main.locator('#media-settings:not([hidden])').waitFor();
    assert.equal(await pause.getAttribute('aria-checked'), 'true');
    await pause.click();
    await volume.click();
    const settings = await main.evaluate(() => window.__calls.filter(([c]) => c === 'call_volume_settings').map(([, a]) => a));
    assert.deepEqual(settings, [{ pauseOnCall: false }, { volume: false }]);
    await push(main, state(null, { pause_on_call: true, volume_remote: false }));
    assert.equal(await pause.getAttribute('aria-checked'), 'true');
    assert.equal(await volume.getAttribute('aria-checked'), 'false');
    for (const theme of ['light', 'dark']) {
      await main.evaluate(t => { document.documentElement.dataset.theme = t; }, theme);
      await main.locator('#media-settings').screenshot({ path: path.join(out, `call-settings-${theme}.png`) });
    }
    assert.deepEqual(errors, []);
    console.log(JSON.stringify({ callVolumeUi: true, sends: (await sent()).length, themes: 2, artifacts: out }));
  } finally {
    await browser.close();
    server.close();
  }
})().catch(e => { console.error(e); process.exitCode = 1; });
