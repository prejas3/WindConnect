// CCS/1 CHECK:committed icon hashes/container dimensions and target-size Light/Dark proof sheet; no app launch or OS settings changed.
const fs=require('node:fs/promises'),path=require('node:path'),assert=require('node:assert/strict'),crypto=require('node:crypto');
const {chromium}=require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const root=path.resolve(__dirname,'..'),out=process.env.ICON_ARTIFACTS || path.join(process.env.TEMP,'windconnect-wn93-icons');
// CCS/1 RULE:the selected empty centre must survive every raster/container; unavailable intentionally carries a slash.
async function checkCentres(page, manifest) {
  let checked=0;
  for(const item of manifest.outputs.filter(i=>i.file.endsWith('.png'))) {
    const uri='data:image/png;base64,'+(await fs.readFile(path.join(root,item.file))).toString('base64');
    const pixels=await page.evaluate(async uri=>{const image=new Image();image.src=uri;await image.decode();const c=document.createElement('canvas');c.width=image.width;c.height=image.height;const g=c.getContext('2d');g.drawImage(image,0,0);return {size:c.width,data:Array.from(g.getImageData(0,0,c.width,c.height).data)};},uri);
    if(item.file.includes('/tray/')) assert.deepEqual(Buffer.from(pixels.data),await fs.readFile(path.join(root,item.file.replace(/\.png$/,'.rgba'))),`PNG/RGBA mismatch ${item.file}`);
    if(item.file.includes('unavailable-')) continue;
    const transparent=item.file.includes('/tray/'),lo=Math.floor(pixels.size*.375),hi=Math.ceil(pixels.size*.625);
    for(let y=lo;y<hi;y++) for(let x=lo;x<hi;x++) {
      const p=(y*pixels.size+x)*4,[r,g,b,a]=pixels.data.slice(p,p+4);
      assert(transparent?a<=16:!(a>16&&r>200&&g>200&&b>200),`centre contains old device ink: ${item.file} at ${x},${y}`);
    }
    checked++;
  }
  console.log(`PASS ${checked} empty-centre rasters (threshold alpha16 / white200),24 PNG/RGBA pairs`);
}
(async()=>{
  const manifest=JSON.parse(await fs.readFile(path.join(root,'assets/brand/generated.json')));
  for(const item of manifest.outputs){const bytes=await fs.readFile(path.join(root,item.file));assert.equal(bytes.length,item.bytes);assert.equal(crypto.createHash('sha256').update(bytes).digest('hex'),item.sha256);}
  const ico=await fs.readFile(path.join(root,'apps/windows/src-tauri/icons/icon.ico'));
  assert.equal(ico.readUInt16LE(2),1);assert.equal(ico.readUInt16LE(4),10);
  for(let i=0;i<10;i++){const p=6+i*16,size=ico[p] || 256,offset=ico.readUInt32LE(p+12),length=ico.readUInt32LE(p+8);assert.equal(ico.readUInt16LE(p+6),32);assert.equal(ico.readUInt32BE(offset+16),size);assert.equal(ico.readUInt32BE(offset+20),size);assert.deepEqual(ico.subarray(offset,offset+length),await fs.readFile(path.join(root,`apps/windows/src-tauri/icons/app-${size}.png`)),`ICO ${size}px payload`);}
  const url=async file=>'data:image/png;base64,'+(await fs.readFile(path.join(root,'apps/windows/src-tauri/icons',file))).toString('base64');
  let rows='';
  for(const theme of ['light','dark']){
    rows+=`<section class="${theme}"><h2>${theme==='light'?'Jasny pasek':'Ciemny pasek'}</h2>`;
    for(const state of ['connected','searching','unavailable']){
      rows+=`<div class="row"><strong>${{connected:'Połączono',searching:'Szukanie',unavailable:'Brak połączenia'}[state]}</strong>`;
      for(const size of [16,20,24,32])rows+=`<figure><img width="${size}" height="${size}" src="${await url(`tray/${state}-${theme}-${size}.png`)}"><figcaption>${size}px / ${size/16*100}%</figcaption></figure>`;
      rows+='</div>';
    }
    rows+='</section>';
  }
  await fs.mkdir(out,{recursive:true});
  const font=(await fs.readFile(path.join(root,'apps/windows/frontend/assets/dm-sans.ttf'))).toString('base64');
  const html=`<!doctype html><meta charset="utf-8"><style>@font-face{font-family:DM;src:url(data:font/ttf;base64,${font})}*{box-sizing:border-box}body{font-family:DM,sans-serif;background:#eef4f4;color:#15282f;margin:0;padding:32px}h1{font-size:30px;margin:0 0 8px}p{font-size:13px;color:#52686e;margin:0 0 26px}.grid{display:grid;grid-template-columns:1fr 1fr;gap:24px}section{border-radius:18px;padding:24px}.light{background:#f8fbfb}.dark{background:#16242b;color:#e9f1f1}h2{font-size:18px;margin:0 0 16px}.row{display:flex;align-items:center;border-top:1px solid #52686e44;padding:16px 0;gap:22px}.row strong{width:100px;font-size:13px}figure{margin:0;width:72px;height:70px;display:flex;align-items:center;justify-content:end;flex-direction:column;gap:14px}figcaption{font-size:10px;white-space:nowrap}img{display:block}.app{display:flex;align-items:center;gap:30px;margin-top:30px}small{color:#52686e}.app figure{width:auto;height:auto;gap:12px}</style><h1>Orbita / zasoby docelowe</h1><p>PNG w skali 1:1, wyrenderowane osobno z wektora. Różne sylwetki stanów, bez polegania na samym kolorze.</p><div class="grid">${rows}</div><div class="app">${await Promise.all([16,32,48,64,128,256].map(async size=>`<figure><img src="${await url(`app-${size}.png`)}" width="${size}" height="${size}"><figcaption>ICO ${size}px</figcaption></figure>`)).then(a=>a.join(''))}</div>`;
  await fs.writeFile(path.join(out,'icons.html'),html);
  let browser;try{browser=await chromium.launch({headless:true});const page=await browser.newPage({viewport:{width:1320,height:920},deviceScaleFactor:1});await checkCentres(page,manifest);await page.setContent(html);await page.evaluate(()=>document.fonts.ready);await page.screenshot({path:path.join(out,'icons.png'),fullPage:true});}finally{if(browser)await browser.close();}
  console.log(`PASS ${manifest.outputs.length} hashes, 10 ICO entries, target-size tray sheet; ${out}`);
})().catch(e=>{console.error(e);process.exitCode=1});
