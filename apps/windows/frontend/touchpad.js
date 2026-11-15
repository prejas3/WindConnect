// CCS/1 M:touchpad controls | IO:pairing snapshot -> status + local one-click withdrawal.
const invoke = window.__TAURI__?.core.invoke;
const box = document.createElement('section');
box.className = 'card';
box.setAttribute('aria-label', 'Touchpad z telefonu');
const status = document.createElement('p'); status.setAttribute('role', 'status');
status.id = 'touchpad-status';
const button = document.createElement('button'); button.className = 'button secondary';
let allowed = true;
button.addEventListener('click', async () => {
  button.disabled = true;
  try { await invoke?.('pairing_action', {action: allowed ? 'touchpad-off' : 'touchpad-on'}); }
  catch { status.textContent = 'Nie udało się zmienić ustawienia touchpada.'; }
  finally { button.disabled = false; }
});
document.querySelector('#touchpad-toggle')?.addEventListener('click', () => button.click());
box.append(status, button);
(document.querySelector('main') || document.body).prepend(box);
function render(state) {
  allowed = state.touchpad?.allowed ?? true;
  status.textContent = state.touchpad?.message || (allowed ? 'Touchpad z telefonu jest dozwolony' : 'Touchpad z telefonu jest wyłączony');
  button.textContent = allowed ? 'Wyłącz touchpad' : 'Włącz touchpad';
  document.querySelector('#touchpad-toggle')?.setAttribute('aria-pressed', String(allowed));
}
if (invoke) {
  await window.__TAURI__.event.listen('pairing-state', e => render(e.payload));
  render(await invoke('pairing_snapshot'));
}
