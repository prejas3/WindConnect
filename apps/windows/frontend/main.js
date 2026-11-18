import { t, formatTime } from './i18n.js';
const invoke = window.__TAURI__?.core.invoke;
import './touchpad.js';
const systemTheme = matchMedia('(prefers-color-scheme: dark)');
let themeChoice = 'system';
try { themeChoice = localStorage.getItem('windconnect-theme') || 'system'; } catch { /* Storage may be disabled by host policy. */ }
if (!['light', 'dark', 'system'].includes(themeChoice)) themeChoice = 'system';

function applyTheme() {
  document.documentElement.dataset.theme = themeChoice === 'system' ? (systemTheme.matches ? 'dark' : 'light') : themeChoice;
  document.querySelectorAll('[data-theme-choice]').forEach(button => button.setAttribute('aria-pressed', String(button.dataset.themeChoice === themeChoice)));
  if (invoke) invoke('apply_window_theme', { theme: document.documentElement.dataset.theme })
    .then(enabled => { document.body.dataset.mica = String(enabled); })
    .catch(error => { document.body.dataset.mica = 'false'; console.error(error); });
}
applyTheme();
systemTheme.addEventListener('change', applyTheme);
document.querySelectorAll('[data-theme-choice]').forEach(button => button.addEventListener('click', () => {
  themeChoice = button.dataset.themeChoice;
  try { localStorage.setItem('windconnect-theme', themeChoice); } catch { /* In-memory preference still works. */ }
  applyTheme();
}));

let toastTimer;
export function toast(message) {
  const element = document.querySelector('#toast');
  clearTimeout(toastTimer);
  element.textContent = message;
  element.hidden = false;
  toastTimer = setTimeout(() => { element.hidden = true; }, 3500);
}

async function windowAction(action) {
  if (!invoke) return;
  try { await invoke('window_action', { action }); }
  catch (error) { console.error(error); toast(t('main.window_failed')); }
}
document.querySelectorAll('[data-window]').forEach(button => button.addEventListener('click', () => windowAction(button.dataset.window)));
document.querySelector('#titlebar').addEventListener('mousedown', event => {
  if (event.button === 0 && !event.target.closest('button')) windowAction(event.detail === 2 ? 'maximize' : 'drag');
});

async function start() {
  const status = document.querySelector('#status');
  try {
    // CCS/1 RULE:release native command always denies the gallery, including URL overrides.
    const localPreview = !invoke && ['localhost', '127.0.0.1'].includes(location.hostname);
    const devMode = invoke ? await invoke('ui_dev_mode') : localPreview;
    if (invoke) {
      const version = await invoke('core_version');
      status.textContent = t('main.core_ready', { version });
    } else {
      status.textContent = t('main.preview');
    }
    if (devMode) {
      const { mountGallery } = await import('./gallery.js');
      mountGallery(toast);
      const toggle = document.querySelector('#gallery-toggle');
      const gallery = document.querySelector('#gallery');
      const home = document.querySelector('#home');
      toggle.hidden = false;
      toggle.addEventListener('click', () => {
        gallery.hidden = !gallery.hidden;
        home.hidden = !gallery.hidden;
        toggle.setAttribute('aria-pressed', String(!gallery.hidden));
      });
      if (new URLSearchParams(location.search).has('gallery')) toggle.click();
    }
  } catch (error) {
    status.textContent = t('main.core_failed');
    console.error(error);
  }
}
start();

// CCS/1 FLOW:actions.json (Rust) -> snapshot -> switches; a switch writes back and redraws from the result.
const actionsSection = document.querySelector('#actions');
const statusText = { done: t('actions.status.done'), refused: t('actions.status.refused'), 'bad-argument': t('actions.status.bad_argument'), failed: t('actions.status.failed'), busy: t('actions.status.busy') };
function actionRow(item) {
  const row = document.createElement('div');
  row.className = 'control-row';
  const text = document.createElement('div');
  const label = document.createElement('span');
  label.id = `action-${item.id}`;
  label.textContent = item.label;
  const detail = document.createElement('small');
  detail.textContent = item.problem || item.detail;
  if (item.problem) detail.className = 'problem';
  text.append(label, detail);
  const toggle = document.createElement('button');
  toggle.className = 'switch';
  toggle.setAttribute('role', 'switch');
  toggle.setAttribute('aria-checked', String(item.enabled && !item.problem));
  toggle.setAttribute('aria-labelledby', label.id);
  toggle.disabled = Boolean(item.problem) || Boolean(actionsSection.dataset.broken);
  toggle.addEventListener('click', async () => {
    const enabled = toggle.getAttribute('aria-checked') !== 'true';
    toggle.disabled = true;
    try { renderActions(await invoke('set_action_enabled', { id: item.id, enabled })); }
    catch (error) { console.error(error); toast(typeof error === 'string' ? error : t('main.save_failed')); toggle.disabled = false; }
  });
  row.append(text, toggle);
  return row;
}
function renderLast(last) {
  const element = document.querySelector('#actions-last');
  if (!last) { element.textContent = t('actions.none_yet'); return; }
  const time = formatTime(last.atMs);
  element.textContent = t('actions.last', { peer: last.peer || t('common.phone'), label: last.label, status: statusText[last.status] || last.status, time });
}
function renderActions(snapshot) {
  const error = document.querySelector('#actions-error');
  actionsSection.dataset.broken = snapshot.error ? 'true' : '';
  error.hidden = !snapshot.error;
  error.textContent = snapshot.error ? t('actions.file_broken') : '';
  document.querySelector('#actions-builtins').replaceChildren(...snapshot.builtins.map(actionRow));
  document.querySelector('#actions-custom').replaceChildren(...snapshot.custom.map(actionRow));
  document.querySelector('#actions-empty').hidden = snapshot.custom.length > 0;
  renderLast(snapshot.last);
  actionsSection.hidden = false;
}
async function refreshActions() {
  if (!invoke) return;
  try { renderActions(await invoke('actions_snapshot')); }
  catch (error) { console.error(error); }
}
refreshActions();
window.addEventListener('focus', refreshActions);
window.__TAURI__?.event?.listen('actions-handled', event => renderLast(event.payload));
// Files dropped on this window go to the phone (WN-39); the result shows as a toast.
window.__TAURI__?.event?.listen('files-notice', event => toast(event.payload.message));
window.__TAURI__?.event?.listen('files-drag', event => document.body.classList.toggle('dropping', event.payload));
document.querySelector('#actions-edit').addEventListener('click', () => invoke?.('edit_custom_actions').catch(error => { console.error(error); toast(t('actions.open_failed')); }));
document.querySelector('#open-pairing').addEventListener('click', () => invoke?.('panel_action', { action: 'show' }).catch(error => { console.error(error); toast(t('main.panel_failed')); }));
document.querySelector('#tray-options').addEventListener('click', () => invoke?.('tray_action', { action: 'menu' }).catch(error => { console.error(error); toast(t('main.menu_failed')); }));
document.querySelector('#quit-app').addEventListener('click', () => invoke?.('tray_action', { action: 'quit' }).catch(error => { console.error(error); toast(t('main.quit_failed')); }));
import './clipboard.js';
import './onboarding.js';
import './pcmedia.js';
import './relay.js';
import './notify.js';
import './callvolume.js';
