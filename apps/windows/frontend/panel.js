const invoke = window.__TAURI__?.core.invoke;
const system = matchMedia('(prefers-color-scheme: dark)');
function theme() {
  let choice = 'system';
  try { choice = localStorage.getItem('windconnect-theme') || choice; } catch {}
  document.documentElement.dataset.theme = ['light', 'dark'].includes(choice) ? choice : (system.matches ? 'dark' : 'light');
}
function error(value) { document.querySelector('#panel-error').hidden = false; console.error(value); }
async function action(action) { try { await invoke?.('panel_action', { action }); } catch (value) { error(value); } }
document.querySelector('#dismiss').addEventListener('click', () => action('hide'));
document.querySelector('#settings').addEventListener('click', () => action('settings'));
document.addEventListener('keydown', event => { if (event.key === 'Escape') { event.preventDefault(); action('hide'); } });
document.addEventListener('contextmenu', event => event.preventDefault());
window.addEventListener('storage', theme); system.addEventListener('change', theme);
theme();
// CCS/1 PERF:preload font/layout before ready; two frames acknowledge content submission, not physical display scanout.
async function refreshShortcut() {
  const state = await invoke('panel_snapshot');
  document.querySelector('#shortcut-error').hidden = !state.shortcut_error;
  document.querySelector('#shortcut').textContent = state.shortcut_error ? 'Skrót niedostępny' : 'Win + Shift + W';
}
async function start() {
  if (!invoke) return;
  await window.__TAURI__.event.listen('panel-opened', async event => {
    theme();
    await refreshShortcut();
    const panel = document.querySelector('.panel');
    panel.classList.remove('opening'); void panel.offsetHeight; panel.classList.add('opening');
    document.querySelector('#dismiss').focus({ preventScroll: true });
    requestAnimationFrame(() => requestAnimationFrame(() => invoke('panel_painted', { sequence: event.payload }).catch(error)));
  });
  await document.fonts.ready;
  document.body.getBoundingClientRect();
  await invoke('panel_ready');
}
start().catch(error);
