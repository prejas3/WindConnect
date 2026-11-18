import { t } from './i18n.js';
const invoke = window.__TAURI__?.core.invoke;
const buttons = [...document.querySelectorAll('[role^="menuitem"]')];
const system = matchMedia('(prefers-color-scheme: dark)');
const labels = { connected: t('state.connected'), searching: t('state.searching'), unavailable: t('state.unavailable') };
function theme() { let choice = 'system'; try { choice = localStorage.getItem('windconnect-theme') || choice; } catch {} document.documentElement.dataset.theme = choice === 'system' ? (system.matches ? 'dark' : 'light') : choice; }
function error(value) { const element = document.querySelector('#menu-error'); element.hidden = false; element.textContent = t('error.action'); console.error(value); }
async function refresh() {
  theme(); if (!invoke) return;
  try { const state = await invoke('tray_snapshot'); document.querySelector('#tray-state').textContent = labels[state.phase] || labels.unavailable; document.querySelector('#autostart').setAttribute('aria-checked', String(await invoke('autostart_enabled'))); document.querySelector('#autostart').disabled = false; document.querySelector('#menu-error').hidden = true; } catch (value) { document.querySelector('#autostart').disabled = true; error(value); }
}
document.querySelectorAll('[data-action]').forEach(button => button.addEventListener('click', () => invoke?.('tray_action', { action: button.dataset.action }).catch(error)));
document.querySelector('#autostart').addEventListener('click', async event => { const button = event.currentTarget; if (!invoke) return; button.disabled = true; try { const enabled = await invoke('set_autostart', { enabled: button.getAttribute('aria-checked') !== 'true' }); button.setAttribute('aria-checked', String(enabled)); } catch (value) { error(value); } finally { button.disabled = false; } });
document.addEventListener('keydown', event => {
  const index = buttons.indexOf(document.activeElement);
  const targets = { ArrowDown: (index + 1) % buttons.length, ArrowUp: (index - 1 + buttons.length) % buttons.length, Home: 0, End: buttons.length - 1 };
  if (event.key in targets) { event.preventDefault(); buttons[targets[event.key]].focus(); }
  else if (event.key === 'Escape') invoke?.('tray_action', { action: 'hide' }).catch(error);
  else if (event.key === 'Tab') { event.preventDefault(); buttons[(index + (event.shiftKey ? -1 : 1) + buttons.length) % buttons.length].focus(); }
});
window.addEventListener('focus', () => { refresh(); buttons[0].focus(); });
window.addEventListener('storage', theme); system.addEventListener('change', theme);
document.addEventListener('contextmenu', event => event.preventDefault());
refresh();
window.__TAURI__?.event?.listen('connection-status', event => { document.querySelector('#tray-state').textContent = labels[event.payload.phase] || labels.unavailable; }).catch(error);
// CCS/1 RULE:hidden WebViews may miss DOM focus; refresh native state on every explicit open.
window.__TAURI__?.event?.listen('tray-menu-opened', refresh).catch(error);
