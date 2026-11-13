// CCS/1 CHECK:real QUIC + production snapshot/rate model + real Chromium transfer renderer, 3 paired samples.
const {spawn} = require('node:child_process');
const fs = require('node:fs/promises'), path = require('node:path'), os = require('node:os'), http = require('node:http');
const readline = require('node:readline'), assert = require('node:assert/strict');
const {chromium} = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const frontend = path.resolve(__dirname, '../frontend');
const root = path.join(os.tmpdir(), 'windconnect-wn41-perf-' + require('node:crypto').randomUUID());
const server = http.createServer(async (req, res) => {
  const name = new URL(req.url, 'http://x').pathname;
  if (name === '/') {
    res.setHeader('Content-Type', 'text/html');
    res.end('<html><head><link rel="stylesheet" href="/tokens.css"><link rel="stylesheet" href="/style.css"><link rel="stylesheet" href="/panel.css"><link rel="stylesheet" href="/transfers.css"></head><body><section id="transfers"><h2 id="transfers-title"></h2><p id="transfers-summary"></p><div id="transfers-total"></div><button id="transfers-clear">Wyczyść</button><ul id="transfers-list" class="transfer-list"></ul></section><script type="module">import {createTransfers} from "/transfers.js";window.render=createTransfers(document,()=>{});</script></body></html>'); return;
  }
  const file = path.resolve(frontend, '.' + name);
  if (!file.startsWith(frontend + path.sep)) { res.writeHead(403).end(); return; }
  try { res.setHeader('Content-Type', name.endsWith('.js') ? 'text/javascript' : name.endsWith('.css') ? 'text/css' : 'application/octet-stream'); res.end(await fs.readFile(file)); }
  catch { res.writeHead(404).end(); }
});
let browser, child;
(async () => { try {
  await fs.mkdir(root, {recursive:true});
  await new Promise(r => server.listen(0,'127.0.0.1',r));
  browser = await chromium.launch({headless:true}); const page = await browser.newPage({viewport:{width:420,height:640}});
  await page.goto(`http://127.0.0.1:${server.address().port}`); await page.waitForFunction(() => !!window.render);
  const errors=[];page.on('pageerror',e=>errors.push(e.message));
  child = spawn(path.resolve('target/release/examples/transfers_bench.exe'),[root],{windowsHide:true,stdio:['ignore','pipe','pipe']});
  let stderr='', chain=Promise.resolve();const runs=new Map();
  child.stderr.on('data',d=>stderr+=d);
  const lines = readline.createInterface({input:child.stdout});
  for await (const line of lines) {
    const event=JSON.parse(line);
    chain=chain.then(async()=>{
      if(event.event==='start') {runs.set(event.run,{...event,receivedAt:performance.now(),events:0,progress:new Map(),intervals:[]});return;}
      const run=runs.get(event.run);
      if(event.event==='snapshot') {
        run.events++;
        for(const row of event.snapshot.rows){
          const previous=run.progress.get(row.key);
          if(previous?.state==='running'&&row.state==='running'&&row.done!==previous.done)run.intervals.push(event.timeMs-previous.at);
          if(!previous||row.done!==previous.done||row.state!==previous.state)run.progress.set(row.key,{state:row.state,done:row.done,at:event.timeMs});
        }
        await page.evaluate(snapshot=>window.render(snapshot),event.snapshot);
      } else {
        run.seconds=event.seconds;run.uiSeconds=Math.max(event.seconds,(performance.now()-run.receivedAt)/1000);
      }
    });
  }
  const exit = await new Promise(resolve => child.exitCode == null ? child.once('exit',resolve) : resolve(child.exitCode));
  await chain; assert.equal(exit,0,stderr);assert.deepEqual(errors,[]);
  const median=a=>[...a].sort((x,y)=>x-y)[1], summary={};
  const reportRuns=[...runs.values()].map(({progress,receivedAt,...r})=>r);
  for(const kind of ['large','small']){
    const baseline=reportRuns.filter(r=>r.kind===kind&&!r.observe), observed=reportRuns.filter(r=>r.kind===kind&&r.observe);
    assert.equal(baseline.length,3);assert.equal(observed.length,3);
    const base=median(baseline.map(r=>r.seconds)), withUi=median(observed.map(r=>r.seconds));
    summary[kind]={baselineMedianSeconds:base,observerAndUiMedianSeconds:withUi,ratio:base/withUi,uiDrainMedianSeconds:median(observed.map(r=>r.uiSeconds))};
  }
  const report={runs:reportRuns,summary,artifacts:root};await fs.writeFile(path.join(root,'performance.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report));
  for(const run of reportRuns.filter(r=>r.observe)) {
    assert(run.events <= Math.ceil(run.seconds*10)+2, `${run.run}: UI event burst`);
    assert(run.intervals.every(ms=>ms>=90), `${run.run}: progress events faster than 100ms cadence`);
  }
  for(const [kind,row] of Object.entries(summary))assert(row.ratio>=.95,`${kind}: throughput ratio ${row.ratio} below .95`);
} finally {
  if(child?.exitCode===null)child.kill();if(browser)await browser.close();server.close();
  // Remove only this run's generated payloads; raw measurement JSON remains for review.
  for (const name of ['source', ...['large','small'].flatMap(kind=>[0,1,2].flatMap(round=>[false,true].map(on=>`${kind}-${round}-${on}`)))]) {
    const target=path.resolve(root,name);assert(target.startsWith(path.resolve(root)+path.sep));
    await fs.rm(target,{recursive:true,force:true});
  }
} })().catch(e=>{console.error(e);console.error('artifacts:',root);process.exitCode=1;});
