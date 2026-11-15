// CCS/1 CHECK:WN-51 panel status, text-only peer name, local withdrawal, Light/Dark at150%; IPC stub, no OS injection.
const assert = require('node:assert/strict');
const http = require('node:http');
const fs = require('node:fs/promises');
const path = require('node:path');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const root = path.resolve(__dirname, '../frontend');
const out = process.env.UI_ARTIFACTS || path.join(require('node:os').tmpdir(), 'WindConnect-WN51-ui');
const server = http.createServer(async (req, res) => {
  const file = path.resolve(root, '.' + new URL(req.url, 'http://x').pathname);
  if (!file.startsWith(root + path.sep)) return res.writeHead(403).end();
  try { res.setHeader('Content-Type', ({'.html':'text/html','.js':'text/javascript','.css':'text/css','.ttf':'font/ttf'})[path.extname(file)] || 'text/plain');res.end(await fs.readFile(file)); }
  catch { res.writeHead(404).end(); }
});
(async () => {
  let browser;
  try {
    await fs.mkdir(out,{recursive:true});await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
    browser=await chromium.launch({headless:true});
    for(const theme of ['light','dark']) {
      const page=await browser.newPage({viewport:{width:480,height:900}});
      const errors=[];page.on('pageerror',e=>errors.push(e.message));
      await page.addInitScript(({theme})=>{
        localStorage.setItem('windconnect-theme',theme);
        const state={phase:'connected',phone:'S25',connected:true,devices:[],notifications:[],missed_calls:[],qr:null,code:null,message:'Połączono',paired:true,find:{available:false,state:'idle'},touchpad:{allowed:true,active:true,message:'Telefon steruje kursorem · <img src=x>'}};
        window.__calls=[];window.__listeners={};
        window.__TAURI__={core:{invoke:async(cmd,args)=>{
          window.__calls.push([cmd,args]);
          if(cmd==='pairing_snapshot')return structuredClone(state);
          if(cmd==='files_snapshot')return {rows:[],active:0,activeDone:0,activeSize:0,phone:null};
          if(cmd==='panel_snapshot')return {shortcut_error:false};
          if(cmd==='clipboard_snapshot')return {enabled:true,status:'',shortcut_error:false};
          if(cmd==='pairing_action'&&args.action.startsWith('touchpad-')){
            state.touchpad={allowed:args.action==='touchpad-on',active:false,message:''};
            for(const cb of window.__listeners['pairing-state']||[])cb({payload:structuredClone(state)});
          }
          return null;
        }},event:{listen:async(name,cb)=>{(window.__listeners[name]||=[]).push(cb);return()=>{};}}};
      },{theme});
      await page.goto(`http://127.0.0.1:${server.address().port}/panel.html`);
      await page.locator('#touchpad-status').filter({hasText:'Telefon steruje kursorem'}).waitFor();
      assert.equal(await page.locator('#touchpad-status img').count(),0);
      await page.addStyleTag({content:'html {font-size:150%}'});
      await page.screenshot({path:path.join(out,`${theme}-150.png`),fullPage:true});
      await page.getByRole('button',{name:'Wyłącz sterowanie',exact:true}).click();
      await page.getByRole('button',{name:'Włącz sterowanie',exact:true}).waitFor();
      assert.equal(await page.locator('#touchpad-status').textContent(),'Touchpad i klawiatura z telefonu są wyłączone');
      await page.locator('#touchpad-toggle').click();
      await page.getByRole('button',{name:'Wyłącz sterowanie',exact:true}).waitFor();
      const calls=await page.evaluate(()=>window.__calls.filter(([c])=>c==='pairing_action'));
      assert.deepEqual(calls.map(([,a])=>a.action),['touchpad-off','touchpad-on']);assert.deepEqual(errors,[]);
      assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false);
      await page.close();
    }
    console.log(JSON.stringify({pass:true,themes:['light','dark'],font:'150%',artifacts:out}));
  } finally {await browser?.close();await new Promise(resolve=>server.close(resolve));}
})().catch(error=>{console.error(error);process.exitCode=1;});
