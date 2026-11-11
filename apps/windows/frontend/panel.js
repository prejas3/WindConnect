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
  const sas = state.phase.startsWith('sas-');
  const asking = state.phase === 'sas-code';
  document.querySelector('#phone-name').textContent = state.phone || 'Twój telefon';
  document.querySelector('#connection-state').textContent = ({ connected: 'Połączono · Wi-Fi', waiting: 'Oczekiwanie na telefon', 'sas-waiting': 'Oczekiwanie na telefon', 'sas-code': 'Porównaj kody', 'sas-confirming': 'Porównaj kody', connecting: 'Sparowano · Łączenie…', offline: 'Telefon rozłączony', loading: 'Uruchamianie…' })[state.phase] || 'Brak połączenia';
  document.querySelector('#pairing-title').textContent = connected ? (state.phone || 'Telefon połączony') : waiting ? 'Połącz telefon' : sas ? 'Paruj bez aparatu' : state.phase === 'connecting' ? 'Kończenie połączenia…' : 'Parowanie telefonu';
  // CCS/1 SEC:the code is shown only while a decision is pending; accept only after the person compares both screens (§12.6).
  const code = document.querySelector('#pairing-code');
  code.hidden = !asking || !state.code;
  code.textContent = asking && state.code ? state.code : '';
  document.querySelector('#pairing-accept').hidden = !asking;
  document.querySelector('#pairing-reject').hidden = !asking;
  document.querySelector('#pairing-sas').hidden = !(waiting || state.phase === 'expired' || state.phase === 'no-network');
  document.querySelector('#pairing-message').textContent = state.message;
  document.querySelector('#quick-actions').hidden = !connected;
  document.querySelector('#pairing-instructions').hidden = !waiting;
  document.querySelector('#pairing-hint').hidden = !connected;
  const refresh = document.querySelector('#pairing-refresh');
  refresh.hidden = connected || asking || ['connecting','loading','failed','sas-confirming'].includes(state.phase);
  refresh.textContent = sas ? 'Wróć do kodu QR' : 'Wyświetl nowy kod';
  refresh.classList.toggle('primary', !sas);
  document.querySelector('#pairing-cancel').hidden = !(waiting || sas);
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
  // CCS/1 SEC:the payload is not kept; zero its modules once drawn (WN-94). The IPC JSON and GC copies are beyond reach.
  if (state.qr && Array.isArray(state.qr.dark)) state.qr.dark.fill(false);
}
// CCS/1 FLOW:files-state (engine, <=10/s) -> summary + newest rows; files-drag -> drop zone; files-notice -> status line.
const units = ['B', 'KB', 'MB', 'GB', 'TB'];
function bytes(value) { let i = 0; while (value >= 1024 && i < units.length - 1) { value /= 1024; i++; } return `${value.toFixed(i && value < 10 ? 1 : 0)} ${units[i]}`; }
const stateText = { queued: 'W kolejce', running: 'Wysyłanie', waiting: 'Czeka na telefon', done: 'Wysłano', failed: 'Nie wysłano', cancelled: 'Anulowano' };
function fileRow(row) {
  const item = document.createElement('li');
  const text = document.createElement('div');
  const name = document.createElement('span');
  name.className = 'name'; name.textContent = row.name; name.title = row.name;
  const detail = document.createElement('small');
  const percent = row.size ? Math.floor(row.done * 100 / row.size) : 100;
  detail.textContent = row.reason || (row.state === 'running' ? `${percent}% · ${bytes(row.size)}` : `${stateText[row.state] || row.state} · ${bytes(row.size)}`);
  if (row.state === 'failed') detail.className = 'problem';
  text.append(name, detail);
  item.append(text);
  const action = ['queued', 'running', 'waiting'].includes(row.state) ? 'cancel' : ['failed', 'cancelled'].includes(row.state) ? 'retry' : null;
  if (action) {
    const button = document.createElement('button');
    button.className = 'button quiet small';
    button.textContent = action === 'cancel' ? 'Anuluj' : 'Ponów';
    button.setAttribute('aria-label', `${button.textContent}: ${row.name}`);
    button.addEventListener('click', () => invoke?.('files_action', { action, id: row.id }).catch(error));
    item.append(button);
  }
  return item;
}
function renderFiles(state) {
  const section = document.querySelector('#transfers');
  section.hidden = state.rows.length === 0;
  if (section.hidden) return;
  const finished = state.rows.filter(r => ['done', 'failed', 'cancelled'].includes(r.state)).length;
  const percent = state.activeSize ? Math.floor(state.activeDone * 100 / state.activeSize) : 100;
  document.querySelector('#transfers-title').textContent = state.active ? `Wysyłanie na ${state.phone || 'telefon'}` : 'Wysłane pliki';
  document.querySelector('#transfers-summary').textContent = state.active ? `${state.active} w toku · ${bytes(state.activeDone)} z ${bytes(state.activeSize)}` : `${finished} zakończone`;
  const total = document.querySelector('#transfers-total');
  total.hidden = !state.active;
  total.setAttribute('aria-valuenow', String(percent));
  total.style.setProperty('--done', `${percent}%`);
  const rows = [...state.rows].sort((a, b) => (a.state === 'running' ? -1 : 0) - (b.state === 'running' ? -1 : 0)).slice(0, 4);
  document.querySelector('#transfers-list').replaceChildren(...rows.map(fileRow));
  document.querySelector('#transfers-clear').hidden = finished === 0;
}
let noticeTimer;
function renderNotice(notice) {
  const element = document.querySelector('#files-notice');
  clearTimeout(noticeTimer);
  element.textContent = notice.message;
  element.classList.toggle('error', notice.error);
  element.hidden = false;
  noticeTimer = setTimeout(() => { element.hidden = true; }, 6000);
}
function renderDrag(over) {
  const phone = document.querySelector('#phone-name').textContent;
  document.querySelector('#drop-title').textContent = `Upuść, aby wysłać na ${phone}`;
  document.querySelector('#drop-zone').hidden = !over;
}
document.querySelector('#send-files').addEventListener('click', () => invoke?.('files_action', { action: 'pick' }).catch(error));
document.querySelector('#transfers-clear').addEventListener('click', () => invoke?.('files_action', { action: 'clear' }).catch(error));
const pairingActions = { 'pairing-refresh': 'refresh', 'pairing-cancel': 'close', 'pairing-sas': 'sas', 'pairing-accept': 'sas-accept', 'pairing-reject': 'sas-reject' };
document.querySelectorAll(Object.keys(pairingActions).map(id => `#${id}`).join(',')).forEach(button => button.addEventListener('click', () => invoke?.('pairing_action', { action: pairingActions[button.id] }).catch(error)));
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
  await window.__TAURI__.event.listen('files-state', event => renderFiles(event.payload));
  await window.__TAURI__.event.listen('files-notice', event => renderNotice(event.payload));
  await window.__TAURI__.event.listen('files-drag', event => renderDrag(event.payload));
  renderFiles(await invoke('files_snapshot'));
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
