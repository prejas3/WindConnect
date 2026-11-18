import { t, locale } from './i18n.js';
import { createTransfers } from './transfers.js';
import './touchpad.js';
const invoke = window.__TAURI__?.core.invoke;
const renderFiles = createTransfers(document, (action, id) => invoke?.('files_action', { action, id }).catch(value => renderNotice({ message: String(value), error: true })));
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
  const when = new Date(row.paired_at * 1000).toLocaleDateString(locale);
  detail.textContent = row.owed ? t('devices.removed_pending') : t('devices.state_line', { state: t(row.connected ? 'devices.connected' : 'devices.disconnected'), when, fingerprint: row.fingerprint });
  detail.className = row.owed ? 'owed' : row.connected ? 'connected' : '';
  text.append(name, detail);
  const actions = document.createElement('div'); actions.className = 'device-actions';
  const button = (label, kind, onClick, aria) => { const b = document.createElement('button'); b.className = `button ${kind} small`; b.textContent = label; b.setAttribute('aria-label', t('common.action_for', { action: aria || label, name: row.name })); b.addEventListener('click', onClick); return b; };
  const send = (action, extra = {}) => invoke?.('device_action', { action, key: row.key, ...extra }).catch(error);
  if (!row.owed) {
    actions.append(button(t('common.rename'), 'quiet', () => { editing = { key: row.key, mode: 'rename' }; renderDevices(lastDevices); }));
    if (row.connected) actions.append(button(t('common.disconnect'), 'quiet', () => send('disconnect')));
    actions.append(button(t('common.remove'), 'quiet', () => { editing = { key: row.key, mode: 'unpair' }; renderDevices(lastDevices); }, t('devices.unpair')));
  }
  head.append(text, actions);
  item.append(head);
  if (editing?.key === row.key && editing.mode === 'rename') {
    const label = document.createElement('label'); label.className = 'field';
    const caption = document.createElement('span'); caption.textContent = t('devices.rename_label');
    const input = document.createElement('input'); input.value = editing.value ?? row.name; input.maxLength = 64;
    input.addEventListener('input', () => { editing.value = input.value; input.removeAttribute('aria-invalid'); });
    const buttons = document.createElement('div'); buttons.className = 'button-row';
    const save = () => { const value = input.value.trim(); if (!value) { input.setAttribute('aria-invalid', 'true'); return; } editing = null; send('rename', { name: value }); };
    input.addEventListener('keydown', e => { if (e.key === 'Enter') save(); if (e.key === 'Escape') { e.stopPropagation(); editing = null; renderDevices(lastDevices); } });
    buttons.append(button(t('common.save'), 'primary', save), button(t('common.cancel'), 'quiet', () => { editing = null; renderDevices(lastDevices); }));
    label.append(caption, input, buttons);
    item.append(label);
    queueMicrotask(() => input.focus());
  } else if (editing?.key === row.key && editing.mode === 'unpair') {
    const confirm = document.createElement('div'); confirm.className = 'confirm'; confirm.setAttribute('role', 'group');
    const question = document.createElement('p');
    question.textContent = t(row.connected ? 'devices.unpair_now' : 'devices.unpair_later', { name: row.name });
    const buttons = document.createElement('div'); buttons.className = 'button-row';
    buttons.append(button(t('devices.unpair'), 'danger', () => { editing = null; send('unpair'); }), button(t('common.cancel'), 'quiet', () => { editing = null; renderDevices(lastDevices); }));
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
  box.setAttribute('aria-label', t(charging ? 'battery.label_charging' : 'battery.label', { level: `${level}%` }));
}

