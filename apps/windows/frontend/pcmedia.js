// CCS/1 FLOW:WN-47 main-window setting "Udostępnij odtwarzanie telefonowi" -> share_media -> pairing-state.share_media.
// The PC's player (Windows' media controls) on the phone, its controls from the phone; default on.
const invoke = window.__TAURI__?.core.invoke;
const toggle = document.querySelector('#share-media');

function render(state) {
  if (!state || typeof state.share_media !== 'boolean') return;
  document.querySelector('#media-settings').hidden = false;
  toggle.setAttribute('aria-checked', String(state.share_media));
  toggle.disabled = false;
}

toggle?.addEventListener('click', async () => {
  if (!invoke) return;
  const next = toggle.getAttribute('aria-checked') !== 'true';
  toggle.disabled = true;
  toggle.setAttribute('aria-checked', String(next));
  try { await invoke('share_media', { enabled: next }); }
  catch (error) { console.error(error); toggle.setAttribute('aria-checked', String(!next)); }
  finally { toggle.disabled = false; }
});

async function start() {
  if (!invoke || !toggle) return;
  await window.__TAURI__.event.listen('pairing-state', event => render(event.payload));
  render(await invoke('pairing_snapshot'));
}
start().catch(console.error);
