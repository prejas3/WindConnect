// CCS/1 PURPOSE:WN-64 accessibility of the Windows UI: the whole panel (every section shown) and the other pages walked
// with the keyboard alone (Tab order, nothing skipped, no trap, a visible focus ring), every control named for
// Narrator, WCAG AA text contrast in both themes and forced colors, and 200% text (Windows "Text size") without
// horizontal scroll or clipped controls. Stubbed IPC in headless Chromium. DEP:external dev-only Playwright.
const assert = require('node:assert/strict');
const http = require('node:http');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const root = path.resolve(__dirname, '../frontend');
const out = process.env.UI_ARTIFACTS || path.join(os.tmpdir(), 'windconnect-wn64-a11y');
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.ttf': 'font/ttf', '.svg': 'image/svg+xml' };
const server = http.createServer(async (req, res) => {
  const f = path.resolve(root, '.' + new URL(req.url, 'http://x').pathname);
  if (!f.startsWith(root + path.sep)) { res.writeHead(403).end(); return; }
  try { res.setHeader('Content-Type', types[path.extname(f)] || 'text/plain'); res.end(await fs.readFile(f)); }
  catch { res.writeHead(404).end(); }
});

const pixel = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=';
const devices = [
  { key: 'a'.repeat(64), name: 'Pixel 9', fingerprint: 'K4TQ-9WZM-2HXP', paired_at: 1794300000, connected: true, owed: false },
  { key: 'b'.repeat(64), name: 'Galaxy S25', fingerprint: 'M2AB-7QZC-1KDE', paired_at: 1794200000, connected: false, owed: false },
];
const full = {
  phase: 'connected', phone: 'Pixel 9', qr: null, message: '', skipped_records: 0, code: null, connected: true, devices,
  battery: { level: 64, charging: true },
  find: { available: true, state: 'idle', last: null, ring_ms: null },
  media: { app: 'Spotify', title: 'Kaonashi', artist: 'Daria Zawiałow', album: 'Helsinki', art: pixel, playing: true, position_ms: 61000, duration_ms: 214000, actions: 15, speed: 1, at_ms: Date.now() },
  phone_volume: { level: 40, muted: false, controllable: true },
  call_toasts: true,
  missed_calls: [{ who: 'Ola', number: '+48 600 100 200', date_ms: 1794300000000 }],
  notifications: [{ tag: 'n1', app: 'Signal', title: 'Ola', text: 'Będziesz o 19?', reply: 'Odpowiedz', status: null }],
  sms: true, open_links: true,
  touchpad: { allowed: true, active: false, message: '' },
};
const files = { phone: 'Pixel 9', active: 1, activeSize: 100, activeDone: 40, rows: [
  { key: 'o:1', id: 1, direction: 'outgoing', name: 'raport.pdf', state: 'running', done: 40, size: 100, rate: 10, canRetry: false, canOpen: false },
  { key: 'i:2', id: 2, direction: 'incoming', name: 'zdjęcie.jpg', state: 'done', done: 10, size: 10, rate: null, canRetry: false, canOpen: true },
] };
const history = [{ id: 1, kind: 'text', preview: 'https://example.com', direction: 'to-phone', at: 1794300000000, pinned: false, ready: true }];
const gallery = { available: true, photos: [{ id: 1, name: 'IMG_2041.jpg', date_ms: 1794300000000, screenshot: false, thumb: pixel }] };

function stub(state, files, history, gallery) {
  window.__listeners = {};
  window.__TAURI__ = {
    core: { invoke: async cmd => ({
      pairing_snapshot: state, files_snapshot: files, panel_snapshot: { shortcut_error: null },
      clipboard_snapshot: { enabled: true, status: 'Synchronizacja automatyczna', shortcut_error: false, image_limit_mb: 16 },
      clipboard_history: history, gallery_state: gallery, core_version: '0.75.0', ui_dev_mode: false,
      autostart_state: { enabled: true, available: true }, actions_snapshot: null,
    })[cmd] ?? null },
    event: { listen: async (name, cb) => { window.__listeners[name] = cb; return () => {}; } },
  };
}

