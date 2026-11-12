// CCS/1 PURPOSE:clipboard UI commands/status, the image limit (WN-36), keyboard, themes and larger text; IPC stub does not prove OS delivery.
const assert = require('node:assert/strict');
const http = require('node:http');
const fs = require('node:fs/promises');
const path = require('node:path');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const root = path.resolve(__dirname, '../frontend');
const out = process.env.UI_ARTIFACTS || path.join(require('node:os').tmpdir(), 'windconnect-wn35-ui');
const server = http.createServer(async (req, res) => {
  const file = path.resolve(root, '.' + (new URL(req.url, 'http://x').pathname === '/' ? '/index.html' : new URL(req.url, 'http://x').pathname));
  if (!file.startsWith(root + path.sep)) { res.writeHead(403).end(); return; }
  try {
    res.setHeader('Content-Type', ({'.html':'text/html','.js':'text/javascript','.css':'text/css','.ttf':'font/ttf'})[path.extname(file)] || 'application/octet-stream');
    res.end(await fs.readFile(file));
  } catch { res.writeHead(404).end(); }
});
(async () => {
  await fs.mkdir(out, { recursive: true });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  let browser;
  try {
    browser = await chromium.launch({ headless: true });
    const page = await browser.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.addInitScript(() => {
      const state = {enabled:true,status:'Synchronizacja automatyczna',shortcut_error:false,image_limit_mb:16};
      window.calls = [];
      window.__TAURI__ = {
        core: {invoke: async (command, args) => {
          window.calls.push([command, args]);
          if (command === 'ui_dev_mode') return false;
          if (command === 'core_version') return '0.38.0';
          if (command === 'clipboard_snapshot') return {...state};
          if (command === 'clipboard_action') {
            if (args.action === 'enabled') state.enabled = args.enabled;
            if (args.action === 'send') state.status = 'Brak połączenia z telefonem';
            if (args.action === 'image-limit') state.image_limit_mb = args.limit;
            window.clipboardEvent?.({payload:{...state}});
          }
          return null;
        }},
        event: {listen: async (name, callback) => { if (name === 'clipboard-state') window.clipboardEvent = callback; return () => {}; }},
      };
    });
    await page.goto(`http://127.0.0.1:${server.address().port}/`);
    const controls = page.locator('[data-clipboard-controls]');
    await controls.waitFor({state:'visible'});
    for (const theme of ['light','dark']) {
      await page.locator(`[data-theme-choice="${theme}"]`).click();
      for (const width of [400,700,1024]) {
        await page.setViewportSize({width,height:1000});
        await page.evaluate(() => { document.documentElement.style.fontSize = '150%'; });
        assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
        await controls.screenshot({path:path.join(out,`${theme}-${width}-large.png`)});
      }
    }
    const toggle = page.getByRole('switch', {name:'Synchronizacja automatyczna'});
    await toggle.focus(); await page.keyboard.press('Space');
    await page.waitForFunction(() => document.querySelector('[data-clipboard-toggle]').getAttribute('aria-checked') === 'false');
    // WN-36: the image limit, by keyboard; one choice pressed.
    const limit = page.getByRole('group', {name:'Maks. rozmiar obrazu'});
    assert.equal(await limit.getByRole('button', {name:'16 MB'}).getAttribute('aria-pressed'), 'true');
    await limit.getByRole('button', {name:'50 MB'}).focus(); await page.keyboard.press('Enter');
    await page.waitForFunction(() => document.querySelector('[data-limit="50"]').getAttribute('aria-pressed') === 'true');
    assert.deepEqual(await limit.locator('[aria-pressed="true"]').allTextContents(), ['50 MB']);
    await page.getByRole('button',{name:'Wyślij schowek na telefon'}).click();
    await page.waitForFunction(() => document.querySelector('[data-clipboard-status]').textContent === 'Brak połączenia z telefonem');
    await page.evaluate(() => window.clipboardEvent({payload:{enabled:false,status:'Treść oznaczona jako wrażliwa — nie wysłano',shortcut_error:true,image_limit_mb:5}}));
    assert.deepEqual(await limit.locator('[aria-pressed="true"]').allTextContents(), ['5 MB'], 'state from the app');
    assert.match(await page.locator('[data-clipboard-shortcut]').textContent(), /jest zajęty/);
    assert.deepEqual(await page.evaluate(() => window.calls.filter(([c]) => c === 'clipboard_action')), [
      ['clipboard_action',{action:'enabled',enabled:false}],['clipboard_action',{action:'image-limit',limit:50}],['clipboard_action',{action:'send'}],
    ]);
    assert.deepEqual(errors, []);
    console.log(`PASS: clipboard keyboard/toggle/image limit/manual/offline/sensitive/conflict, Light/Dark and 150% text; ${out}`);
  } finally { if (browser) await browser.close(); await new Promise(resolve => server.close(resolve)); }
})().catch(error => { console.error(error); process.exitCode = 1; });
