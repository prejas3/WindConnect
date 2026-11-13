// CCS/1 CHECK:real QR/TLS + external-UID share grants + actual picker -> TEMP Windows engine, byte hashes.
const {spawn, spawnSync}=require('node:child_process');
const fs=require('node:fs/promises'),path=require('node:path'),os=require('node:os'),crypto=require('node:crypto'),dgram=require('node:dgram');
const assert=require('node:assert/strict');
const serial=process.env.ANDROID_SERIAL,pkg='io.github.prejas3.windconnect';
const cancelOnly=process.env.FILES_CANCEL_ONLY==='1';
const lifecycleOnly=process.env.RECEIVE_LIFECYCLE_ONLY==='1';
assert(serial?.startsWith('emulator-'),'explicit disposable ANDROID_SERIAL emulator required');
const root=path.join(os.tmpdir(),'windconnect-wn120-'+crypto.randomUUID());
const adb=path.join(process.env.ANDROID_HOME,'platform-tools/adb.exe');
const delay=ms=>new Promise(resolve=>setTimeout(resolve,ms));
function device(...args){const r=spawnSync(adb,['-s',serial,...args],{windowsHide:true,encoding:'utf8',timeout:20000});assert.equal(r.status,0,r.stderr);return r.stdout;}
function bytes(...args){const r=spawnSync(adb,['-s',serial,'exec-out',...args],{windowsHide:true,timeout:20000,maxBuffer:16*1024*1024});assert.equal(r.status,0,r.stderr?.toString());return r.stdout;}
let peer,instrument,socket,timer,redirected=false,verified=false,oldScale,oldStay,output='',peerLog='',dropRequested=false;
(async()=>{try{
  assert.equal(device('shell','getprop','ro.kernel.qemu').trim(),'1');verified=true;
  await fs.mkdir(root,{recursive:true});
  oldScale=device('shell','settings','get','system','font_scale').trim();
  oldStay=device('shell','settings','get','global','stay_on_while_plugged_in').trim();
  device('shell','settings','put','system','font_scale','1.5');device('shell','svc','power','stayon','true');
  device('shell','input','keyevent','224');device('shell','wm','dismiss-keyguard');
  device('install','-r','apps/android/app/build/outputs/apk/debug/app-debug.apk');
  device('install','-r','-t','apps/android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk');
  device('shell','am','force-stop',pkg);device('shell','pm','clear',pkg);
  device('emu','redir','add','udp:47827:47821');redirected=true;
  const sources=path.join(root,'sources'); await fs.mkdir(path.join(sources,'nested'),{recursive:true});
  for(const name of ['data.json','archive.tar.gz','noextension','installer.apk','nested/photo.jpg','nested/data.json'])await fs.writeFile(path.join(sources,name),crypto.randomBytes(131079));
  const big=await fs.open(path.join(sources,'large.bin'),'w');await big.truncate(1024*1024*1024);await big.close();
  await fs.copyFile(path.resolve('target/debug/examples/files_peer.exe'),path.join(root,'files_peer.exe'));
  peer=spawn(path.join(root,'files_peer.exe'),[root,sources],{windowsHide:true,stdio:['ignore','pipe','pipe']});
  peer.stdout.on('data',d=>peerLog+=d);peer.stderr.on('data',d=>peerLog+=d);
  for(let i=0;i<200;i++){try{await fs.access(path.join(root,'port.txt'));break;}catch{}assert.equal(peer.exitCode,null,peerLog);await delay(50);}
  const javaFile=path.join(root,'EncodeQr.java'),imageFile=path.join(root,'desktop-qr.png');
  await fs.writeFile(javaFile,'import java.nio.file.*; import java.awt.image.BufferedImage; import javax.imageio.ImageIO; class EncodeQr { public static void main(String[] a) throws Exception { var rows=Files.readAllLines(Path.of(a[0])); int w=Integer.parseInt(rows.get(0)),scale=3,size=(w+8)*scale; var bits=rows.get(1); var b=new BufferedImage(size,size,BufferedImage.TYPE_INT_RGB); for(int y=0;y<size;y++) for(int x=0;x<size;x++) { int mx=x/scale-4,my=y/scale-4; boolean dark=mx>=0&&my>=0&&mx<w&&my<w&&bits.charAt(my*w+mx)==\'1\'; b.setRGB(x,y,dark?0:0xffffff); } ImageIO.write(b,"png",Path.of(a[1]).toFile()); } }');
  const encoded=spawnSync(path.join(process.env.JAVA_HOME,'bin/java.exe'),[javaFile,path.join(root,'matrix.txt'),imageFile],{windowsHide:true,encoding:'utf8',timeout:10000});assert.equal(encoded.status,0,encoded.stderr);
  const offer=new URL(await fs.readFile(path.join(root,'offer.txt'),'utf8'));
  const fingerprint=crypto.createHash('sha256').update(Buffer.concat([Buffer.from('302a300506032b6570032100','hex'),Buffer.from(offer.searchParams.get('k'),'base64url')])).digest();
  const packet=Buffer.alloc(39);packet.write('WCD1');packet[4]=2;packet.writeUInt16BE(Number(await fs.readFile(path.join(root,'port.txt'),'utf8')),5);fingerprint.copy(packet,7);
  socket=dgram.createSocket({type:'udp4',reuseAddr:true});
  await new Promise((resolve,reject)=>{socket.once('error',reject);socket.bind({port:47821,address:'0.0.0.0',exclusive:false},resolve);});
  timer=setInterval(()=>socket.send(packet,47827,'127.0.0.1'),150);
  device('push',imageFile,'/data/local/tmp/wn120-qr.png');device('shell','run-as',pkg,'mkdir','-p','cache');
  device('shell',`cat /data/local/tmp/wn120-qr.png | run-as ${pkg} tee cache/desktop-qr.png >/dev/null`);
  instrument=spawn(adb,['-s',serial,'shell','am','instrument','-w','-r','-e','class',pkg+'.PairingTest#qrConnectsRealDesktop','-e','desktopQr','true','-e','receiveFilesProof','true','-e','receiveLifecycleOnly',String(lifecycleOnly),'-e','notifications','denied',pkg+'.test/androidx.test.runner.AndroidJUnitRunner'],{windowsHide:true,stdio:['ignore','pipe','pipe']});
  instrument.stdout.on('data',d=>{output+=d;if(!dropRequested&&output.includes('receiveReady=true')){dropRequested=true;fs.writeFile(path.join(root,'receive-ready'),'').catch(e=>console.error(e));}});instrument.stderr.on('data',d=>output+=d);
  for(let i=0;i<1500&&instrument.exitCode===null;i++)await delay(500);
  await fs.writeFile(path.join(root,'instrumentation.txt'),output);await fs.writeFile(path.join(root,'peer.txt'),peerLog);
  assert(output.includes('OK (1 test)')&&output.includes('receiveFilesProof=true'),output);
  if(lifecycleOnly){assert(output.includes('receiveLifecycleProof=true'));const report={serial,lifecycle:true,toggle:true,activityStopsIdleReceiver:true,persistentForegroundOwner:true,notifications:'denied',artifacts:root};await fs.writeFile(path.join(root,'run.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report));return;}
  const received=JSON.parse(bytes('run-as',pkg,'cat','cache/wn120-received.json').toString());
  let sent=[];for(let i=0;i<1200;i++){try{sent=JSON.parse(await fs.readFile(path.join(root,'received.json'),'utf8'));if(sent.length===7)break;}catch{}await delay(100);}
  assert.equal(sent.length,7,'all sender Complete acknowledgements');
  assert.equal(received.length,7);for(const row of sent){const found=received.find(r=>r.sha256===row.sha256&&r.folder.endsWith('/'+path.dirname(row.name).replace(/\\/g,'/')+'/'));assert(found,JSON.stringify(row));assert.equal(found.size,row.size);assert(found.name.length>0,'actual MediaStore collision-safe display name');}
  await fs.writeFile(path.join(root,'android-received.json'),JSON.stringify(received,null,2));
  const report={serial,files:7,largeBytes:1073741824,hashesVerified:true,actualWindowsSender:true,publicDownloads:true,notifications:'denied',artifacts:root};await fs.writeFile(path.join(root,'run.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report));
}catch(error){throw error;}finally{
  if(timer)clearInterval(timer);if(socket)socket.close();
  if(instrument?.exitCode===null)instrument.kill();
  if(peer?.exitCode===null){await fs.writeFile(path.join(root,'done'),'');for(let i=0;i<50&&peer.exitCode===null;i++)await delay(100);if(peer.exitCode===null)peer.kill();}
  if(verified){for(const args of [['shell','am','force-stop',pkg],['shell','am','force-stop',pkg+'.test'],oldScale==='null'?['shell','settings','delete','system','font_scale']:['shell','settings','put','system','font_scale',oldScale],oldStay==='null'?['shell','settings','delete','global','stay_on_while_plugged_in']:['shell','settings','put','global','stay_on_while_plugged_in',oldStay],...(redirected?[['emu','redir','del','udp:47827']]:[])]){try{device(...args);}catch(e){console.error(e.message);}}}
  await fs.rm(path.join(root,'offer.txt'),{force:true});await fs.rm(path.join(root,'matrix.txt'),{force:true});
}})().catch(error=>{console.error(error);console.error('artifacts:',root);process.exitCode=1;});