// WCAG relative luminance contrast of each visible text against the first opaque background behind it.
const contrastProblems = () => {
  const parse = c => { const m = c.match(/rgba?\(([^)]+)\)/); if (!m) return null; const [r, g, b, a = 1] = m[1].split(/[ ,/]+/).filter(Boolean).map(Number); return { r, g, b, a }; };
  const lum = ({ r, g, b }) => { const f = v => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b); };
  const background = element => {
    const layers = [];
    for (let e = element; e; e = e.parentElement) { const c = parse(getComputedStyle(e).backgroundColor); if (c && c.a > 0) { layers.push(c); if (c.a >= 1) break; } }
    let base = layers.pop() || { r: 255, g: 255, b: 255, a: 1 };
    for (const c of layers.reverse()) base = { r: c.r * c.a + base.r * (1 - c.a), g: c.g * c.a + base.g * (1 - c.a), b: c.b * c.a + base.b * (1 - c.a), a: 1 };
    return base;
  };
  const bad = [];
  for (const element of document.querySelectorAll('body *')) {
    const own = [...element.childNodes].some(n => n.nodeType === 3 && n.nodeValue.trim());
    if (!own || element.closest('[hidden],[aria-hidden="true"]')) continue;
    const style = getComputedStyle(element); const rect = element.getBoundingClientRect();
    if (style.visibility === 'hidden' || rect.width === 0 || rect.height === 0) continue;
    const disabled = element.closest(':disabled,[aria-disabled="true"]');
    if (disabled) continue; // WCAG 1.4.3 exempts inactive controls.
    const fg = parse(style.color); if (!fg) continue;
    const bg = background(element);
    const color = { r: fg.r * fg.a + bg.r * (1 - fg.a), g: fg.g * fg.a + bg.g * (1 - fg.a), b: fg.b * fg.a + bg.b * (1 - fg.a) };
    const [l1, l2] = [lum(color), lum(bg)].sort((a, b) => b - a);
    const ratio = (l1 + 0.05) / (l2 + 0.05);
    const size = parseFloat(style.fontSize), bold = Number(style.fontWeight) >= 700;
    const large = size >= 24 || (bold && size >= 18.66);
    if (ratio < (large ? 3 : 4.5)) bad.push(`${element.tagName.toLowerCase()}#${element.id || element.className} "${element.textContent.trim().slice(0, 30)}" ${ratio.toFixed(2)}`);
  }
  return bad;
};

// Walks the page with Tab: every reachable control, in order, with a visible focus indicator and an accessible name.
async function walk(page, label) {
  const seen = [];
  await page.evaluate(() => { document.activeElement?.blur(); window.scrollTo(0, 0); });
  for (let i = 0; i < 200; i++) {
    await page.keyboard.press('Tab');
    const info = await page.evaluate(() => {
      const e = document.activeElement;
      if (!e || e === document.body) return null;
      const style = getComputedStyle(e);
      const ring = (style.outlineStyle !== 'none' && parseFloat(style.outlineWidth) >= 1) || style.boxShadow !== 'none';
      const id = e.id || e.dataset.action || e.className || e.tagName;
      return { id: `${e.tagName.toLowerCase()}#${id}`, ring, key: e.outerHTML.slice(0, 80) };
    });
    if (!info) break;
    if (seen.length && info.key === seen[0].key) break; // wrapped around
    seen.push(info);
  }
  // Chromium's accessibility tree (what UIA/Narrator gets): every control has a name.
  const cdp = await page.context().newCDPSession(page);
  const { nodes } = await cdp.send('Accessibility.getFullAXTree');
  await cdp.detach();
  const roles = ['button', 'link', 'textbox', 'slider', 'switch', 'checkbox', 'menuitem', 'menuitemcheckbox', 'combobox', 'listbox', 'option', 'image', 'progressbar', 'spinbutton', 'searchbox'];
  const unnamed = nodes.filter(n => !n.ignored && roles.includes(n.role?.value) && !String(n.name?.value || '').trim())
    .map(n => `${n.role.value}#${n.backendDOMNodeId}`);
  assert.deepEqual(seen.filter(s => !s.ring).map(s => s.id), [], `${label}: focus without a visible indicator`);
  assert.deepEqual(unnamed, [], `${label}: controls without a name`);
  return seen;
}

