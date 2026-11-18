import { t } from './i18n.js';
// CCS/1 M:touchpad controls | IO:pairing snapshot -> status + local one-click withdrawal.
const invoke = window.__TAURI__?.core.invoke;
const box = document.createElement('section');
const inPanel = !!document.querySelector('#find-status');
box.className = inPanel ? 'transfers touchpad-card' : 'surface touchpad-card';
// In the panel the card belongs to a connected phone (WN-65); the main window keeps the setting visible.
box.hidden = inPanel;
box.setAttribute('aria-label', t('touchpad.title'));
const status = document.createElement('p'); status.setAttribute('role', 'status');
status.id = 'touchpad-status';
const button = document.createElement('button'); button.className = inPanel ? 'button quiet small' : 'button';
let allowed = true;
button.addEventListener('click', async () => {
  button.disabled = true;
  try { await invoke?.('pairing_action', {action: allowed ? 'touchpad-off' : 'touchpad-on'}); }
  catch { status.textContent = t('touchpad.save_failed'); }
  finally { button.disabled = false; }
});
document.querySelector('#touchpad-toggle')?.addEventListener('click', () => button.click());
box.append(status, button);
// WN-65: under the panel's quick actions (the header comes first, as in the mockup); first in other windows.
const anchor = document.querySelector('#find-status') || document.querySelector('.page-heading');
if (anchor) anchor.after(box); else (document.querySelector('main') || document.body).prepend(box);
function render(state) {
  allowed = state.touchpad?.allowed ?? true;
  if (inPanel) box.hidden = !state.connected;
  status.textContent = state.touchpad?.message || (allowed ? t('touchpad.allowed') : t('touchpad.off'));
  button.textContent = allowed ? t('touchpad.turn_off') : t('touchpad.turn_on');
  document.querySelector('#touchpad-toggle')?.setAttribute('aria-pressed', String(allowed));
}
if (invoke) {
  await window.__TAURI__.event.listen('pairing-state', e => render(e.payload));
  render(await invoke('pairing_snapshot'));
}
