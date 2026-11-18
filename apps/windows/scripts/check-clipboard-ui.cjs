// CCS/1 PURPOSE:clipboard UI commands/status, the image limit (WN-36), the history (WN-37), keyboard, themes and larger text; IPC stub does not prove OS delivery.
const assert = require('node:assert/strict');
const http = require('node:http');
const fs = require('node:fs/promises');
const path = require('node:path');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const root = path.resolve(__dirname, '../frontend');
const out = process.env.UI_ARTIFACTS || path.join(require('node:os').tmpdir(), 'windconnect-wn35-ui');
// The app's own CSP (WN-37: thumbnails need img-src data:, nothing else does).
const csp = require('../src-tauri/tauri.conf.json').app.security.csp;
assert.match(csp, /img-src 'self' data:;/); assert.doesNotMatch(csp.replace(/img-src[^;]*;/, ''), /data:/, 'data: only for images');
const server = http.createServer(async (req, res) => {
  res.setHeader('Content-Security-Policy', csp);
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
    const page = await browser.newPage({ locale: 'pl-PL' });
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.addInitScript(() => {
      const state = {enabled:true,status:'Synchronizacja automatyczna',shortcut_error:false,image_limit_mb:16};
      window.calls = [];
      // WN-37: a history as the app sends it (previews and a thumbnail only).
      const pixel = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkqPhfDwAEZQH/0cGSPwAAAABJRU5ErkJggg==';
      let history = [
        {id:3,kind:'text',direction:'to-phone',at:Date.UTC(2026,10,12,9,30),pinned:false,preview:'<img src=x onerror="window.injected=1"> Lista zakupów: mleko, chleb',thumbnail:null,width:null,height:null,ready:true},
        {id:2,kind:'image',direction:'from-phone',at:Date.UTC(2026,10,12,9,20),pinned:false,preview:null,thumbnail:pixel,width:1920,height:1080,ready:true},
        {id:1,kind:'image',direction:'from-phone',at:Date.UTC(2026,10,12,9,10),pinned:false,preview:null,thumbnail:pixel,width:800,height:600,ready:false},
      ];
      const emitHistory = () => window.historyEvent?.({payload:history.map(e => ({...e}))});
      window.__TAURI__ = {
        core: {invoke: async (command, args) => {
          window.calls.push([command, args]);
          if (command === 'ui_dev_mode') return false;
          if (command === 'core_version') return '0.38.0';
          if (command === 'clipboard_snapshot') return {...state};
          if (command === 'clipboard_history') return history.map(e => ({...e}));
          if (command === 'clipboard_action' && args.action.startsWith('history-')) {
            if (args.action === 'history-pin') history = history.map(e => e.id === args.id ? {...e, pinned: args.pinned} : e).sort((a, b) => (b.pinned - a.pinned) || (b.at - a.at));
            if (args.action === 'history-delete') history = history.filter(e => e.id !== args.id);
            if (args.action === 'history-clear') history = history.filter(e => e.pinned);
            if (args.action === 'history-copy') { state.status = 'Skopiowano z historii'; window.clipboardEvent?.({payload:{...state}}); }
            setTimeout(emitHistory, 0);
            return null;
          }
          if (command === 'clipboard_action') {
            if (args.action === 'enabled') state.enabled = args.enabled;
            if (args.action === 'send') state.status = 'Brak połączenia z telefonem';
            if (args.action === 'image-limit') state.image_limit_mb = args.limit;
            window.clipboardEvent?.({payload:{...state}});
          }
          return null;
        }},
        event: {listen: async (name, callback) => { if (name === 'clipboard-state') window.clipboardEvent = callback; if (name === 'clipboard-history') window.historyEvent = callback; return () => {}; }},
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
    // WN-37: the history. A preview is text, never markup.
    const history = page.locator('[data-clipboard-history]');
    await history.waitFor({state:'visible'});
    const list = history.locator('[data-history-list]');
    assert.equal(await list.locator('li').count(), 3);
    assert.equal(await list.locator('li').first().locator('img').count(), 0, 'preview markup is not rendered');
    assert.equal(await page.evaluate(() => window.injected), undefined);
    assert.match(await list.locator('li').first().textContent(), /<img src=x/);
    assert.match(await list.locator('li').nth(1).textContent(), /1920×1080/);
    assert.equal(await list.locator('li').nth(1).locator('img').evaluate(img => img.complete && img.naturalWidth), 1, 'thumbnail loads under the CSP');
    assert.equal(await list.locator('li').nth(2).locator('[data-history-copy]').isDisabled(), true, 'an image still arriving is not copyable');
    assert.match(await list.locator('li').nth(2).textContent(), /Obraz dochodzi/);
    await history.screenshot({path:path.join(out,'history.png')});
    // Keyboard: copy, then pin (focus stays on the pin of the same entry after the re-render), delete, clear.
    const copyFirst = list.locator('li').first().locator('[data-history-copy]');
    await copyFirst.focus(); await page.keyboard.press('Enter');
    await page.waitForFunction(() => document.querySelector('[data-clipboard-status]').textContent === 'Skopiowano z historii');
    await page.keyboard.press('Tab');
    assert.equal(await page.evaluate(() => document.activeElement.getAttribute('aria-label')), 'Przypnij');
    await page.keyboard.press('Space');
    await page.waitForFunction(() => document.querySelector('[data-history-list] li[data-id="3"] [data-history-pin]')?.getAttribute('aria-pressed') === 'true');
    assert.equal(await page.evaluate(() => document.activeElement.closest('li')?.dataset.id + ':' + document.activeElement.getAttribute('aria-label')), '3:Odepnij');
    await list.locator('li[data-id="2"] [data-history-delete]').focus(); await page.keyboard.press('Enter');
    await page.waitForFunction(() => !document.querySelector('[data-history-list] li[data-id="2"]'));
    await history.getByRole('button', {name:'Wyczyść'}).click();
    await page.waitForFunction(() => document.querySelectorAll('[data-history-list] li').length === 1);
    assert.equal(await history.getByRole('button', {name:'Wyczyść'}).isVisible(), false, 'nothing left to clear but pins');
    assert.deepEqual(await page.evaluate(() => window.calls.filter(([c, a]) => c === 'clipboard_action' && a.action.startsWith('history-'))), [
      ['clipboard_action',{action:'history-copy',id:3}],['clipboard_action',{action:'history-pin',id:3,pinned:true}],
      ['clipboard_action',{action:'history-delete',id:2}],['clipboard_action',{action:'history-clear'}],
    ]);
    await page.evaluate(() => window.historyEvent({payload:[]}));
    assert.equal(await history.locator('[data-history-empty]').isVisible(), true);
    assert.match(await history.locator('[data-history-empty]').textContent(), /Synchronizacja wyłączona: historia nie jest zapisywana/, 'sync is off here (toggled above)');
    // The panel: the history keeps the panel's width and scrolls on its own.
    const panel = await browser.newPage({ locale: 'pl-PL', viewport:{width:380,height:720}});
    panel.on('pageerror', error => errors.push(error.message));
    await panel.addInitScript(() => {
      const many = Array.from({length:22}, (_, i) => ({id:i+1,kind:'text',direction:i % 2 ? 'to-phone' : 'from-phone',at:Date.UTC(2026,10,12,9,i),pinned:i < 2,preview:`Pozycja ${i+1} `.repeat(12).trim(),thumbnail:null,width:null,height:null,ready:true}));
      window.__TAURI__ = {core:{invoke: async command => command === 'clipboard_history' ? many : command === 'clipboard_snapshot' ? {enabled:true,status:'Synchronizacja automatyczna',shortcut_error:false,image_limit_mb:16} : null},event:{listen: async () => () => {}}};
    });
    await panel.goto(`http://127.0.0.1:${server.address().port}/panel.html`);
    const panelList = panel.locator('[data-history-list]');
    await panelList.locator('li').first().waitFor();
    assert.equal(await panelList.locator('li').count(), 22);
    assert.equal(await panel.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, 'no sideways scroll');
    assert.equal(await panelList.evaluate(list => list.scrollHeight > list.clientHeight && list.clientHeight <= 248), true, 'own scroll');
    for (const theme of ['light','dark']) {
      await panel.evaluate(theme => { document.documentElement.dataset.theme = theme; }, theme);
      await panel.locator('[data-clipboard-history]').screenshot({path:path.join(out,`panel-history-${theme}.png`)});
    }
    await panel.close();
    assert.deepEqual(await page.evaluate(() => window.calls.filter(([c, a]) => c === 'clipboard_action' && !a.action.startsWith('history-'))), [
      ['clipboard_action',{action:'enabled',enabled:false}],['clipboard_action',{action:'image-limit',limit:50}],['clipboard_action',{action:'send'}],
    ]);
    assert.deepEqual(errors, []);
    console.log(`PASS: clipboard keyboard/toggle/image limit/manual/offline/sensitive/conflict, history (text-only previews, keyboard copy/pin/delete/clear, panel scroll), Light/Dark and 150% text; ${out}`);
  } finally { if (browser) await browser.close(); await new Promise(resolve => server.close(resolve)); }
})().catch(error => { console.error(error); process.exitCode = 1; });
