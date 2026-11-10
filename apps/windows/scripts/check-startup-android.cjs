// CCS/1 PURPOSE:finite cold/warm release startup evidence; no data clearing, no secret-bearing logcat output.
// RULE:install a signed non-debuggable APK first; ANDROID_SERIAL is explicit; compilation mode is recorded, never hidden.
const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const assert = require('node:assert/strict');
const serial = process.env.ANDROID_SERIAL;
assert(serial, 'set ANDROID_SERIAL to the intended test device');
const adb = path.join(process.env.ANDROID_HOME, 'platform-tools/adb.exe');
const pkg = 'io.github.prejas3.windconnect';
const root = fs.mkdtempSync(path.join(os.tmpdir(), 'windconnect-startup-'));
function device(...args) {
  const result = spawnSync(adb, ['-s', serial, ...args], { windowsHide: true, encoding: 'utf8', timeout: 30000 });
  assert.equal(result.status, 0, result.stderr); return result.stdout;
}
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
function summary(values) {
  const sorted = [...values].sort((a,b) => a-b);
  return { min: sorted[0], median: (sorted[4]+sorted[5])/2, p95: sorted[9], max: sorted[9] };
}
(async () => {
  try {
    const details = device('shell','dumpsys','package',pkg);
    assert(!/flags=\[[^\]]*DEBUGGABLE/.test(details), 'use a release APK, not debug');
    const compilation = process.env.STARTUP_COMPILATION || 'verify';
    assert(['verify','speed'].includes(compilation), 'STARTUP_COMPILATION=verify or speed');
    device('shell','cmd','package','compile','--reset',pkg);
    device('shell','cmd','package','compile','-m',compilation,'-f',pkg);
    const samples = [];
    for (let index=0;index<10;index++) {
      device('logcat','-c');
      const output = device('shell','am','start','-W','-S','-n',pkg+'/.MainActivity');
      await delay(300);
      const drawn = device('logcat','-d','-s','ActivityTaskManager:I').split('\n')
        .filter(line => line.includes(pkg+'/.MainActivity') && /Displayed |Fully drawn /.test(line));
      const ttfd = drawn.find(line => line.includes('Fully drawn '))?.match(/\+(?:(\d+)s)?(\d+)ms/);
      const ttid = Number(output.match(/TotalTime:\s*(\d+)/)?.[1]);
      assert(ttid > 0 && ttfd, 'both TTID and TTFD must be reported');
      device('shell','input','keyevent','KEYCODE_HOME');
      await delay(100);
      const warm = device('shell','am','start','-W','-n',pkg+'/.MainActivity');
      const warmWaitMs = Number(warm.match(/WaitTime:\s*(\d+)/)?.[1]);
      assert(Number.isFinite(warmWaitMs), 'warm return command must report WaitTime');
      samples.push({ index:index+1, coldTtidMs:ttid, coldTtfdMs:Number(ttfd[1]||0)*1000+Number(ttfd[2]), warmWaitMs, coldOutput:output, warmOutput:warm, drawn });
    }
    const report = { serial, model:device('shell','getprop','ro.product.model').trim(),
      api:device('shell','getprop','ro.build.version.sdk').trim(),
      emulator:device('shell','getprop','ro.kernel.qemu').trim()==='1',
      version:details.match(/versionName=(\S+)/)?.[1], compilation, samples,
      coldTtidMs:summary(samples.map(s=>s.coldTtidMs)), coldTtfdMs:summary(samples.map(s=>s.coldTtfdMs)),
      warmWaitMs:summary(samples.map(s=>s.warmWaitMs)), artifacts:root };
    fs.writeFileSync(path.join(root,'run.json'),JSON.stringify(report,null,2));
    console.log(JSON.stringify({...report,samples:undefined},null,2));
  } finally { device('shell','am','force-stop',pkg); }
})().catch(error=>{console.error(error);process.exitCode=1;});
