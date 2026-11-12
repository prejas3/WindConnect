// CCS/1 PURPOSE:WN-46 the phone's player in the panel with a stubbed IPC: title/artist/app/art, progress extrapolated locally
// (no IPC call), buttons follow `actions`, commands (toggle/next/previous/seek by click and keys), hostile text stays text,
// a non-image art is not shown, hidden without a player or offline, Light/Dark, 150% text | DEP:external dev-only Playwright 1.61.1.
const assert = require('node:assert/strict');
const http = require('node:http');
const fs = require('node:fs/promises');
const path = require('node:path');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const root = path.resolve(__dirname, '../frontend');
const out = process.env.UI_ARTIFACTS || path.join(require('node:os').tmpdir(), 'windconnect-wn46-ui');
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.ttf': 'font/ttf' };
const server = http.createServer(async (req, res) => {
  const p = new URL(req.url, 'http://x').pathname;
  const f = path.resolve(root, '.' + p);
  if (!f.startsWith(root + path.sep)) { res.writeHead(403).end(); return; }
  try { res.setHeader('Content-Type', types[path.extname(f)] || 'text/plain'); res.end(await fs.readFile(f)); }
  catch { res.writeHead(404).end(); }
});
// A valid 1×1 PNG (the PC already checked the bytes; the panel only accepts the data URL shape).
const ART = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==';
const ALL = 1 | 2 | 4 | 8 | 16;
const phone = { key: 'a'.repeat(64), name: 'Pixel 9', fingerprint: 'K4TQ-9WZM-2HXP', paired_at: 1794300000, connected: true, owed: false };
const song = (over = {}) => ({ title: 'Kaonashi (wersja bardzo długa, która musi się skończyć wielokropkiem w panelu)', artist: 'Daria Zawiałow', album: 'Wojny i noce', app: 'Spotify', duration_ms: 200_000, position_ms: 60_000, received_ms: Date.now(), playing: true, speed_permille: 1000, actions: ALL, art: ART, ...over });
const state = media => ({ phase: 'connected', phone: phone.name, qr: null, message: 'Połączono · Wi-Fi', skipped_records: 0, code: null, connected: true, devices: [phone], paired: true, battery: null, find: { available: false, state: 'idle', last: null, ring_ms: null }, media });
const stub = first => {
  window.__calls = [];
  window.__listeners = {};
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => {
      window.__calls.push([cmd, args]);
      if (cmd === 'pairing_snapshot') return structuredClone(first);
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
    const page = await browser.newPage({ viewport: { width: 380, height: 760 } });
    const errors = [];
    page.on('pageerror', e => errors.push(e.message));
    await page.addInitScript(stub, state(song()));
    await page.goto(`http://127.0.0.1:${server.address().port}/panel.html`);
    const card = page.locator('#media');
    await card.waitFor();
    await page.evaluate(() => document.fonts.ready);
    const emit = s => page.evaluate(v => window.__listeners['pairing-state']({ payload: v }), s);
    const sent = () => page.evaluate(() => window.__calls.filter(([c]) => c === 'media_action').map(([, a]) => a));
    const snapshots = () => page.evaluate(() => window.__calls.filter(([c]) => c === 'pairing_snapshot').length);

    // 1. What the phone plays: texts, art, the app, progress ~1:00 of 3:20, pause available.
    assert.equal(await page.locator('#media-app').textContent(), 'Na telefonie · Spotify');
    assert.equal(await page.locator('#media-sub').textContent(), 'Daria Zawiałow · Wojny i noce');
    assert.equal(await page.locator('#media-art').isVisible(), true);
    assert.equal(await page.locator('#media-art').evaluate(i => i.complete && i.naturalWidth > 0), true, 'art decoded');
    assert.equal(await page.locator('#media-total').textContent(), '3:20');
    assert.equal(await page.locator('#media-toggle').getAttribute('aria-label'), 'Wstrzymaj');
    assert.equal(await page.locator('#media-title').evaluate(e => e.scrollWidth > e.clientWidth), true, 'long title cut with an ellipsis');
    // 2. Progress moves by itself, without asking the PC.
    const before = await snapshots();
    const t0 = await page.locator('#media-progress').getAttribute('aria-valuenow');
    await page.waitForTimeout(2200);
    const t1 = await page.locator('#media-progress').getAttribute('aria-valuenow');
    assert(Number(t1) >= Number(t0) + 1, `progress moved ${t0} -> ${t1}`);
    assert.equal(await snapshots(), before, 'no polling');
    await page.screenshot({ path: path.join(out, 'panel-media-playing.png'), fullPage: true });
    // 3. Commands: buttons, a click on the bar (seek), the keyboard.
    await page.locator('#media-toggle').click();
    await page.locator('#media-next').click();
    await page.locator('#media-previous').click();
    const bar = await page.locator('#media-progress').boundingBox();
    await page.mouse.click(bar.x + bar.width / 2, bar.y + bar.height / 2);
    await page.locator('#media-progress').focus(); await page.keyboard.press('ArrowRight');
    const calls = await sent();
    assert.deepEqual(calls.slice(0, 3), [{ action: 'toggle' }, { action: 'next' }, { action: 'previous' }]);
    assert.equal(calls[3].action, 'seek'); assert(Math.abs(calls[3].positionMs - 100_000) < 3_000, `seek to the middle: ${calls[3].positionMs}`);
    assert.equal(calls[4].action, 'seek'); assert(calls[4].positionMs > 70_000, 'ArrowRight +10 s');
    // 4. Paused, only play allowed: next/previous/seek disabled, the bar doesn't seek.
    await emit(state(song({ playing: false, actions: 1 })));
    assert.equal(await page.locator('#media-toggle').getAttribute('aria-label'), 'Odtwórz');
    assert.equal(await page.locator('#media-next').isDisabled(), true);
    assert.equal(await page.locator('#media-previous').isDisabled(), true);
    assert.equal(await page.locator('#media-progress').getAttribute('aria-disabled'), 'true');
    const n = (await sent()).length;
    await page.mouse.click(bar.x + 10, bar.y + bar.height / 2);
    assert.equal((await sent()).length, n, 'no seek when not allowed');
    // 5. Hostile text stays text; art that isn't a JPEG/PNG data URL isn't shown.
    await emit(state(song({ title: '<img src=x onerror="window.__pwned=1">', artist: null, album: null, art: 'javascript:alert(1)' })));
    assert.equal(await page.locator('#media-title').textContent(), '<img src=x onerror="window.__pwned=1">');
    assert.equal(await page.evaluate(() => window.__pwned), undefined);
    assert.equal(await page.locator('#media-art').isVisible(), false);
    assert.equal(await page.locator('#media-sub').textContent(), '');
    // 6. No player, or offline: hidden.
    await emit(state(null));
    assert.equal(await card.isVisible(), false);
    await emit({ ...state(song()), phase: 'offline', connected: false });
    assert.equal(await card.isVisible(), false);
    // 7. Themes and 150% text: nothing overflows.
    await emit(state(song()));
    for (const theme of ['light', 'dark']) {
      for (const text of ['100%', '150%']) {
        await page.evaluate(([t, f]) => { document.documentElement.dataset.theme = t; document.documentElement.style.fontSize = f; }, [theme, text]);
        await page.waitForTimeout(200);
        assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, `overflow ${theme} ${text}`);
        await card.screenshot({ path: path.join(out, `panel-media-${theme}-${text.replace('%', '')}.png`) });
      }
    }
    assert.deepEqual(errors, []);
    console.log(JSON.stringify({ mediaUi: true, progressWithoutPolling: true, commands: calls.length, gatedByActions: true, hostileText: true, themes: 2, artifacts: out }));
  } finally {
    await browser.close();
    server.close();
  }
})().catch(e => { console.error(e); process.exitCode = 1; });
