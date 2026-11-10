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
function renderPairing(state) {
  const waiting = state.phase === 'waiting';
  const connected = state.phase === 'connected';
  document.querySelector('#phone-name').textContent = state.phone || 'Twój telefon';
  document.querySelector('#connection-state').textContent = ({ connected: 'Połączono · Wi-Fi', waiting: 'Oczekiwanie na telefon', connecting: 'Sparowano · Łączenie…', offline: 'Telefon rozłączony', loading: 'Uruchamianie…' })[state.phase] || 'Brak połączenia';
  document.querySelector('#pairing-title').textContent = connected ? (state.phone || 'Telefon połączony') : waiting ? 'Połącz telefon' : state.phase === 'connecting' ? 'Kończenie połączenia…' : 'Parowanie telefonu';
  document.querySelector('#pairing-message').textContent = state.message;
  document.querySelector('#quick-actions').hidden = !connected;
  document.querySelector('#pairing-instructions').hidden = !waiting;
  document.querySelector('#pairing-hint').hidden = !connected;
  document.querySelector('#pairing-refresh').hidden = connected || ['connecting','loading','failed'].includes(state.phase);
  document.querySelector('#pairing-cancel').hidden = !waiting;
  document.querySelector('#pairing-warning').hidden = !state.skipped_records;
  document.querySelector('#pairing-warning').textContent = state.skipped_records ? `Pominięto uszkodzone wpisy sparowania: ${state.skipped_records}.` : '';
  const canvas = document.querySelector('#pairing-qr');
  canvas.hidden = !waiting || !state.qr;
  if (!canvas.hidden) {
    // CCS/1 RULE:integer modules and a four-module quiet zone; black/white in every theme, no raw link in DOM/storage.
    const { width, dark } = state.qr;
    const scale = Math.max(1, Math.floor(236 / (width + 8)));
    canvas.width = canvas.height = (width + 8) * scale;
    const context = canvas.getContext('2d');
    context.fillStyle = '#fff'; context.fillRect(0, 0, canvas.width, canvas.height);
    context.fillStyle = '#000';
    dark.forEach((value, index) => { if (value) context.fillRect((index % width + 4) * scale, (Math.floor(index / width) + 4) * scale, scale, scale); });
  } else { canvas.width = canvas.height = 0; }
}
document.querySelectorAll('#pairing-refresh,#pairing-cancel').forEach(button => button.addEventListener('click', () => invoke?.('pairing_action', { action: button.id === 'pairing-refresh' ? 'refresh' : 'close' }).catch(error)));
// CCS/1 PERF:preload font/layout before ready; two frames acknowledge content submission, not physical display scanout.
async function refreshShortcut() {
  const state = await invoke('panel_snapshot');
  document.querySelector('#shortcut-error').hidden = !state.shortcut_error;
  document.querySelector('#shortcut').textContent = state.shortcut_error ? 'Skrót niedostępny' : 'Win + Shift + W';
}
async function start() {
  if (!invoke) return;
  await window.__TAURI__.event.listen('pairing-state', event => renderPairing(event.payload));
  renderPairing(await invoke('pairing_snapshot'));
  await window.__TAURI__.event.listen('panel-opened', async event => {
    theme();
    await refreshShortcut();
    renderPairing(await invoke('pairing_snapshot'));
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
