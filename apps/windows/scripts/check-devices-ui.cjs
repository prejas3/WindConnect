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
  { key: KEY_A, name: 'Galaxy S25 Ultra', fingerprint: 'K4TQ-9WZM-2HXP', paired_at: 1794300000, connected: true, owed: false, model: 'SM-S938B' },
  { key: KEY_B, name: 'Pixel 9 for work with a very long device name that has to wrap', fingerprint: 'M7RC-1QDN-8LVA', paired_at: 1794390000, connected: false, owed: false },
  { key: KEY_C, name: 'Removed phone', fingerprint: 'Z2PE-6YHT-4KUB', paired_at: 1794400000, connected: false, owed: true },
];
const stub = rows => {
  window.__calls = [];
  window.__listeners = {};
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => {
      window.__calls.push([cmd, args]);
      if (cmd === 'pairing_snapshot') return { phase: 'connected', phone: 'Galaxy S25 Ultra', qr: null, message: 'Connected · Wi-Fi', skipped_records: 0, code: null, connected: true, battery: { level: 68, charging: false }, devices: structuredClone(rows) };
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
    const page = await browser.newPage({ locale: 'en-US', viewport: { width: 380, height: 640 } });
    const errors = [];
    page.on('pageerror', e => errors.push(e.message));
    await page.addInitScript(stub, devices);
    await page.goto(`http://127.0.0.1:${server.address().port}/panel.html`);
    await page.locator('#devices:not([hidden])').waitFor();
    await page.evaluate(() => document.fonts.ready);
    const calls = () => page.evaluate(() => window.__calls.filter(([c]) => c === 'device_action' || c === 'pairing_action'));
    const rows = page.locator('#device-list > li');
    assert.equal(await rows.count(), 3);
    assert.match(await rows.nth(0).textContent(), /Connected/);
    assert.match(await rows.nth(2).textContent(), /learns of it on the next connection/);
    assert.equal(await rows.nth(2).locator('button').count(), 0, 'an owed row has no actions');
    assert.equal(await rows.nth(1).getByRole('button', { name: /^Disconnect/ }).count(), 0, 'disconnect only for the connected phone');
    // WN-161: name, state with its dot and the battery; the fingerprint, date and model only under "Details".
    assert.equal(await rows.nth(0).locator('.device-state').textContent(), 'Connected · battery 68%');
    assert.equal(await rows.nth(0).locator('.device-state.connected').count(), 1);
    assert.equal(await rows.nth(1).locator('.device-state').textContent(), 'Not connected');
    assert.equal(await page.locator('#device-list .device-fingerprint').count(), 0, 'no fingerprint in the main view');
    assert.equal(/K4TQ|sparowano/i.test(await page.locator('#device-list').textContent()), false);
    // One main action as a button (outlined, not a text link); the rest in the "…" menu.
    const disconnect = rows.nth(0).getByRole('button', { name: /^Disconnect/ });
    assert.doesNotMatch(await disconnect.evaluate(b => b.className), /quiet|primary|danger/);
    assert.equal(await rows.nth(0).getByRole('button', { name: /^Rename|^Remove/ }).count(), 0, 'rename and remove are in the menu');
    // "Add a phone" is a proper (outlined) button with an icon, on the title's line.
    const add = page.getByRole('button', { name: 'Add a phone' });
    assert.doesNotMatch(await add.evaluate(b => b.className), /quiet/);
    const addBox = await add.boundingBox(), titleBox = await page.locator('#devices-title').boundingBox();
    assert.ok(Math.abs((addBox.y + addBox.height / 2) - (titleBox.y + titleBox.height / 2)) < 3, 'add is centred on the title line');
    for (const theme of ['light', 'dark']) {
      await page.evaluate(t => { document.documentElement.dataset.theme = t; }, theme);
      await page.waitForTimeout(400);
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, 'overflow ' + theme);
      await page.locator('#devices').screenshot({ path: path.join(out, 'panel-devices-' + theme + '.png') });
    }

    // The menu: own control, opened from "…", keyboard (arrows, Escape back to its button), closes on a click outside.
    const more = rows.nth(0).getByRole('button', { name: 'More actions: Galaxy S25 Ultra' });
    assert.equal(await more.getAttribute('aria-haspopup'), 'menu');
    await more.click();
    assert.equal(await more.getAttribute('aria-expanded'), 'true');
    const menu = rows.nth(0).getByRole('menu');
    assert.deepEqual(await menu.getByRole('menuitem').allTextContents(), ['Rename', 'Details', 'Remove pairing']);
    const focusedText = () => page.evaluate(() => document.activeElement.textContent);
    assert.equal(await focusedText(), 'Rename');
    await page.keyboard.press('ArrowDown');
    assert.equal(await focusedText(), 'Details');
    await page.keyboard.press('ArrowUp'); await page.keyboard.press('ArrowUp');
    assert.equal(await focusedText(), 'Remove pairing');
    await page.waitForTimeout(300); // after the menu's fade-in
    await page.locator('#devices').screenshot({ path: path.join(out, 'panel-devices-menu.png') });
    const panelCalls = () => page.evaluate(() => window.__calls.filter(([c]) => c === 'panel_action').length);
    const hides = await panelCalls();
    await page.keyboard.press('Escape');
    assert.equal(await panelCalls(), hides, 'Escape in the menu closes the menu, not the panel');
    assert.equal(await rows.nth(0).getByRole('menu').count(), 0);
    assert.equal(await page.evaluate(() => document.activeElement.getAttribute('aria-label')), 'More actions: Galaxy S25 Ultra');
    await more.click();
    await page.locator('#devices-title').click();
    assert.equal(await rows.nth(0).getByRole('menu').count(), 0, 'a click outside closes the menu');
    // A state update while the menu is open keeps it and its focus.
    await more.click(); await page.keyboard.press('ArrowDown');
    await page.evaluate(v => window.__listeners['pairing-state']({ payload: { phase: 'connected', phone: 'Galaxy S25 Ultra', qr: null, message: '', skipped_records: 0, code: null, connected: true, battery: { level: 67, charging: false }, devices: v } }), devices);
    assert.equal(await focusedText(), 'Details');
    await page.keyboard.press('Enter');
    const details = rows.nth(0).locator('.device-details');
    assert.equal(await details.locator('.device-fingerprint').textContent(), 'Fingerprint: K4TQ-9WZM-2HXP');
    assert.match(await details.textContent(), /Paired .*2026/);
    assert.match(await details.textContent(), /Model: SM-S938B/);
    await page.locator('#devices').screenshot({ path: path.join(out, 'panel-devices-details.png') });
    await more.click();
    assert.equal(await rows.nth(0).getByRole('menuitem', { name: 'Hide details' }).count(), 1);
    await page.keyboard.press('Escape');

    // Rename (from the menu): Enter saves the trimmed name; an empty name is refused in place.
    await rows.nth(1).getByRole('button', { name: /^More/ }).click();
    await rows.nth(1).getByRole('menuitem', { name: 'Rename' }).click();
    const input = rows.nth(1).locator('input');
    await input.fill('   ');
    await input.press('Enter');
    assert.equal(await input.getAttribute('aria-invalid'), 'true');
    await input.fill("  Ola's phone ");
    // A state update while typing keeps what was typed (and the focus).
    await page.evaluate(v => window.__listeners['pairing-state']({ payload: { phase: 'connected', phone: 'Galaxy S25 Ultra', qr: null, message: '', skipped_records: 0, code: null, connected: true, devices: v } }), devices);
    assert.equal(await rows.nth(1).locator('input').inputValue(), "  Ola's phone ");
    assert.equal(await page.evaluate(() => document.activeElement.tagName), 'INPUT');
    await rows.nth(1).locator('input').press('Enter');
    assert.deepEqual((await calls()).at(-1), ['device_action', { action: 'rename', key: KEY_B, name: "Ola's phone" }]);

    // Remove pairing is the dangerous entry: it asks in place first; Cancel sends nothing.
    await more.click();
    assert.match(await rows.nth(0).getByRole('menuitem', { name: 'Remove pairing' }).evaluate(b => b.className), /danger/);
    await rows.nth(0).getByRole('menuitem', { name: 'Remove pairing' }).click();
    assert.match(await rows.nth(0).locator('.confirm').textContent(), /The phone removes it too\./);
    await page.locator('#devices').screenshot({ path: path.join(out, 'panel-devices-confirm.png') });
    const before = (await calls()).length;
    await rows.nth(0).getByRole('button', { name: /^Cancel/ }).click();
    assert.equal((await calls()).length, before);
    await rows.nth(1).getByRole('button', { name: /^More/ }).click();
    await rows.nth(1).getByRole('menuitem', { name: 'Remove pairing' }).click();
    assert.match(await rows.nth(1).locator('.confirm').textContent(), /on the next connection/);
    await rows.nth(1).getByRole('button', { name: /^Remove pairing: / }).click();
    assert.deepEqual((await calls()).at(-1), ['device_action', { action: 'unpair', key: KEY_B }]);

    await disconnect.click();
    assert.deepEqual((await calls()).at(-1), ['device_action', { action: 'disconnect', key: KEY_A }]);
    await add.click();
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
