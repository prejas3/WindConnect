// CCS/1 PURPOSE:finite emulator-only real QR/TLS, Activity destruction, Doze and notification-stop evidence.
// SEC:isolated ephemeral PC identity; clears only the test app on a verified emulator. Installed PC app remains untouched.
const { spawn, spawnSync } = require('node:child_process');
const fs = require('node:fs/promises');
const path = require('node:path');
const os = require('node:os');
const crypto = require('node:crypto');
const dgram = require('node:dgram');
const assert = require('node:assert/strict');
const homeProof = process.env.HOME_SCREEN_PROOF === '1';
const root = path.join(os.tmpdir(), (homeProof ? 'windconnect-wn31-' : 'windconnect-wn29-') + crypto.randomUUID());
const adb = path.join(process.env.ANDROID_HOME, 'platform-tools/adb.exe');
const serial = process.env.ANDROID_SERIAL || 'emulator-5556';
const pkg = 'io.github.prejas3.windconnect';
const soakSeconds = Math.max(60, Math.min(1800, Number(process.env.SOAK_SECONDS || 60)));
const batteryExempt = process.env.BATTERY_EXEMPT !== '0';
const redirectPort = Number(process.env.DISCOVERY_REDIRECT_PORT || 47823);
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
function device(...args) {
  const result = spawnSync(adb, ['-s', serial, ...args], { windowsHide: true, encoding: 'utf8', timeout: 20000 });
  assert.equal(result.status, 0, result.stderr); return result.stdout;
}
let peer, instrument, socket, timer, redirected = false, output = '', wasExempt = false, emulatorVerified = false, discoveryPaused = false;
(async () => {
  try {
    assert.equal(device('shell', 'getprop', 'ro.kernel.qemu').trim(), '1', 'requires disposable emulator');
    emulatorVerified = true;
    wasExempt = device('shell', 'dumpsys', 'deviceidle', 'whitelist').includes(pkg);
    await fs.mkdir(root, { recursive: true });
    device('install', '-r', 'apps/android/app/build/outputs/apk/debug/app-debug.apk');
    device('install', '-r', '-t', 'apps/android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk');
    device('shell', 'am', 'force-stop', pkg);
    device('shell', 'pm', 'clear', pkg);
    device('emu', 'redir', 'add', 'udp:'+redirectPort+':47821'); redirected = true;
    peer = spawn(path.resolve('target/debug/examples/background_peer.exe'), [root], { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
    let peerOutput = ''; peer.stderr.on('data', data => { peerOutput += data; });
    for (let attempt = 0; attempt < 100; attempt++) { try { await fs.access(path.join(root, 'port.txt')); break; } catch {} assert.equal(peer.exitCode, null, peerOutput); await delay(50); }
    const offerFile = path.join(root, 'offer.txt'), imageFile = path.join(root, 'desktop-qr.png'), javaFile = path.join(root, 'EncodeQr.java');
    await fs.writeFile(javaFile, `import java.nio.file.*; import java.awt.image.BufferedImage; import javax.imageio.ImageIO; class EncodeQr { public static void main(String[] a) throws Exception { var rows=Files.readAllLines(Path.of(a[0])); int w=Integer.parseInt(rows.get(0)),scale=3,size=(w+8)*scale; var bits=rows.get(1); var b=new BufferedImage(size,size,BufferedImage.TYPE_INT_RGB); for(int y=0;y<size;y++) for(int x=0;x<size;x++) { int mx=x/scale-4,my=y/scale-4; boolean dark=mx>=0&&my>=0&&mx<w&&my<w&&bits.charAt(my*w+mx)=='1'; b.setRGB(x,y,dark?0:0xffffff); } ImageIO.write(b,"png",Path.of(a[1]).toFile()); } }`);
    const encoded = spawnSync(path.join(process.env.JAVA_HOME, 'bin/java.exe'), [javaFile, path.join(root,'matrix.txt'), imageFile], { windowsHide: true, encoding: 'utf8', timeout: 10000 });
    assert.equal(encoded.status, 0, encoded.stderr);
    const offer = new URL(await fs.readFile(offerFile, 'utf8'));
    const fingerprint = crypto.createHash('sha256').update(Buffer.concat([Buffer.from('302a300506032b6570032100','hex'), Buffer.from(offer.searchParams.get('k'),'base64url')])).digest();
    const packet = Buffer.alloc(39); packet.write('WCD1'); packet[4]=2; packet.writeUInt16BE(Number(await fs.readFile(path.join(root,'port.txt'),'utf8')),5); fingerprint.copy(packet,7);
    socket = dgram.createSocket({type:'udp4',reuseAddr:true});
    await new Promise((resolve,reject)=>{socket.once('error',reject);socket.bind({port:47821,address:'0.0.0.0',exclusive:false},resolve);});
    timer=setInterval(()=>socket.send(packet,redirectPort,'127.0.0.1'),150);
    device('push',imageFile,'/data/local/tmp/windconnect-wn29-qr.png');
    device('shell','run-as',pkg,'mkdir','-p','cache');
    device('shell',`cat /data/local/tmp/windconnect-wn29-qr.png | run-as ${pkg} tee cache/desktop-qr.png >/dev/null`);
    instrument=spawn(adb,['-s',serial,'shell','am','instrument','-w','-r','-e','class',pkg+'.PairingTest#qrConnectsRealDesktop','-e','desktopQr','true','-e','backgroundService',String(!homeProof),'-e','homeScreenProof',String(homeProof),'-e','backgroundBoot','true','-e','soakSeconds',String(soakSeconds),'-e','batteryExempt',String(batteryExempt),pkg+'.test/androidx.test.runner.AndroidJUnitRunner'],{windowsHide:true,stdio:['ignore','pipe','pipe']});
    instrument.stdout.on('data',data=>{output+=data; for(const line of data.toString().split('\n'))if(line.includes('soakProgress='))console.log(line.trim()); if(output.includes('backgroundDozeComplete=true') && !timer)timer=setInterval(()=>socket.send(packet,redirectPort,'127.0.0.1'),150);}); instrument.stderr.on('data',data=>{output+=data;});
    let connectedSamples=0;
    for(let attempt=0;attempt<(soakSeconds+180)*2 && instrument.exitCode===null;attempt++) {
      try { if((await fs.readFile(path.join(root,'connected.txt'),'utf8'))==='true') {connectedSamples++;if(!discoveryPaused){clearInterval(timer);timer=undefined;discoveryPaused=true;console.log('Public discovery fixture paused during Doze; only real QUIC traffic remains.');}} } catch {}
      await delay(500);
    }
    await fs.writeFile(path.join(root,'instrumentation.log'),output);
    assert(output.includes('OK (1 test)'),output);
    if (homeProof) {
      assert(output.includes('homeScreenProof=true'),output);
      assert.equal(await fs.readFile(path.join(root,'connected.txt'),'utf8'),'true','home recreation retains real Session');
      for (const theme of ['Light','Dark']) {
        const result=spawnSync(adb,['-s',serial,'exec-out','run-as',pkg,'cat',`cache/wn31-${theme}.png`],{windowsHide:true,timeout:10000});
        assert.equal(result.status,0);await fs.writeFile(path.join(root,`home-${theme}.png`),result.stdout);
      }
      const report={realHome:true,actualSession:true,themes:['Light','Dark'],recreation:true,disabledUnimplementedActions:true,artifacts:root};
      await fs.writeFile(path.join(root,'run.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report));return;
    }
    assert(output.includes('backgroundDozeSeconds='+soakSeconds) && output.includes('stopReleasesService=true'),output);
    assert(!batteryExempt || connectedSamples>=soakSeconds*1.8,`real PC session sampled ${connectedSamples} times`);
    const beforeBoot = Number(await fs.readFile(path.join(root,'sessions.txt'),'utf8'));
    device('reboot');
    let booted=false;
    for(let attempt=0;attempt<180;attempt++) {
      const result=spawnSync(adb,['-s',serial,'shell','getprop','sys.boot_completed'],{windowsHide:true,encoding:'utf8',timeout:2000});
      if(result.status===0 && result.stdout.trim()==='1') {booted=true;break;} await delay(500);
    }
    assert(booted,'emulator reboot completed');
    // Emulator console UDP redirections may be cleared by reboot.
    const redirections=device('emu','redir','list');
    if(!redirections.includes(String(redirectPort)))device('emu','redir','add','udp:'+redirectPort+':47821');
    let restored=false;
    for(let attempt=0;attempt<100;attempt++) {
      if(Number(await fs.readFile(path.join(root,'sessions.txt'),'utf8'))>beforeBoot && (await fs.readFile(path.join(root,'connected.txt'),'utf8'))==='true') {restored=true;break;} await delay(500);
    }
    assert(restored,'BOOT_COMPLETED restored a fresh authenticated session without opening Activity');
    // CCS/1 WHY:a second instrumentation run kills the target process; use the actual system notification action instead.
    device('shell','input','keyevent','224');device('shell','wm','dismiss-keyguard');
    device('shell','cmd','statusbar','expand-notifications');await delay(1000);
    function nodes() {
      device('shell','uiautomator','dump','/data/local/tmp/wn29-notification.xml');
      return device('shell','cat','/data/local/tmp/wn29-notification.xml').match(/<node\b[^>]*>/g)||[];
    }
    function tap(node) {
      const bounds=node.match(/bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"/).slice(1).map(Number);
      device('shell','input','tap',String(Math.round((bounds[0]+bounds[2])/2)),String(Math.round((bounds[1]+bounds[3])/2)));
    }
    let items=nodes(),stop=items.find(node=>/text="Zatrzymaj"/i.test(node));
    if(!stop) {
      const title=items.findIndex(node=>node.includes('text="Test PC"'));assert(title>=0,'paired device visible in notification');
      let first=title;while(first>0 && !items[first].includes('id/expandableNotificationRow'))first--;
      let last=first+1;while(last<items.length && !items[last].includes('id/expandableNotificationRow'))last++;
      const expand=items.slice(first,last).find(node=>/content-desc="(?:Expand|Rozwiń)/.test(node));assert(expand,'notification expand action');tap(expand);await delay(500);
      items=nodes();stop=items.find(node=>/text="Zatrzymaj"/i.test(node));
    }
    await fs.writeFile(path.join(root,'notification.xml'),items.join('\n'));
    assert(stop,'real notification contains Stop');
    const screenshot=spawnSync(adb,['-s',serial,'exec-out','screencap','-p'],{windowsHide:true,timeout:10000});
    assert.equal(screenshot.status,0);await fs.writeFile(path.join(root,'notification.png'),screenshot.stdout);
    tap(stop);
    let closed=false;
    for(let attempt=0;attempt<50;attempt++){if((await fs.readFile(path.join(root,'connected.txt'),'utf8'))==='false'){closed=true;break;}await delay(100);}
    assert(closed,'notification stop closes real session after boot');
    assert(device('shell','dumpsys','activity','services',pkg).includes('(nothing)'),'service stopped after boot');
    const report={qrToConnectedMs:Number(output.match(/qrToConnectedMs=(\d+)/)?.[1]),dozeSeconds:soakSeconds,batteryExempt,connectedDozeChecks:Number(output.match(/connectedDozeChecks=(\d+)/)?.[1]),wakeLock:false,discoveryPausedDuringSoak:discoveryPaused,connectedSamples,actualSession:true,activityDestroyed:true,notificationStop:true,rebootRestores:true,artifacts:root};
    await fs.writeFile(path.join(root,'run.json'),JSON.stringify(report,null,2));
    console.log(JSON.stringify(report));
  } finally {
    if(timer)clearInterval(timer);if(socket)socket.close();
    if(instrument && instrument.exitCode===null)instrument.kill();
    if(peer && peer.exitCode===null)peer.kill();
    await fs.rm(path.join(root,'offer.txt'),{force:true});
    await fs.rm(path.join(root,'matrix.txt'),{force:true});
    const cleanup=emulatorVerified ? [['shell','am','force-stop',pkg],['shell','dumpsys','deviceidle','unforce'],['shell','dumpsys','battery','reset'],['shell','input','keyevent','224']] : [];
    if(emulatorVerified)cleanup.push(['shell','cmd','deviceidle','whitelist',(wasExempt?'+':'-')+pkg]);
    if(redirected)cleanup.push(['emu','redir','del','udp:'+redirectPort]);
    for(const args of cleanup){try{device(...args);}catch(error){console.error('cleanup:',error.message);}}
  }
})().catch(error=>{console.error(error);console.error('artifacts:',root);process.exitCode=1;});
