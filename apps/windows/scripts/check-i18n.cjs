// CCS/1 PURPOSE:WN-63 translation regression: PL/EN key parity, every used key exists, no Polish left in English UI.
// Static part runs anywhere (node apps/windows/scripts/check-i18n.cjs); with PLAYWRIGHT_MODULE set it also renders
// every page in en-US and pl-PL (screenshots in %TEMP%/windconnect-wn63-i18n or UI_ARTIFACTS).
const assert = require('node:assert/strict');
const fs = require('node:fs');
const fsp = require('node:fs/promises');
const http = require('node:http');
const os = require('node:os');
const path = require('node:path');
const { pathToFileURL } = require('node:url');

const root = path.resolve(__dirname, '../frontend');
const native = path.resolve(__dirname, '../src-tauri/locales');
const placeholders = text => [...String(text).matchAll(/\{(\w+)\}/g)].map(m => m[1]).sort().join();

function parity(name, pl, en) {
  assert.deepEqual(Object.keys(pl).sort(), Object.keys(en).sort(), `${name}: the same keys in pl and en`);
  for (const key of Object.keys(pl)) {
    assert.ok(pl[key] && en[key], `${name} ${key}: empty text`);
    assert.equal(placeholders(pl[key]), placeholders(en[key]), `${name} ${key}: placeholders`);
  }
}

function usedKeys() {
  const used = new Set();
  for (const file of fs.readdirSync(root).filter(name => /\.(html|js)$/.test(name))) {
    const text = fs.readFileSync(path.join(root, file), 'utf8');
    if (file.endsWith('.html') || file.endsWith('.js')) {
      for (const m of text.matchAll(/data-(?:i18n(?:-[a-z-]+)?|feedback)="([^"]+)"/g)) if (!['start'].includes(m[1])) used.add(m[1]);
    }
    if (file.endsWith('.js') && file !== 'i18n.js') {
      for (const m of text.matchAll(/(?<![\w.])t\('([^']+)'/g)) used.add(m[1]);
      for (const m of text.matchAll(/(?<![\w.])plural\('([^']+)'/g)) used.add(`${m[1]}.other`);
    }
  }
  return used;
}

function nativeUsed() {
  const used = new Set();
  const src = path.resolve(__dirname, '../src-tauri/src');
  for (const file of fs.readdirSync(src)) {
    const text = fs.readFileSync(path.join(src, file), 'utf8');
    for (const m of text.matchAll(/i18n::tr[fp]?\(\s*"([^"]+)"/g)) used.add(m[1]);
  }
  return used;
}

async function render(pl, en) {
  const { chromium } = require(process.env.PLAYWRIGHT_MODULE);
  const artifacts = process.env.UI_ARTIFACTS || path.join(os.tmpdir(), 'windconnect-wn63-i18n');
  await fsp.mkdir(artifacts, { recursive: true });
  const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.ttf': 'font/ttf', '.svg': 'image/svg+xml' };
  const server = http.createServer(async (request, response) => {
    const pathname = new URL(request.url, 'http://localhost').pathname;
    const filename = path.resolve(root, '.' + pathname);
    if (!filename.startsWith(root + path.sep)) { response.writeHead(403).end(); return; }
    try {
      response.setHeader('Content-Type', types[path.extname(filename)] || 'text/plain');
      response.end(await fsp.readFile(filename));
    } catch { response.writeHead(404).end(); }
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const base = `http://127.0.0.1:${server.address().port}`;
  const browser = await chromium.launch({ headless: true });
  const polish = /[ąćęłńóśźżĄĆĘŁŃÓŚŹŻ]/;
  const rawKey = /^(?:[a-z]+\.)+[a-z0-9_-]+$/;
  try {
    for (const locale of ['en-US', 'pl-PL']) {
      const page = await browser.newPage({ locale, viewport: { width: 920, height: 900 } });
      const errors = [];
      page.on('pageerror', error => errors.push(error.message));
      for (const file of ['index.html', 'panel.html', 'sms.html', 'tray-menu.html']) {
        await page.goto(`${base}/${file}`);
        await page.waitForFunction(() => document.documentElement.lang !== '');
        const seen = await page.evaluate(() => {
          const out = [document.title];
          // Every marked text and attribute, visible or not (hidden sections show later).
          for (const element of document.querySelectorAll('[data-i18n]')) out.push(element.textContent.trim());
          for (const name of ['aria-label', 'placeholder', 'title', 'alt']) {
            for (const element of document.querySelectorAll(`[${name}]`)) out.push(element.getAttribute(name));
          }
          return { lang: document.documentElement.lang, texts: out.filter(Boolean) };
        });
        assert.equal(seen.lang, locale.slice(0, 2), `${file} ${locale}: html lang`);
        for (const text of seen.texts) {
          assert.ok(!rawKey.test(text), `${file} ${locale}: raw key shown: ${text}`);
          if (locale === 'en-US') assert.ok(!polish.test(text) || /Zażółć gęślą jaźń/.test(text), `${file}: Polish text in English UI: ${text}`);
        }
        await page.screenshot({ path: path.join(artifacts, `${locale}-${file.replace('.html', '')}.png`), fullPage: true });
      }
      assert.deepEqual(errors, [], `${locale}: page errors`);
      await page.close();
    }
    console.log(`Rendered pl-PL and en-US: ${artifacts}`);
  } finally {
    await browser.close();
    server.close();
  }
}

(async () => {
  const pl = (await import(pathToFileURL(path.join(root, 'locales/pl.js')))).default;
  const en = (await import(pathToFileURL(path.join(root, 'locales/en.js')))).default;
  parity('frontend', pl, en);
  const missing = [...usedKeys()].filter(key => !(key in en));
  assert.deepEqual(missing, [], 'frontend keys used but not defined');
  const nativePl = JSON.parse(fs.readFileSync(path.join(native, 'pl.json'), 'utf8'));
  const nativeEn = JSON.parse(fs.readFileSync(path.join(native, 'en.json'), 'utf8'));
  parity('native', nativePl, nativeEn);
  const nativeMissing = [...nativeUsed()].filter(key => !(key in nativeEn) && !(`${key}.other` in nativeEn));
  assert.deepEqual(nativeMissing, [], 'native keys used but not defined');
  console.log(`Keys: frontend ${Object.keys(en).length}, native ${Object.keys(nativeEn).length}; PL/EN parity ok.`);
  if (process.env.PLAYWRIGHT_MODULE) await render(pl, en);
})().catch(error => { console.error(error); process.exitCode = 1; });
