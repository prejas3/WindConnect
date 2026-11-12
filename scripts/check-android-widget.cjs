// CCS/1 CHECK:finite native widget/idle fixture on an explicitly selected disposable emulator; never selects a phone.
const {spawnSync}=require('node:child_process');
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const serial=process.env.ANDROID_SERIAL;
assert(serial?.startsWith('emulator-'),'set ANDROID_SERIAL to your exclusive disposable emulator');
const out=process.env.WIDGET_ARTIFACTS || path.join(process.env.TEMP,'windconnect-wn33');
const pkg='io.github.prejas3.windconnect';
fs.mkdirSync(out,{recursive:true});
function run(...args){const r=spawnSync('adb',['-s',serial,...args],{windowsHide:true,encoding:'utf8',timeout:300000});assert.equal(r.status,0,r.stderr||r.error?.message);return r.stdout;}
assert.equal(run('shell','getprop','ro.kernel.qemu').trim(),'1');
try{
  run('install','-r','apps/android/app/build/outputs/apk/debug/app-debug.apk');
  run('install','-r','-t','apps/android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk');
  const log=run('shell','am','instrument','-w','-r','-e','widgetFixture','true','-e','class',pkg+'.WidgetHostTest',pkg+'.test/androidx.test.runner.AndroidJUnitRunner');
  fs.writeFileSync(path.join(out,'widget-host-instrumentation.txt'),log);
  assert(log.includes('OK (1 test)'),log);
  for(const mode of ['Light','Dark']){
    const r=spawnSync('adb',['-s',serial,'exec-out','run-as',pkg,'cat',`cache/wn33-${mode}-150.png`],{windowsHide:true,timeout:10000});
    assert.equal(r.status,0);fs.writeFileSync(path.join(out,`widget-${mode}-150.png`),r.stdout);
  }
  const metrics={serial};
  for(const key of ['idleHostUpdates','idlePublishes','idleOnUpdateCalls','onUpdateCallsTotal','idleDurationSeconds']){
    const value=log.match(new RegExp(`${key}=(\\d+)`));assert(value,`missing ${key}`);metrics[key]=Number(value[1]);
  }
  assert.equal(metrics.idleHostUpdates,0);assert.equal(metrics.idlePublishes,0);assert(metrics.idleDurationSeconds>=120);
  fs.writeFileSync(path.join(out,'widget-metrics.json'),JSON.stringify(metrics,null,2));
  console.log(JSON.stringify(metrics));
}finally{run('shell','am','force-stop',pkg);}
