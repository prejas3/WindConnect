import { t } from './i18n.js';
// CCS/1 FLOW:WN-44 pairing-state.notifications -> "Notifications" in the panel; reply (Enter/Send) and ✕ -> notify_action.
// SEC:texts come from other apps: textContent only. The reply text is never logged; it stays in the field until the phone says
// it went out (Replied ok), and is kept after a failure so it isn't lost. Over 1024 UTF-8 bytes: blocked with a message, never
// trimmed. Rows are reused by tag, so a push from the PC never wipes what is being typed.
const invoke = window.__TAURI__?.core.invoke;
export const MAX_REPLY = 1024;
const bytes = text => new TextEncoder().encode(text).length;
const rows = new Map();

function statusText(row, state) {
  if (state.local) return state.local;
  return ({ sending: t('notifications.sending'), sent: t('transfers.sent'), failed: t('notifications.failed'), 'too-long': t('notifications.too_long', { max: MAX_REPLY }) })[row.status] || '';
}

function build(row) {
  const item = document.createElement('li');
  item.className = 'notification';
  const head = document.createElement('div'); head.className = 'notification-head';
  const text = document.createElement('div');
  const app = document.createElement('small'); app.className = 'notification-app';
  const title = document.createElement('span'); title.className = 'name';
  const body = document.createElement('p'); body.className = 'notification-text';
  text.append(app, title, body);
  const close = document.createElement('button');
  close.className = 'button quiet small notification-dismiss';
  close.innerHTML = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="m6 6 12 12M18 6 6 18"/></svg>';
  head.append(text, close);
  item.append(head);
  const state = { item, app, title, body, close, form: null, input: null, status: null, pending: null, local: '' };
  close.addEventListener('click', () => {
    invoke?.('notify_action', { action: 'dismiss', tag: row.tag }).catch(e => console.error(String(e).slice(0, 80)));
  });
  return state;
}

function ensureForm(state, tag) {
  if (state.form) return;
  const form = document.createElement('div'); form.className = 'notification-reply';
  const input = document.createElement('input');
  input.type = 'text'; input.placeholder = t('notifications.reply_placeholder'); input.autocomplete = 'off'; input.spellcheck = true;
  const send = document.createElement('button'); send.className = 'button primary small'; send.textContent = t('common.send');
  const status = document.createElement('small'); status.className = 'notification-status'; status.setAttribute('role', 'status');
  const submit = async () => {
    const text = input.value.trim();
    if (!text) { input.focus(); return; }
    if (bytes(text) > MAX_REPLY) {
      state.local = t('notifications.too_long_now', { bytes: bytes(text), max: MAX_REPLY });
      input.setAttribute('aria-invalid', 'true');
      render(state);
      return;
    }
    state.local = '';
    state.pending = 'queued';
    try { await invoke?.('notify_action', { action: 'reply', tag, text }); }
    catch (e) { state.pending = null; state.local = t('notifications.failed'); render(state); console.error(String(e).slice(0, 80)); }
  };
  input.addEventListener('input', () => {
    input.removeAttribute('aria-invalid');
    if (state.local && bytes(input.value.trim()) <= MAX_REPLY) { state.local = ''; render(state); }
  });
  input.addEventListener('keydown', e => { if (e.key === 'Enter') { e.preventDefault(); submit(); } if (e.key === 'Escape') e.stopPropagation(); });
  send.addEventListener('click', submit);
  form.append(input, send, status);
  state.item.append(form);
  Object.assign(state, { form, input, status, send });
}

function render(state) {
  const row = state.row;
  if (state.status) {
    const line = statusText(row, state);
    if (state.status.textContent !== line) state.status.textContent = line;
    state.status.classList.toggle('problem', !!state.local || row.status === 'failed' || row.status === 'too-long');
  }
}

function update(state, row) {
  const previous = state.row;
  state.row = row;
  state.app.textContent = row.app || t('notifications.phone');
  const title = row.title || row.app || t('notifications.notification');
  state.title.textContent = title; state.title.title = title;
  state.body.textContent = row.text || '';
  state.body.hidden = !row.text;
  state.close.setAttribute('aria-label', t('notifications.dismiss', { title }));
  if (row.reply) {
    ensureForm(state, row.tag);
    state.input.setAttribute('aria-label', `${row.reply}: ${title}`);
    state.send.setAttribute('aria-label', t('notifications.send_reply', { title }));
    // Sending: the PC queued it ('sending'); then the phone's answer. Clear only after the phone said it went out.
    if (state.pending === 'queued' && row.status === 'sending') state.pending = 'sending';
    if (state.pending === 'sending' && row.status === 'sent') { state.input.value = ''; state.pending = null; }
    if (state.pending && row.status === 'failed') state.pending = null;
    // A too-long reply typed in the toast comes here to be shortened, not lost.
    if (row.draft && row.draft !== previous?.draft && !state.input.value) state.input.value = row.draft;
  }
  render(state);
}

export function renderNotifications(list, connected) {
  const section = document.querySelector('#notifications');
  if (!section) return;
  const items = connected && Array.isArray(list) ? list : [];
  section.hidden = items.length === 0;
  const seen = new Set();
  const ordered = items.map(row => {
    seen.add(row.tag);
    let state = rows.get(row.tag);
    if (!state) { state = build(row); rows.set(row.tag, state); }
    update(state, row);
    return state.item;
  });
  for (const [tag, state] of rows) if (!seen.has(tag)) { state.item.remove(); rows.delete(tag); }
  const container = document.querySelector('#notification-list');
  // Reorder without detaching the focused field (moving a node drops its focus).
  ordered.forEach((item, index) => { if (container.children[index] !== item) container.insertBefore(item, container.children[index] || null); });
}
