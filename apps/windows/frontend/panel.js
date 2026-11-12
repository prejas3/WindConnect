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
// CCS/1 FLOW:device list (WC-28) -> device_action(rename|unpair|disconnect, key) -> the pairing actor; one row edits at a time.
let editing = null;
function deviceRow(row) {
  const item = document.createElement('li');
  const head = document.createElement('div'); head.className = 'device-head';
  const text = document.createElement('div');
  const name = document.createElement('span'); name.className = 'name'; name.textContent = row.name; name.title = row.name;
  const detail = document.createElement('small');
  const when = new Date(row.paired_at * 1000).toLocaleDateString('pl-PL');
  detail.textContent = row.owed ? `Usunięto · telefon dowie się przy następnym połączeniu` : `${row.connected ? 'Połączony' : 'Niepołączony'} · sparowano ${when} · ${row.fingerprint}`;
  detail.className = row.owed ? 'owed' : row.connected ? 'connected' : '';
  text.append(name, detail);
  const actions = document.createElement('div'); actions.className = 'device-actions';
  const button = (label, kind, onClick, aria) => { const b = document.createElement('button'); b.className = `button ${kind} small`; b.textContent = label; b.setAttribute('aria-label', `${aria || label}: ${row.name}`); b.addEventListener('click', onClick); return b; };
  const send = (action, extra = {}) => invoke?.('device_action', { action, key: row.key, ...extra }).catch(error);
  if (!row.owed) {
    actions.append(button('Zmień nazwę', 'quiet', () => { editing = { key: row.key, mode: 'rename' }; renderDevices(lastDevices); }));
    if (row.connected) actions.append(button('Rozłącz', 'quiet', () => send('disconnect')));
    actions.append(button('Usuń', 'quiet', () => { editing = { key: row.key, mode: 'unpair' }; renderDevices(lastDevices); }, 'Usuń parowanie'));
  }
  head.append(text, actions);
  item.append(head);
  if (editing?.key === row.key && editing.mode === 'rename') {
    const label = document.createElement('label'); label.className = 'field';
    const caption = document.createElement('span'); caption.textContent = 'Nazwa widoczna na tym komputerze';
    const input = document.createElement('input'); input.value = editing.value ?? row.name; input.maxLength = 64;
    input.addEventListener('input', () => { editing.value = input.value; input.removeAttribute('aria-invalid'); });
    const buttons = document.createElement('div'); buttons.className = 'button-row';
    const save = () => { const value = input.value.trim(); if (!value) { input.setAttribute('aria-invalid', 'true'); return; } editing = null; send('rename', { name: value }); };
    input.addEventListener('keydown', e => { if (e.key === 'Enter') save(); if (e.key === 'Escape') { e.stopPropagation(); editing = null; renderDevices(lastDevices); } });
    buttons.append(button('Zapisz', 'primary', save), button('Anuluj', 'quiet', () => { editing = null; renderDevices(lastDevices); }));
    label.append(caption, input, buttons);
    item.append(label);
    queueMicrotask(() => input.focus());
  } else if (editing?.key === row.key && editing.mode === 'unpair') {
    const confirm = document.createElement('div'); confirm.className = 'confirm'; confirm.setAttribute('role', 'group');
    const question = document.createElement('p');
    question.textContent = `Usunąć parowanie z: ${row.name}? Telefon też je usunie${row.connected ? '' : ' przy następnym połączeniu'}. Ponowne połączenie wymaga nowego parowania.`;
    const buttons = document.createElement('div'); buttons.className = 'button-row';
    buttons.append(button('Usuń parowanie', 'danger', () => { editing = null; send('unpair'); }), button('Anuluj', 'quiet', () => { editing = null; renderDevices(lastDevices); }));
    confirm.append(question, buttons);
    item.append(confirm);
  }
  return item;
}
let lastDevices = [];
function renderDevices(rows) {
  lastDevices = rows || [];
  if (editing && !lastDevices.some(r => r.key === editing.key)) editing = null;
  document.querySelector('#devices').hidden = lastDevices.length === 0;
  document.querySelector('#device-list').replaceChildren(...lastDevices.map(deviceRow));
}
document.querySelector('#device-add').addEventListener('click', () => invoke?.('pairing_action', { action: 'refresh' }).catch(error));
// WN-49: level and charging from the phone (pushed in pairing-state, never polled). Not a live region: a label only.
function renderBattery(battery) {
  const box = document.querySelector('#phone-battery');
  box.hidden = !battery;
  if (!battery) return;
  const level = Math.max(0, Math.min(100, Number(battery.level) || 0));
  const charging = battery.charging === true;
  box.classList.toggle('charging', charging);
  box.classList.toggle('low', !charging && level <= 15);
  box.querySelector('.level').setAttribute('width', String(Math.max(level > 0 ? 1.5 : 0, 16 * level / 100)));
  box.querySelector('.battery-text').textContent = `${level}%`;
  box.setAttribute('aria-label', `Bateria telefonu ${level}%${charging ? ', ładuje się' : ''}`);
}

// WN-50: ring the phone; the tile turns into "Zatrzymaj" while it starts or rings. State comes pushed in pairing-state.
function renderFind(find, connected) {
  const button = document.querySelector('#find-phone');
  const status = document.querySelector('#find-status');
  const active = !!find && (find.state === 'starting' || find.state === 'ringing');
  button.disabled = !connected || !find || (!find.available && !active);
  button.setAttribute('aria-pressed', String(active));
  button.querySelector('.find-label').textContent = active ? 'Zatrzymaj' : 'Dzwoń';
  button.setAttribute('aria-label', active ? 'Zatrzymaj dzwonienie telefonu' : 'Zadzwoń telefonem, by go znaleźć');
  const line = !connected ? '' : active ? (find.state === 'ringing' ? 'Telefon dzwoni, także w trybie cichym' : 'Łączenie z telefonem…')
    : find?.last || (find && !find.available ? 'Dzwonienie niedostępne: włącz je w ustawieniach WindConnect na telefonie' : '');
  if (status.textContent !== line) status.textContent = line;
}

function renderPairing(state) {
  renderDevices(state.devices);
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
  // A phone keeps its session (and these actions) while another one is being paired (WC-28).
  document.querySelector('#quick-actions').hidden = !(state.connected || connected);
  renderBattery(state.connected || connected ? state.battery : null);
  renderFind(state.find, !!(state.connected || connected));
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
document.querySelector('#find-phone').addEventListener('click', event => {
  const ringing = event.currentTarget.getAttribute('aria-pressed') === 'true';
  invoke?.('pairing_action', { action: ringing ? 'ring-stop' : 'ring' }).catch(error);
});
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
import './clipboard.js';
