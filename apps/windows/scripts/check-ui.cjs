// CCS/1 PURPOSE:behavior/accessibility regression | DEP:external dev-only Playwright 1.61.1.
const assert = require('node:assert/strict');
const http = require('node:http');
const fs = require('node:fs/promises');
const path = require('node:path');
const os = require('node:os');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const root = path.resolve(__dirname, '../frontend');
const artifacts = process.env.UI_ARTIFACTS || path.join(os.tmpdir(), 'windconnect-wn21-ui');
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.ttf': 'font/ttf' };
const server = http.createServer(async (request, response) => {
  const pathname = new URL(request.url, 'http://localhost').pathname;
  const filename = path.resolve(root, '.' + (pathname === '/' ? '/index.html' : pathname));
  if (!filename.startsWith(root + path.sep)) { response.writeHead(403).end(); return; }
  try {
    response.setHeader('Content-Security-Policy', "default-src 'self'; script-src 'self'; style-src 'self'; font-src 'self'; img-src 'self' data:; connect-src ipc: http://ipc.localhost");
    response.setHeader('Content-Type', types[path.extname(filename)] || 'text/plain');
    response.end(await fs.readFile(filename));
  } catch { response.writeHead(404).end(); }
});

(async () => {
  await fs.mkdir(artifacts, { recursive: true });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const url = `http://127.0.0.1:${server.address().port}/?gallery`;
  let browser;
  try {
    browser = await chromium.launch({ headless: true });
    const page = await browser.newPage({ viewport: { width: 1024, height: 900 } });
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(url);
    await page.locator('#gallery:not([hidden])').waitFor();
    await page.evaluate(() => document.fonts.ready);
    assert.equal(await page.evaluate(() => document.fonts.check('13px "DM Sans"')), true);
    const font = await page.evaluate(() => [...document.fonts].find(font => font.family === '"DM Sans"' || font.family === 'DM Sans')?.status);
    assert.equal(font, 'loaded');
    for (const theme of ['light', 'dark']) {
      await page.locator(`[data-theme-choice="${theme}"]`).click();
      assert.equal(await page.locator('html').getAttribute('data-theme'), theme);
      for (const width of [400, 700, 1024]) {
        await page.setViewportSize({ width, height: 900 });
        assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, `overflow ${theme}/${width}`);
        await page.screenshot({ path: path.join(artifacts, `${theme}-${width}.png`), fullPage: true });
      }
    }
    await page.locator('[data-theme-choice="light"]').click();
    const switchControl = page.locator('#gallery [role="switch"]').first();
    await switchControl.focus(); await page.keyboard.press('Space');
    assert.equal(await switchControl.getAttribute('aria-checked'), 'false');
    await page.keyboard.press('Enter');
    assert.equal(await switchControl.getAttribute('aria-checked'), 'true');
    const disabled = page.locator('#gallery [role="switch"][disabled]');
    assert.equal(await disabled.isDisabled(), true);
    await page.locator('.checkbox').focus(); await page.keyboard.press('Space');
    assert.equal(await page.locator('.checkbox').isChecked(), false);
    await page.locator('#sample-form button').click();
    assert.equal(await page.locator('#device-name').getAttribute('aria-invalid'), 'true');
    assert.equal(await page.locator('#device-name').evaluate(element => element === document.activeElement), true);
    await page.locator('#device-name').fill('Zażółć gęślą jaźń');
    await page.locator('#sample-form button').click();
    assert.equal(await page.locator('#device-name').getAttribute('aria-invalid'), 'false');
    assert.equal(await page.locator('#name-error').isHidden(), true);
    const options = page.locator('[role="option"]');
    await options.nth(0).focus(); await page.keyboard.press('End');
    assert.equal(await options.nth(1).getAttribute('aria-selected'), 'true');
    await page.keyboard.press('ArrowDown');
    assert.equal(await options.nth(0).getAttribute('aria-selected'), 'true');
    assert.equal(await options.nth(2).getAttribute('aria-selected'), 'false');
    assert.equal(await options.nth(0).evaluate(element => getComputedStyle(element).outlineStyle), 'solid');
    await page.locator('#open-dialog').click();
    assert.equal(await page.locator('#main').evaluate(element => element.inert), true);
    await page.keyboard.press('Shift+Tab');
    assert.equal(await page.locator('[data-confirm]').evaluate(element => element === document.activeElement), true);
    await page.keyboard.press('Tab');
    assert.equal(await page.locator('[data-dismiss]').evaluate(element => element === document.activeElement), true);
    await page.keyboard.press('Escape');
    assert.equal(await page.locator('#overlay').isHidden(), true);
    assert.equal(await page.locator('#open-dialog').evaluate(element => element === document.activeElement), true);
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.locator('#replay-motion').click();
    assert.equal(await page.locator('#motion-preview').evaluate(element => getComputedStyle(element).animationName), 'none');
    await page.locator('#complete-progress').click();
    assert.equal(await page.locator('#sample-progress').getAttribute('aria-valuenow'), '100');
    assert.equal(await page.locator('#sample-progress span').evaluate(element => getComputedStyle(element).transitionDuration), '0s');
    await page.emulateMedia({ forcedColors: 'active' });
    await page.screenshot({ path: path.join(artifacts, 'forced-colors.png'), fullPage: true });
    // Native release reply wins over localhost and the query parameter.
    const release = await browser.newPage();
    await release.addInitScript(() => { window.__TAURI__ = { core: { invoke: async command => command === 'ui_dev_mode' ? false : command === 'core_version' ? 'release-test' : false } }; });
    await release.goto(url);
    await release.waitForFunction(() => document.querySelector('#status').textContent.includes('release-test'));
    assert.equal(await release.locator('#gallery-toggle').isHidden(), true);
    assert.equal(await release.locator('#gallery').isHidden(), true);
    assert.equal(await release.locator('#gallery').textContent(), '');
    assert.equal(await release.locator('#home').isVisible(), true);
    assert.deepEqual(errors, []);
    const menu = await browser.newPage({ viewport: { width: 280, height: 250 } });
    await menu.addInitScript(() => { window.menuCalls = []; window.__TAURI__ = { core: { invoke: async (command,args) => { window.menuCalls.push([command,args]); if (command === 'tray_snapshot') return { phase:'searching' }; if (command === 'autostart_enabled') return false; if (command === 'set_autostart') return args.enabled; } } }; });
    await menu.goto(url.replace('/?gallery','/tray-menu.html'));
    await menu.waitForFunction(() => document.querySelector('#tray-state').textContent === 'Szukam urządzenia');
    await menu.locator('[data-action="open"]').focus(); await menu.keyboard.press('ArrowDown');
    assert.equal(await menu.locator('#autostart').evaluate(e => e === document.activeElement),true);
    await menu.keyboard.press('Space');
    await menu.waitForFunction(() => document.querySelector('#autostart').getAttribute('aria-checked') === 'true');
    await menu.keyboard.press('Escape');
    assert.equal(await menu.evaluate(() => window.menuCalls.some(([command,args]) => command === 'tray_action' && args.action === 'hide')),true);
    await menu.locator('[data-action="quit"]').click();
    assert.equal(await menu.evaluate(() => window.menuCalls.some(([command,args]) => command === 'tray_action' && args.action === 'quit')),true);
    await menu.screenshot({path:path.join(artifacts,'tray-menu.png')});
    console.log(`PASS: themes, 6 layouts, DM Sans, keyboard, validation, list, dialog focus, reduced motion, release gate; screenshots: ${artifacts}`);
  } finally {
    if (browser) await browser.close();
    await new Promise(resolve => server.close(resolve));
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
