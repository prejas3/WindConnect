// CCS/1 FLOW:WN-43 main-window setting "Powiadomienia z telefonu" -> phone_notifications -> pairing-state.phone_notifications.
// The phone's notifications as Windows toasts (notify/1); default on, the phone asks for its own consent (WN-42).
const invoke = window.__TAURI__?.core.invoke;
const toggle = document.querySelector('#phone-notifications');

function render(state) {
  if (!state || typeof state.phone_notifications !== 'boolean') return;
  document.querySelector('#notify-settings').hidden = false;
  toggle.setAttribute('aria-checked', String(state.phone_notifications));
  toggle.disabled = false;
}

toggle?.addEventListener('click', async () => {
  if (!invoke) return;
  const next = toggle.getAttribute('aria-checked') !== 'true';
  toggle.disabled = true;
  toggle.setAttribute('aria-checked', String(next));
  try { await invoke('phone_notifications', { enabled: next }); }
  catch (error) { console.error(error); toggle.setAttribute('aria-checked', String(!next)); }
  finally { toggle.disabled = false; }
});

async function start() {
  if (!invoke || !toggle) return;
  await window.__TAURI__.event.listen('pairing-state', event => render(event.payload));
  render(await invoke('pairing_snapshot'));
}
start().catch(console.error);