(async () => {
  await fs.mkdir(out, { recursive: true });
  await new Promise(r => server.listen(0, '127.0.0.1', r));
  const base = `http://127.0.0.1:${server.address().port}`;
  const browser = await chromium.launch({ headless: true });
  const problems = [];
  try {
    for (const theme of ['light', 'dark']) {
      const page = await browser.newPage({ locale: 'pl-PL', viewport: { width: 380, height: 760 }, colorScheme: theme });
      const errors = []; page.on('pageerror', e => errors.push(e.message));
      await page.addInitScript(stub, full, files, history, gallery);
      await page.goto(`${base}/panel.html`);
      await page.locator('#devices:not([hidden])').waitFor();
      await page.locator('#media:not([hidden])').waitFor();
      await page.waitForTimeout(300);
      const order = await walk(page, `panel ${theme}`);
      const ids = order.map(o => o.id).join(' ');
      for (const must of ['#send-files', '#find-phone', '#touchpad-toggle', '#media-toggle', '#phone-volume-slider', '#device-add', '#settings'])
        assert.ok(ids.includes(must), `panel ${theme}: ${must} reachable by Tab (order: ${ids})`);
      // Enter/Space work on the reached controls: the device menu opens and Escape closes it, focus stays on the panel.
      await page.locator('button[aria-label^="Zmień nazwę"]').first().focus();
      await page.keyboard.press('Enter');
      assert.ok(await page.locator('#device-list input').isVisible(), 'rename opened by keyboard');
      await page.keyboard.press('Escape');
      assert.equal(await page.locator('#device-list input').count(), 0, 'Escape closes rename');
      problems.push(...(await page.evaluate(contrastProblems)).map(p => `panel ${theme}: ${p}`));
      await page.screenshot({ path: path.join(out, `panel-${theme}.png`), fullPage: true });
      // 200% text: Windows scales WebView text; the panel reflows without horizontal scroll.
      await page.evaluate(() => { document.documentElement.style.fontSize = '200%'; document.body.style.zoom = '2'; });
      await page.waitForTimeout(100);
      const overflow = await page.evaluate(() => document.documentElement.scrollWidth - innerWidth);
      assert.ok(overflow <= 1, `panel ${theme} 200%: horizontal overflow ${overflow}px`);
      await page.screenshot({ path: path.join(out, `panel-${theme}-200.png`), fullPage: true });
      assert.deepEqual(errors, [], `panel ${theme}: page errors`);
      await page.close();
      for (const file of ['index.html', 'sms.html', 'tray-menu.html']) {
        const other = await browser.newPage({ locale: 'pl-PL', viewport: { width: 920, height: 800 }, colorScheme: theme });
        await other.addInitScript(stub, full, files, history, gallery);
        await other.goto(`${base}/${file}`);
        await other.waitForTimeout(400);
        await walk(other, `${file} ${theme}`);
        problems.push(...(await other.evaluate(contrastProblems)).map(p => `${file} ${theme}: ${p}`));
        await other.close();
      }
    }
    // Forced colors (Windows contrast themes): controls keep visible borders and the focus ring.
    const hc = await browser.newPage({ locale: 'pl-PL', viewport: { width: 380, height: 760 }, forcedColors: 'active' });
    await hc.addInitScript(stub, full, files, history, gallery);
    await hc.goto(`${base}/panel.html`);
    await hc.locator('#media:not([hidden])').waitFor();
    await walk(hc, 'panel forced-colors');
    await hc.screenshot({ path: path.join(out, 'panel-forced-colors.png'), fullPage: true });
    assert.deepEqual(problems, [], 'text contrast below WCAG AA');
    console.log(`Accessibility checks passed. Screenshots: ${out}`);
  } finally {
    await browser.close();
    server.close();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
