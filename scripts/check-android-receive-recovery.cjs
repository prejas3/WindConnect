// CCS/1 CHECK:force-stop actual app processes at every durable MediaStore transaction boundary; disposable emulator only.
const {spawn,spawnSync}=require('node:child_process');
const fs=require('node:fs/promises'),path=require('node:path'),os=require('node:os'),crypto=require('node:crypto');
const assert=require('node:assert/strict');
const serial=process.env.ANDROID_SERIAL,pkg='io.github.prejas3.windconnect';
assert(serial?.startsWith('emulator-'),'explicit disposable emulator required');
const adb=path.join(process.env.ANDROID_HOME,'platform-tools/adb.exe');
const delay=ms=>new Promise(resolve=>setTimeout(resolve,ms));
function device(...args){const r=spawnSync(adb,['-s',serial,...args],{windowsHide:true,encoding:'utf8',timeout:90000});assert.equal(r.status,0,r.stderr);return r.stdout;}
const root=path.join(os.tmpdir(),'windconnect-wn120-recovery-'+crypto.randomUUID());
let child;
(async()=>{await fs.mkdir(root,{recursive:true});try{
  assert.equal(device('shell','getprop','ro.kernel.qemu').trim(),'1');
  device('install','-r','apps/android/app/build/outputs/apk/debug/app-debug.apk');
  device('install','-r','-t','apps/android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk');
  const phases=['inserted','pending','copying','ready','visible','published'];
  for(const phase of phases){
    let output='';
    child=spawn(adb,['-s',serial,'shell','am','instrument','-w','-r','-e','class',pkg+'.IncomingFilesTest#killAtTransactionPhase','-e','killPhase',phase,pkg+'.test/androidx.test.runner.AndroidJUnitRunner'],{windowsHide:true,stdio:['ignore','pipe','pipe']});
    child.stdout.on('data',d=>output+=d);child.stderr.on('data',d=>output+=d);
    for(let i=0;i<300&&!output.includes('killReady='+phase)&&child.exitCode===null;i++)await delay(100);
    assert(output.includes('killReady='+phase),output);
    device('shell','am','force-stop',pkg);
    for(let i=0;i<50&&child.exitCode===null;i++)await delay(100);
    if(child.exitCode===null)child.kill();
    await fs.writeFile(path.join(root,phase+'-killed.txt'),output);
    const recovered=device('shell','am','instrument','-w','-r','-e','class',pkg+'.IncomingFilesTest#recoverAfterProcessKill','-e','recoverKilled','true',pkg+'.test/androidx.test.runner.AndroidJUnitRunner');
    await fs.writeFile(path.join(root,phase+'-recovered.txt'),recovered);
    assert(recovered.includes('OK (1 test)'),recovered);
  }
  const report={serial,actualForceStop:true,phases,bytesVerified:true,noPending:true,noDuplicates:true,artifacts:root};
  await fs.writeFile(path.join(root,'run.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report));
}finally{if(child?.exitCode===null)child.kill();device('shell','am','force-stop',pkg);}})().catch(error=>{console.error(error);console.error('artifacts:',root);process.exitCode=1;});