// WN-46: the phone's player. Every text from it goes in with textContent only (it comes from other apps); the art only as a
// data: URL of JPEG/PNG (checked by the PC by magic bytes). Progress moves on a local timer from the pushed state: no polling.
const MEDIA = { play: 1, pause: 2, next: 4, previous: 8, seek: 16 };
let media = null;
let mediaTimer = null;
const clock = ms => { const s = Math.max(0, Math.floor(ms / 1000)); return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`; };
function mediaPosition() {
  if (!media) return 0;
  const moved = media.playing ? (Date.now() - media.received_ms) * media.speed_permille / 1000 : 0;
  const at = media.position_ms + Math.max(0, moved);
  return media.duration_ms ? Math.min(at, media.duration_ms) : at;
}
function tickMedia() {
  if (!media) return;
  const at = mediaPosition();
  const total = media.duration_ms || 0;
  const bar = document.querySelector('#media-progress');
  bar.firstElementChild.style.width = total ? `${(100 * at / total).toFixed(2)}%` : '0';
  bar.setAttribute('aria-valuemax', String(Math.round(total / 1000)));
  bar.setAttribute('aria-valuenow', String(Math.round(at / 1000)));
  bar.setAttribute('aria-valuetext', total ? `${clock(at)} z ${clock(total)}` : clock(at));
  document.querySelector('#media-elapsed').textContent = clock(at);
  document.querySelector('#media-total').textContent = total ? clock(total) : '';
}
function renderMedia(next) {
  media = next || null;
  const card = document.querySelector('#media');
  card.hidden = !media;
  clearInterval(mediaTimer); mediaTimer = null;
  if (!media) return;
  document.querySelector('#media-app').textContent = media.app ? t('media.on_phone_app', { app: media.app }) : t('media.on_phone');
  document.querySelector('#media-title').textContent = media.title || t('media.untitled');
  document.querySelector('#media-sub').textContent = [media.artist, media.album].filter(Boolean).join(' · ');
  const art = document.querySelector('#media-art');
  const ok = typeof media.art === 'string' && /^data:image\/(jpeg|png);base64,[A-Za-z0-9+/=]+$/.test(media.art);
  art.hidden = !ok;
  if (ok) art.src = media.art; else art.removeAttribute('src');
  const can = bit => (media.actions & bit) !== 0;
  const toggle = document.querySelector('#media-toggle');
  toggle.classList.toggle('playing', media.playing);
  toggle.setAttribute('aria-label', media.playing ? t('media.pause') : t('panel.media-toggle_label'));
  toggle.disabled = !can(media.playing ? MEDIA.pause : MEDIA.play);
  document.querySelector('#media-next').disabled = !can(MEDIA.next);
  document.querySelector('#media-previous').disabled = !can(MEDIA.previous);
  document.querySelector('#media-progress').setAttribute('aria-disabled', String(!can(MEDIA.seek) || !media.duration_ms));
  tickMedia();
  if (media.playing) mediaTimer = setInterval(() => { if (document.visibilityState === 'visible') tickMedia(); }, 1000);
}
const mediaAction = (action, position_ms) => invoke?.('media_action', position_ms === undefined ? { action } : { action, positionMs: Math.round(position_ms) }).catch(error);
document.querySelector('#media-toggle').addEventListener('click', () => mediaAction('toggle'));
document.querySelector('#media-next').addEventListener('click', () => mediaAction('next'));
document.querySelector('#media-previous').addEventListener('click', () => mediaAction('previous'));
const seekable = () => media && (media.actions & MEDIA.seek) && media.duration_ms;
document.querySelector('#media-progress').addEventListener('click', event => {
  if (!seekable()) return;
  const box = event.currentTarget.getBoundingClientRect();
  mediaAction('seek', media.duration_ms * Math.min(1, Math.max(0, (event.clientX - box.left) / box.width)));
});
document.querySelector('#media-progress').addEventListener('keydown', event => {
  if (!seekable() || !['ArrowLeft', 'ArrowRight'].includes(event.key)) return;
  event.preventDefault();
  mediaAction('seek', Math.min(media.duration_ms, Math.max(0, mediaPosition() + (event.key === 'ArrowRight' ? 10_000 : -10_000))));
});

// WN-57: the phone's missed calls (newest first, at most 10), as text only.
const missedTime = new Intl.DateTimeFormat(locale, { weekday: 'short', hour: '2-digit', minute: '2-digit' });
function renderMissed(items) {
  const section = document.querySelector('#missed-calls');
  section.hidden = !items.length;
  const key = JSON.stringify(items);
  if (section.dataset.key === key) return;
  section.dataset.key = key;
  document.querySelector('#missed-list').replaceChildren(...items.slice(0, 5).map(m => {
    const li = document.createElement('li'); li.className = 'missed-row';
    const who = document.createElement('span'); who.className = 'missed-who'; who.textContent = m.who;
    const when = document.createElement('span'); when.className = 'subtle'; when.textContent = (m.number ? m.number + ' · ' : '') + missedTime.format(new Date(m.date_ms));
    li.append(who, when); return li;
  }));
}

// WN-50: ring the phone; the tile turns into t('find.stop') while it starts or rings. State comes pushed in pairing-state.
function renderFind(find, connected) {
  const button = document.querySelector('#find-phone');
  const status = document.querySelector('#find-status');
  const active = !!find && (find.state === 'starting' || find.state === 'ringing');
  button.disabled = !connected || !find || (!find.available && !active);
  button.setAttribute('aria-pressed', String(active));
  button.querySelector('.find-label').textContent = active ? t('find.stop') : t('panel.dzwon');
  button.setAttribute('aria-label', active ? t('find.stop_label') : t('find.ring_label'));
  const line = !connected ? '' : active ? (find.state === 'ringing' ? t('find.ringing') : t('find.connecting'))
    : find?.last || (find && !find.available ? t('find.unavailable') : '');
  if (status.textContent !== line) status.textContent = line;
}

// WN-145: the panel kept its last scroll, so it could open mid-way (looking cut off at the top). Each opening starts at
// the top, except while a pairing shows its QR or code: that card (near the bottom) is brought into view, also when the
// QR appears with the panel open ("Dodaj telefon").
let pairingShown = false;
function pairingVisible() { return !document.querySelector('#pairing-qr').hidden || !document.querySelector('#pairing-code').hidden; }
function showPairing() { document.querySelector('#pairing-qr').closest('section').scrollIntoView({ block: 'nearest' }); }
function scrollForOpening() {
  if (pairingVisible()) showPairing();
  else { document.scrollingElement.scrollTop = 0; document.body.scrollTop = 0; }
}
function renderPairing(state) {
  renderDevices(state.devices);
  const waiting = state.phase === 'waiting';
  const connected = state.phase === 'connected';
  const sas = state.phase.startsWith('sas-');
  const asking = state.phase === 'sas-code';
  document.querySelector('#phone-name').textContent = state.phone || t('panel.phone-name');
  document.querySelector('#connection-state').textContent = ({ connected: t('panel.state.connected'), waiting: t('panel.state.waiting'), 'sas-waiting': t('panel.state.waiting'), 'sas-code': t('panel.state.compare'), 'sas-confirming': t('panel.state.compare'), connecting: t('panel.state.connecting'), offline: t('panel.state.offline'), loading: t('main.status') })[state.phase] || t('menu.tray-state');
  document.querySelector('#pairing-title').textContent = connected ? (state.phone || t('panel.title.connected')) : waiting ? t('panel.pairing-title') : sas ? t('panel.pairing-sas') : state.phase === 'connecting' ? t('panel.title.finishing') : t('panel.title.pairing');
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
  // WN-55: the "Wiadomości" window; it explains itself when the phone doesn't share SMS.
  document.querySelector('#open-sms').hidden = !(state.connected || connected);
  // WN-100: links to the phone, when it takes them.
  document.querySelector('#open-on-phone').hidden = !((state.connected || connected) && state.open_on_phone);
  document.querySelector('#open-sms').setAttribute('aria-description', state.sms ? t('panel.sms_title') : t('panel.sms_unavailable'));
  renderMedia(state.connected || connected ? state.media : null);
  renderNotifications(state.notifications, !!(state.connected || connected));
  renderMissed(state.call_toasts && (state.connected || connected) ? state.missed_calls || [] : []);
  renderPhoneVolume(state.connected || connected ? state.phone_volume : null);
  document.querySelector('#pairing-instructions').hidden = !waiting;
  document.querySelector('#pairing-hint').hidden = !connected;
  const refresh = document.querySelector('#pairing-refresh');
  refresh.hidden = connected || asking || ['connecting','loading','failed','sas-confirming'].includes(state.phase);
  refresh.textContent = sas ? t('onboarding.back_to_qr') : t('panel.pairing-refresh');
  refresh.classList.toggle('primary', !sas);
  document.querySelector('#pairing-cancel').hidden = !(waiting || sas);
  document.querySelector('#pairing-warning').hidden = !state.skipped_records;
  document.querySelector('#pairing-warning').textContent = state.skipped_records ? t('pairing.skipped', { count: state.skipped_records }) : '';
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
  const shown = pairingVisible();
  if (shown && !pairingShown) showPairing();
  pairingShown = shown;
}
// CCS/1 FLOW:files-state (engine, <=10/s) -> summary + newest rows; files-drag -> drop zone; files-notice -> status line.
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
  document.querySelector('#drop-title').textContent = t('panel.drop_to', { phone });
  document.querySelector('#drop-zone').hidden = !over;
}
const openStatus = { done: t('open.done'), refused: t('open.refused'), 'bad-argument': t('open.bad_url'), failed: t('open.failed'), busy: t('open.busy'), unavailable: t('open.unavailable') };
document.querySelector('#open-on-phone').addEventListener('submit', async event => {
  event.preventDefault();
  const input = document.querySelector('#phone-url');
  const status = document.querySelector('#open-on-phone-status');
  if (!input.value.trim()) return;
  try {
    const result = await invoke?.('open_on_phone', { url: input.value });
    status.textContent = openStatus[result] || t('open.failed');
    if (result === 'done') input.value = '';
  } catch (e) { error(e); }
});
document.querySelector('#open-sms').addEventListener('click', () => invoke?.('sms_open').catch(error));
document.querySelector('#send-files').addEventListener('click', () => invoke?.('files_action', { action: 'pick' }).catch(error));
document.querySelector('#find-phone').addEventListener('click', event => {
  const ringing = event.currentTarget.getAttribute('aria-pressed') === 'true';
  invoke?.('pairing_action', { action: ringing ? 'ring-stop' : 'ring' }).catch(error);
});

const pairingActions = { 'pairing-refresh': 'refresh', 'pairing-cancel': 'close', 'pairing-sas': 'sas', 'pairing-accept': 'sas-accept', 'pairing-reject': 'sas-reject' };
document.querySelectorAll(Object.keys(pairingActions).map(id => `#${id}`).join(',')).forEach(button => button.addEventListener('click', () => invoke?.('pairing_action', { action: pairingActions[button.id] }).catch(error)));
// CCS/1 PERF:preload font/layout before ready; two frames acknowledge content submission, not physical display scanout.
async function refreshShortcut() {
  const state = await invoke('panel_snapshot');
  document.querySelector('#shortcut-error').hidden = !state.shortcut_error;
  document.querySelector('#shortcut').textContent = state.shortcut_error ? t('panel.shortcut_unavailable') : 'Win + Shift + W';
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
    scrollForOpening();
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
import './photos.js';
import { renderNotifications } from './notifications.js';
import { renderPhoneVolume } from './phonevolume.js';
