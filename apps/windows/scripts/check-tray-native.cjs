// CCS/1 PURPOSE:real WebView2, QUIC session lifecycle, native tray timing and window ownership.
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const { spawn } = require('node:child_process');
const assert = require('node:assert/strict');
const path = require('node:path');
const executable = path.resolve(process.argv[2] || 'target/debug/windconnect-windows.exe');
const release = process.argv.includes('--release');
const port = 19323;
const child = spawn(executable, ['--autostart'], { windowsHide:true, stdio:'ignore', env:{...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port}`, WEBVIEW2_USER_DATA_FOLDER:path.join(process.env.TEMP,'windconnect-wn23-'+(release?'release':'debug'))} });
let exited = false; child.once('exit', () => { exited=true; });
const delay = milliseconds => new Promise(resolve => setTimeout(resolve,milliseconds));
(async()=>{
  let browser, second;
  try {
    let ready=false;
    for(let attempt=0;attempt<80;attempt++){try{if((await fetch(`http://127.0.0.1:${port}/json/version`)).ok){ready=true;break}}catch{}await delay(250);}
    assert(ready,'WebView2 debugger started');
    browser=await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
    let main,menu;
    for(let attempt=0;attempt<60;attempt++){
      const pages=browser.contexts().flatMap(context=>context.pages());
      menu=pages.find(page=>page.url().includes('tray-menu.html'));
      main=pages.find(page=>page.url().includes('tauri.localhost')&&!/tray-menu.html|panel.html/.test(page.url()));
      if(main&&menu)break;await delay(100);
    }
    assert(main&&menu,'both app webviews created');
    await main.waitForFunction(()=>document.querySelector('#status')?.textContent.includes('gotowy'));
    await main.waitForFunction(async()=>{try{await window.__TAURI__.core.invoke('tray_snapshot');return true;}catch{return false;}});
    const invoke=(page,command,args)=>page.evaluate(({command,args})=>window.__TAURI__.core.invoke(command,args),{command,args});
    let state=await invoke(main,'tray_snapshot'); assert.equal(state.main_visible,false,'login startup has no main window flash');
    const samples=[];
    if(!release){
      for(let index=0;index<5;index++){
        const result=await invoke(main,'dev_tray_probe');
        assert.deepEqual(result.map(value=>value.phase),['searching','connected','unavailable']);
        for(const value of result){assert.equal(value.error,null);assert(value.latency_us<100000,`native tray update took ${value.latency_us}us`);samples.push(value.latency_us);}
      }
    }else{
      assert.equal(await main.evaluate(()=>window.__TAURI__.core.invoke('dev_tray_probe').then(()=>false,()=>true)),true,'release denies synthetic probe');
    }
    await invoke(main,'tray_action',{action:'open'});
    assert.equal((await invoke(main,'tray_snapshot')).main_visible,true);
    await invoke(main,'window_action',{action:'close'});
    await main.waitForFunction(async()=>!(await window.__TAURI__.core.invoke('tray_snapshot')).main_visible);
    second=spawn(executable,[],{windowsHide:true,stdio:'ignore'});
    const secondExit=await Promise.race([new Promise(resolve=>second.once('exit',resolve)),delay(5000).then(()=>{throw Error('second instance stayed alive')})]);
    assert.equal(secondExit,0);
    await main.waitForFunction(async()=>(await window.__TAURI__.core.invoke('tray_snapshot')).main_visible);
    await invoke(main,'tray_action',{action:'menu'});
    await menu.waitForFunction(async()=>(await window.__TAURI__.core.invoke('tray_snapshot')).menu_visible);
    await menu.waitForFunction(()=>!document.querySelector('#autostart').disabled);
    await menu.locator('[data-action="open"]').focus();await menu.keyboard.press('ArrowDown');
    assert.equal(await menu.locator('#autostart').evaluate(element=>element===document.activeElement),true);
    await menu.keyboard.press('Escape');
    await main.waitForFunction(async()=>!(await window.__TAURI__.core.invoke('tray_snapshot')).menu_visible);
    console.log(JSON.stringify({release,sessionTransitions:samples.length,maxNativeUpdateUs:samples.length?Math.max(...samples):null,quietStartup:true,closeHides:true,secondInstanceRestores:true,customMenuKeyboard:true}));
    await invoke(main,'tray_action',{action:'quit'}).catch(()=>{});
    for(let attempt=0;attempt<40&&!exited;attempt++)await delay(50);
    assert(exited,'Quit terminates process');
  } finally {
    if(browser)await browser.close();
    if(second&&second.exitCode===null)second.kill();
    if(!exited)child.kill();
    if(!exited)await Promise.race([new Promise(resolve=>child.once('exit',resolve)),delay(5000)]);
  }
})().catch(error=>{console.error(error);process.exitCode=1;});
