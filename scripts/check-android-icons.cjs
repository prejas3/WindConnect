// CCS/1 CHECK:finite actual APK resources and persistent notification on an explicitly selected disposable emulator; never selects a phone.
const {spawnSync}=require('node:child_process');
const fs=require('node:fs');
const path=require('node:path');
const assert=require('node:assert/strict');
const serial=process.env.ANDROID_SERIAL;
assert(serial?.startsWith('emulator-'),'set ANDROID_SERIAL to your exclusive disposable emulator');
const adb=path.join(process.env.ANDROID_HOME,'platform-tools/adb.exe');
const out=process.env.ICON_ARTIFACTS || path.join(process.env.TEMP,'windconnect-wn93-android');
const pkg='io.github.prejas3.windconnect';
function run(...args){const r=spawnSync(adb,['-s',serial,...args],{windowsHide:true,encoding:'utf8',timeout:180000});assert.equal(r.status,0,r.stderr||r.error?.message);return r.stdout;}
assert.equal(run('shell','getprop','ro.kernel.qemu').trim(),'1');
run('shell','input','keyevent','224');
run('shell','wm','dismiss-keyguard');
run('shell','cmd','statusbar','collapse');
fs.mkdirSync(out,{recursive:true});
try{
  run('install','-r','apps/android/app/build/outputs/apk/debug/app-debug.apk');
  run('install','-r','-t','apps/android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk');
  const log=run('shell','am','instrument','-w','-r','-e','iconFixture','true','-e','class',pkg+'.IconAssetsTest',pkg+'.test/androidx.test.runner.AndroidJUnitRunner');
  fs.writeFileSync(path.join(out,'instrumentation.log'),log);
  assert(log.includes('OK (2 tests)'),log);
  for(const name of ['wn93-icons.png','wn93-notification.png']){const r=spawnSync(adb,['-s',serial,'exec-out','run-as',pkg,'cat','cache/'+name],{windowsHide:true,timeout:10000});assert.equal(r.status,0);fs.writeFileSync(path.join(out,name),r.stdout);}
  console.log(JSON.stringify({serial,packagedDrawables:true,safeZone:true,monochrome:true,notification:true,artifacts:out}));
}finally{run('shell','am','force-stop',pkg);}
