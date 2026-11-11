// CCS/1 SEC:only settings/status cross IPC; clipboard payload stays in the native adapter.
const invoke = window.__TAURI__?.core.invoke;
function render(state) {
  document.querySelectorAll('[data-clipboard-controls]').forEach(element => { element.hidden = false; });
  document.querySelectorAll('[data-clipboard-toggle]').forEach(element => { element.setAttribute('aria-checked', String(state.enabled)); element.disabled = false; });
  document.querySelectorAll('[data-clipboard-status]').forEach(element => { element.textContent = state.status; });
  document.querySelectorAll('[data-clipboard-shortcut]').forEach(element => { element.textContent = state.shortcut_error ? 'Ctrl + Alt + Shift + C jest zajęty. Użyj przycisku Schowek.' : 'Wyślij: Ctrl + Alt + Shift + C'; });
}
async function refresh() { if (invoke) render(await invoke('clipboard_snapshot')); }
function failed() { document.querySelectorAll('[data-clipboard-status]').forEach(element => { element.textContent = 'Nie udało się wykonać operacji schowka.'; }); }
document.querySelectorAll('[data-clipboard-send]').forEach(button => button.addEventListener('click', () => invoke?.('clipboard_action', { action: 'send' }).catch(failed)));
document.querySelectorAll('[data-clipboard-toggle]').forEach(button => button.addEventListener('click', async () => {
  if (!invoke) return;
  button.disabled = true;
  try { await invoke('clipboard_action', { action: 'enabled', enabled: button.getAttribute('aria-checked') !== 'true' }); await refresh(); }
  catch { button.disabled = false; failed(); }
}));
if (invoke) {
  window.__TAURI__?.event?.listen('clipboard-state', event => render(event.payload));
  window.addEventListener('focus', () => refresh().catch(failed));
  refresh().catch(failed);
}
