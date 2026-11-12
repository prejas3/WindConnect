// CCS/1 SOURCE:original Orbita SVGs; every raster is rendered directly at its target size, never resized from another bitmap.
// DEP:external dev-only Playwright/Chromium; no runtime dependency. --check compares every output without writing.
const fs = require('node:fs/promises');
const path = require('node:path');
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const root = path.resolve(__dirname, '..');
const check = process.argv.includes('--check');
const windows = 'apps/windows/src-tauri/icons';
const android = 'apps/android/app/src/main/res';
const sizes = [16,20,24,32,40,48,64,96,128,256];
const outputs = [];
async function emit(file, data) {
  const bytes = Buffer.isBuffer(data) ? data : Buffer.from(data);
  if (check) assert.deepEqual(await fs.readFile(path.join(root,file)), bytes, `regenerate ${file}`);
  else { await fs.mkdir(path.dirname(path.join(root,file)),{recursive:true}); await fs.writeFile(path.join(root,file),bytes); }
  outputs.push({file,bytes:bytes.length,sha256:crypto.createHash('sha256').update(bytes).digest('hex')});
}
function iconDirectory(images) {
  const directory = Buffer.alloc(6+16*images.length); directory.writeUInt16LE(1,2); directory.writeUInt16LE(images.length,4);
  let offset=directory.length;
  images.forEach(({size,png},i)=>{ const p=6+i*16; directory[p]=size===256?0:size;directory[p+1]=directory[p];directory.writeUInt16LE(1,p+4);directory.writeUInt16LE(32,p+6);directory.writeUInt32LE(png.length,p+8);directory.writeUInt32LE(offset,p+12);offset+=png.length; });
  return Buffer.concat([directory,...images.map(i=>i.png)]);
}
const paths = svg => [...svg.matchAll(/<path stroke-width="([\d.]+)" d="([^"]+)"\/>/g)].map(m=>({width:m[1],d:m[2]}));
function vector(svg, viewport, scale, offset) {
  return `<vector xmlns:android="http://schemas.android.com/apk/res/android" android:width="${viewport}dp" android:height="${viewport}dp" android:viewportWidth="${viewport}" android:viewportHeight="${viewport}">\n  <group android:translateX="${offset}" android:translateY="${offset}" android:scaleX="${scale}" android:scaleY="${scale}">\n${paths(svg).map(p=>`    <path android:fillColor="#00000000" android:strokeColor="#FFFFFFFF" android:strokeWidth="${p.width}" android:strokeLineCap="round" android:strokeLineJoin="round" android:pathData="${p.d}" />`).join('\n')}\n  </group>\n</vector>\n`;
}
function wrap(svg, ink, background, round=false) {
  const unit=svg.includes('viewBox="0 0 16 16"')?16:64;
  return svg.replace('<svg ',`<svg style="color:${ink}" `).replace(/(<svg[^>]+>)/,`$1${background ? `<rect width="${unit}" height="${unit}" rx="${unit/(round?2:4)}" fill="${background}" stroke="none"/>`:''}`);
}
function inset(svg, unit, scale) {
  const offset = unit * (1 - scale) / 2;
  return svg.replace(/(<svg[^>]+>)/, `$1<g transform="translate(${offset} ${offset}) scale(${scale})">`)
    .replace('</svg>', '</g></svg>');
}
(async()=>{
  const master=await fs.readFile(path.join(root,'assets/brand/orbita.svg'),'utf8');
  const small=await fs.readFile(path.join(root,'assets/brand/orbita-small.svg'),'utf8');
  assert.equal(paths(master).length,4); assert.equal(paths(small).length,4);
  await emit('apps/windows/frontend/assets/orbita-small.svg',small);
  let browser;
  try {
    browser=await chromium.launch({headless:true}); const page=await browser.newPage();
    async function raster(svg,size) {
      return await page.evaluate(async({svg,size})=>{
        const image=new Image(); image.src='data:image/svg+xml;base64,'+btoa(svg.replace('<svg ',`<svg width="${size}" height="${size}" `)); await image.decode();
        const canvas=document.createElement('canvas');canvas.width=canvas.height=size;const ctx=canvas.getContext('2d');ctx.drawImage(image,0,0,size,size);
        return {png:canvas.toDataURL('image/png').split(',')[1],rgba:Array.from(ctx.getImageData(0,0,size,size).data)};
      },{svg,size});
    }
    const images=[];
    for(const size of sizes) { const unit=size<=32?16:64; const r=await raster(wrap(inset(size<=32?small:master,unit,.75),'#ffffff','#0d7f74'),size);const png=Buffer.from(r.png,'base64');images.push({size,png});await emit(`${windows}/app-${size}.png`,png); }
    await emit(`${windows}/icon.ico`,iconDirectory(images));
    for(const theme of ['light','dark']) for(const state of ['connected','searching','unavailable']) {
      const color=({light:{connected:'#0d7f74',searching:'#805015',unavailable:'#52686e'},dark:{connected:'#43c9b8',searching:'#efbc7c',unavailable:'#a1b2b6'}})[theme][state];
      let mark=small;
      if(state!=='connected') mark=mark.replace(/  <path stroke-width="1.5" d="m9.5[^\n]+\n/,'');
      if(state==='searching') mark=mark.replace('stroke-width="1.5"','stroke-dasharray="1.7 1.8" stroke-width="1.5"');
      if(state==='unavailable') mark=mark.replace('</svg>','  <path stroke-width="1.5" d="M4 12 12 4"/>\n</svg>');
      for(const size of [16,20,24,32]) {const r=await raster(wrap(mark,color),size);await emit(`${windows}/tray/${state}-${theme}-${size}.png`,Buffer.from(r.png,'base64'));await emit(`${windows}/tray/${state}-${theme}-${size}.rgba`,Buffer.from(r.rgba));}
    }
    // CCS/1 WHY:108dp layers are cropped to a ~72dp mask; a 48dp mark leaves visual padding inside the 66dp safe zone.
    await emit(`${android}/drawable/ic_launcher_foreground.xml`,vector(master,108,.75,30));
    await emit(`${android}/drawable/ic_launcher_monochrome.xml`,vector(master,108,.75,30));
    await emit(`${android}/drawable/ic_connection.xml`,vector(small,24,1.25,2));
    for(const name of ['ic_launcher','ic_launcher_round']) await emit(`${android}/mipmap-anydpi-v26/${name}.xml`,'<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">\n    <background android:drawable="@color/icon_background" />\n    <foreground android:drawable="@drawable/ic_launcher_foreground" />\n    <monochrome android:drawable="@drawable/ic_launcher_monochrome" />\n</adaptive-icon>\n');
    for(const [density,size] of [['mdpi',48],['hdpi',72],['xhdpi',96],['xxhdpi',144],['xxxhdpi',192]]) for(const name of ['ic_launcher','ic_launcher_round']) {const r=await raster(wrap(inset(master,64,2/3),'#ffffff','#0d7f74',name.endsWith('round')),size);await emit(`${android}/mipmap-${density}/${name}.png`,Buffer.from(r.png,'base64'));}
    // CCS/1 RULE:manifest excludes itself; hashes prove every committed output was regenerated from this source/toolchain.
    await emit('assets/brand/generated.json',JSON.stringify({renderer:`Chromium ${browser.version()}`,source:'Orbita / WN-93 direction C',outputs},null,2)+'\n');
    console.log(`${check?'Verified':'Generated'} ${outputs.length} assets directly from SVG.`);
  } finally {if(browser)await browser.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
