// CCS/1 PURPOSE:WN-54 remote-actions section with a stubbed IPC (themes, 400/700/1024, keyboard, problems) | DEP:external dev-only Playwright 1.61.1.
const assert = require('node:assert/strict');
const http = require('node:http');
const fs = require('node:fs/promises');
const path = require('node:path');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const root = path.resolve(__dirname, '../frontend');
const out = process.env.UI_ARTIFACTS || path.join(require('node:os').tmpdir(), 'windconnect-wn54-ui');
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.ttf': 'font/ttf' };
const server = http.createServer(async (req, res) => {
  const p = new URL(req.url, 'http://x').pathname;
  const f = path.resolve(root, '.' + (p === '/' ? '/index.html' : p));
  if (!f.startsWith(root + path.sep)) { res.writeHead(403).end(); return; }
  try { res.setHeader('Content-Type', types[path.extname(f)] || 'text/plain'); res.end(await fs.readFile(f)); }
  catch { res.writeHead(404).end(); }
});
const stub = () => {
  const snapshot = {
    builtins: [
      { id: 'lock', label: 'Zablokuj', detail: 'Ekran blokady, jak Win+L', enabled: true, problem: null },
      { id: 'sleep', label: 'Uśpij', detail: 'Uśpienie komputera', enabled: false, problem: null },
      { id: 'open-url', label: 'Otwórz link', detail: 'Tylko adresy http i https, w domyślnej przeglądarce', enabled: true, problem: null },
    ],
    custom: [
      { id: 'x-kopia', label: 'Kopia zapasowa', detail: 'C:\\Program Files\\Backup Tools Very Long Folder Name\\backup.exe --teraz --bez-pytania', enabled: true, problem: null },
      { id: 'x-bat', label: 'Skrypt', detail: 'C:\\Tools\\b.bat', enabled: true, problem: 'Program musi być pełną ścieżką do pliku .exe.' },
    ],
    error: null,
    last: { peer: 'Galaxy S25 Ultra', label: 'Zablokuj', status: 'done', atMs: Date.now() },
  };
  window.__calls = [];
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => {
      window.__calls.push([cmd, args]);
      if (cmd === 'ui_dev_mode') return false;
      if (cmd === 'core_version') return '0.34.0';
      if (cmd === 'apply_window_theme') return false;
      if (cmd === 'actions_snapshot') return snapshot;
      if (cmd === 'set_action_enabled') {
        const item = [...snapshot.builtins, ...snapshot.custom].find(i => i.id === args.id);
        item.enabled = args.enabled; return snapshot;
      }
      return null;
    } },
    event: { listen: async () => () => {} },
  };
};
(async () => {
  await fs.mkdir(out, { recursive: true });
  await new Promise(r => server.listen(0, '127.0.0.1', r));
  const url = `http://127.0.0.1:${server.address().port}/`;
  const browser = await chromium.launch({ headless: true });
  try {
    const page = await browser.newPage({ viewport: { width: 1024, height: 1000 } });
    const errors = [];
    page.on('pageerror', e => errors.push(e.message));
    await page.addInitScript(stub);
    await page.goto(url);
    await page.locator('#actions:not([hidden])').waitFor();
    await page.evaluate(() => document.fonts.ready);
    for (const theme of ['light', 'dark']) {
      await page.locator(`[data-theme-choice="${theme}"]`).click();
      for (const width of [400, 700, 1024]) {
        await page.setViewportSize({ width, height: 1000 });
        assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, `overflow ${theme}/${width}`);
        await page.locator('#actions').screenshot({ path: path.join(out, `actions-${theme}-${width}.png`) });
      }
    }
    const sleep = page.getByRole('switch', { name: 'Uśpij' });
    assert.equal(await sleep.getAttribute('aria-checked'), 'false');
    await sleep.focus(); await page.keyboard.press('Space');
    await page.waitForFunction(() => window.__calls.some(c => c[0] === 'set_action_enabled'));
    assert.equal(await page.getByRole('switch', { name: 'Uśpij' }).getAttribute('aria-checked'), 'true');
    assert.equal(await page.getByRole('switch', { name: 'Skrypt' }).isDisabled(), true);
    assert.match(await page.locator('#actions-last').textContent(), /Galaxy S25 Ultra · Zablokuj · wykonano/);
    assert.deepEqual(errors, []);
    console.log(`PASS: actions section, themes, 6 layouts, keyboard, problems; screenshots: ${out}`);
  } finally { await browser.close(); server.close(); }
})().catch(e => { console.error(e); process.exit(1); });
