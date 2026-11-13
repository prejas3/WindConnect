// CCS/1 FLOW:WN-48 main-window settings "Wstrzymaj odtwarzanie przy połączeniu" / "Głośność z telefonu" and WN-57 "Połączenia
// z telefonu" -> call_volume_settings -> pairing-state.pause_on_call / volume_remote / call_toasts. All on by default.
const invoke = window.__TAURI__?.core.invoke;
const switches = { pause_on_call: document.querySelector('#pause-on-call'), volume: document.querySelector('#volume-remote'), calls: document.querySelector('#call-toasts') };
const field = { pause_on_call: 'pause_on_call', volume: 'volume_remote', calls: 'call_toasts' };
const arg = { pause_on_call: 'pauseOnCall', volume: 'volume', calls: 'calls' };

function render(state) {
  if (!state || typeof state.pause_on_call !== 'boolean') return;
  for (const [key, toggle] of Object.entries(switches)) {
    if (!toggle) continue;
    toggle.setAttribute('aria-checked', String(state[field[key]]));
    toggle.disabled = false;
  }
}

for (const [key, toggle] of Object.entries(switches)) {
  toggle?.addEventListener('click', async () => {
    if (!invoke) return;
    const next = toggle.getAttribute('aria-checked') !== 'true';
    toggle.setAttribute('aria-checked', String(next));
    toggle.disabled = true;
    try { await invoke('call_volume_settings', { [arg[key]]: next }); }
    catch (error) { console.error(error); toggle.setAttribute('aria-checked', String(!next)); }
    finally { toggle.disabled = false; }
  });
}

async function start() {
  if (!invoke) return;
  await window.__TAURI__.event.listen('pairing-state', event => render(event.payload));
  render(await invoke('pairing_snapshot'));
}
start().catch(console.error);
