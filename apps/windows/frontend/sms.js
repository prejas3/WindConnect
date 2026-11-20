import { t, plural, locale } from './i18n.js';
// CCS/1 M:WN-55 the "Messages" window (PROTOCOL.md §9.11). FLOW:sms_status/sms-status -> threads (memory first, then the
// phone) -> click -> sms_thread (memory + newer from the phone) -> render; `sms-changed` (the phone's watch) refreshes both.
// SEC:every phone text goes in with textContent, never as HTML. PERF:one DocumentFragment per render; window.__smsOpenMs is
// the last click-to-painted time (check-sms-android.cjs reads it).
const tauri = window.__TAURI__;
const invoke = tauri?.core.invoke;
const listen = tauri?.event.listen;
const $ = s => document.querySelector(s);
const system = matchMedia('(prefers-color-scheme: dark)');
function theme() {
  let choice = 'system';
  try { choice = localStorage.getItem('windconnect-theme') || choice; } catch {}
  document.documentElement.dataset.theme = ['light', 'dark'].includes(choice) ? choice : (system.matches ? 'dark' : 'light');
}
window.addEventListener('storage', theme); system.addEventListener('change', theme); theme();

let status = { connected: false, available: false };
let threads = [];
let selected = null;
let opening = 0;
const photos = new Map();
const time = new Intl.DateTimeFormat(locale, { hour: '2-digit', minute: '2-digit' });
const day = new Intl.DateTimeFormat(locale, { weekday: 'long', day: 'numeric', month: 'long', year: 'numeric' });
const short = new Intl.DateTimeFormat(locale, { day: 'numeric', month: 'short' });
const sameDay = (a, b) => a.toDateString() === b.toDateString();
function when(ms) {
  const d = new Date(ms), now = new Date();
  return sameDay(d, now) ? time.format(d) : short.format(d);
}
function initials(text) {
  const words = text.replace(/[^\p{L}\p{N} ]/gu, '').trim().split(/\s+/).filter(Boolean);
  if (!words.length || /^\d/.test(words[0])) return '#';
  return (words[0][0] + (words[1]?.[0] || '')).toUpperCase();
}
function avatar(el, thread) {
  el.textContent = initials(thread.title);
  const person = thread.people.length === 1 && thread.people[0].photo ? thread.people[0] : null;
  if (!person || !status.available) return;
  const show = url => { if (!url || !el.isConnected) return; const img = document.createElement('img'); img.alt = ''; img.src = url; el.replaceChildren(img); };
  if (photos.has(person.address)) return show(photos.get(person.address));
  photos.set(person.address, null);
  invoke('sms_photo', { address: person.address }).then(url => { photos.set(person.address, url); show(url); }).catch(() => photos.delete(person.address));
}

function say(selector, text) { const el = $(selector); if (el.textContent !== text) el.textContent = text; }

function renderThreads() {
  const query = $('#search').value.trim().toLowerCase();
  const list = $('#thread-list');
  const fragment = document.createDocumentFragment();
  for (const th of threads) {
    if (query && !th.title.toLowerCase().includes(query) && !th.snippet.toLowerCase().includes(query)
      && !th.people.some(p => p.address.includes(query))) continue;
    const li = document.createElement('li');
    const b = document.createElement('button');
    b.className = 'thread' + (th.unread ? ' unread' : '');
    b.setAttribute('role', 'option');
    b.setAttribute('aria-selected', String(th.id === selected));
    b.dataset.id = th.id;
    const a = document.createElement('span'); a.className = 'avatar'; a.setAttribute('aria-hidden', 'true'); avatar(a, th);
    const title = document.createElement('span'); title.className = 'thread-title'; title.textContent = th.title;
    const date = document.createElement('span'); date.className = 'thread-date'; date.textContent = when(th.date_ms);
    const snippet = document.createElement('span'); snippet.className = 'thread-snippet'; snippet.textContent = th.snippet || t('sms.mms');
    b.append(a, title, date, snippet);
    if (th.unread) { const badge = document.createElement('span'); badge.className = 'badge'; badge.textContent = th.unread; badge.setAttribute('aria-label', t('sms.unread', { count: th.unread })); b.append(badge); }
    b.addEventListener('click', () => open(th.id));
    li.append(b);
    fragment.append(li);
  }
  list.replaceChildren(fragment);
  say('#threads-status', !status.connected && !threads.length ? t('sms.not_connected')
    : status.connected && !status.available ? t('sms.unavailable')
    : !threads.length ? t('sms.no_threads') : !status.connected ? t('sms.offline_cached') : '');
}

async function loadThreads(refresh) {
  try {
    threads = await invoke('sms_threads', { refresh });
    renderThreads();
  } catch (e) { say('#threads-status', String(e)); }
}

const statusLabel = { sent: '', pending: t('sms.status.sending'), failed: t('sms.status.not_sent') };
const deliveryLabel = { pending: t('sms.status.sent'), delivered: t('sms.status.delivered'), failed: t('sms.status.not_delivered') };
const pictures = new IntersectionObserver(entries => {
  for (const entry of entries) {
    if (!entry.isIntersecting) continue;
    const img = entry.target; pictures.unobserve(img);
    invoke('sms_part', { message: Number(img.dataset.message), part: Number(img.dataset.part) })
      .then(url => { img.src = url; }).catch(() => { img.replaceWith(Object.assign(document.createElement('span'), { className: 'attachment', textContent: t('sms.picture_failed') })); });
  }
}, { rootMargin: '200px' });
function renderMessages(thread, answer, keepBottom) {
  const box = $('#messages');
  const fromBottom = box.scrollHeight - box.scrollTop;
  const fragment = document.createDocumentFragment();
  let last = null;
  const group = thread && thread.people.length > 1;
  for (const m of answer.messages) {
    const d = new Date(m.date_ms);
    if (!last || !sameDay(last, d)) {
      const sep = document.createElement('li'); sep.className = 'day'; sep.textContent = day.format(d); fragment.append(sep);
    }
    last = d;
    const li = document.createElement('li');
    li.className = 'bubble' + (m.folder === 'in' ? '' : ' out') + (m.folder === 'failed' ? ' failed' : '');
    li.dataset.key = m.key;
    if (group && m.folder === 'in' && m.sender) { const s = document.createElement('span'); s.className = 'sender'; s.textContent = m.sender; li.append(s); }
    if (m.body) li.append(document.createTextNode(m.body));
    // WN-56: MMS pictures load when scrolled into view; other parts stay a note.
    for (const part of m.parts || []) {
      const img = document.createElement('img'); img.className = 'mms'; img.alt = t('sms.mms_photo');
      img.dataset.message = m.key.slice(1); img.dataset.part = part; pictures.observe(img); li.append(img);
    }
    const other = m.attachments - (m.parts?.length || 0);
    if (other > 0) { const a = document.createElement('span'); a.className = 'attachment'; a.textContent = other === 1 ? t('sms.attachment') : t('sms.attachments', { count: other }); li.append(a); }
    const meta = document.createElement('span'); meta.className = 'meta';
    meta.textContent = time.format(d) + (statusLabel[m.folder] || '') + (m.folder === 'sent' && m.delivery ? deliveryLabel[m.delivery] : ''); li.append(meta);
    fragment.append(li);
  }
  for (const local of localSends.values()) if (local.thread === selected) fragment.append(localBubble(local));
  $('#message-list').replaceChildren(fragment);
  $('#older').hidden = answer.complete || !status.available;
  box.scrollTop = keepBottom ? box.scrollHeight : box.scrollHeight - fromBottom;
}

function header(thread) {
  $('#conversation-head').hidden = false;
  say('#conversation-title', thread.title);
  const numbers = thread.people.map(p => p.address).join(', ');
  const count = plural('sms.count', thread.count);
  say('#conversation-sub', numbers === thread.title ? count : t('sms.numbers_count', { numbers, count }));
  avatar($('#conversation-avatar'), thread);
}

async function open(id) {
  const started = performance.now();
  const ticket = ++opening;
  selected = id;
  composing = false; $('#new-to').hidden = true; renderComposer();
  const thread = threads.find(th => th.id === id);
  renderThreads();
  if (thread) header(thread);
  $('#empty').hidden = true; $('#messages').hidden = false;
  say('#conversation-status', t('sms.loading'));
  try {
    const answer = await invoke('sms_thread', { id });
    if (ticket !== opening) return;
    renderMessages(thread, answer, true);
    say('#conversation-status', answer.cached ? t('sms.offline_thread') : '');
    requestAnimationFrame(() => requestAnimationFrame(() => {
      if (ticket === opening) window.__smsOpenMs = { id, ms: Math.round(performance.now() - started), count: answer.messages.length };
    }));
  } catch (e) { if (ticket === opening) say('#conversation-status', String(e)); }
}

async function older() {
  if (selected === null) return;
  const id = selected;
  $('#older').disabled = true;
  try {
    const answer = await invoke('sms_older', { id });
    if (id === selected) renderMessages(threads.find(th => th.id === id), answer, false);
  } catch (e) { say('#conversation-status', String(e)); }
  finally { $('#older').disabled = false; }
}

// A change on the phone: the list, and what is newer in the open thread (kept at the bottom if it was there).
let changing = false, again = false;
async function changed() {
  if (changing) { again = true; return; }
  changing = true;
  try {
    do {
      again = false;
      await loadThreads(true);
      if (selected !== null && status.available) {
        const box = $('#messages');
        const atBottom = box.scrollHeight - box.scrollTop - box.clientHeight < 40;
        const answer = await invoke('sms_thread', { id: selected }).catch(() => null);
        const thread = threads.find(th => th.id === selected);
        if (thread) header(thread);
        if (answer) renderMessages(thread, answer, atBottom);
      }
    } while (again);
  } finally { changing = false; }
}

function applyStatus(next) {
  const was = status;
  status = next;
  renderThreads();
  if (next.available && !was.available) changed();
  loadSims();
}

// CCS/1 FLOW:WN-56 composer -> sms_send (phone's SIM, picture scaled here) -> local bubble -> `sms-send` states -> on
// sent/failed the phone's own record replaces it through the watch. Sending is its own setting on the phone (sms_sims).
let sims = null, simsNote = '', composing = false, attachment = null;
const localSends = new Map();
async function loadSims() {
  if (!status.available) { sims = null; simsNote = ''; return renderComposer(); }
  try { sims = await invoke('sms_sims'); simsNote = ''; } catch (e) { sims = null; simsNote = String(e); }
  renderComposer();
}
const canSend = () => status.available && !!sims?.length;
function recipients() {
  if (composing) return $('#to').value.split(/[,;]/).map(a => a.trim()).filter(Boolean);
  return threads.find(th => th.id === selected)?.people.map(p => p.address) || [];
}
const GSM = /^[@£$¥èéùìòÇ\nØø\rÅåΔ_ΦΓΛΩΠΨΣΘΞÆæßÉ !"#¤%&'()*+,\-./0-9:;<=>?¡A-ZÄÖÑÜ§¿a-zäöñüà^{}\\\[~\]|€]*$/;
function segments(text) {
  if (!text) return '';
  const gsm = GSM.test(text);
  const single = gsm ? 160 : 70, multi = gsm ? 153 : 67;
  const n = text.length <= single ? 1 : Math.ceil(text.length / multi);
  return t(gsm ? 'sms.segments' : 'sms.segments_unicode', { chars: text.length, count: n });
}
function renderComposer() {
  $('#new-message').hidden = !status.available;
  $('#composer').hidden = !status.available || (selected === null && !composing);
  const ok = canSend();
  for (const id of ['#send', '#attach', '#body']) $(id).disabled = !ok;
  const select = $('#sim');
  select.hidden = !ok || sims.length < 2;
  if (ok && select.dataset.ids !== sims.map(s => s.id).join()) {
    select.replaceChildren(...sims.map(s => Object.assign(document.createElement('option'), { value: s.id, textContent: s.label, selected: s.default })));
    select.dataset.ids = sims.map(s => s.id).join();
  }
  const mms = !!attachment || recipients().length > 1;
  $('#body').placeholder = mms ? t('sms.mms_placeholder') : t('sms.body_placeholder');
  say('#composer-note', !ok ? simsNote : mms ? t(recipients().length > 1 ? 'sms.will_send_group_mms' : 'sms.will_send_mms') : segments($('#body').value));
}
function localBubble(local) {
  const li = document.createElement('li');
  li.className = 'bubble out local' + (local.state === 'failed' || local.state === 'refused' ? ' failed' : '');
  li.dataset.local = local.id;
  if (local.url) { const img = document.createElement('img'); img.className = 'mms'; img.alt = t('sms.sending_photo'); img.src = local.url; li.append(img); }
  if (local.body) li.append(document.createTextNode(local.body));
  const meta = document.createElement('span'); meta.className = 'meta';
  meta.textContent = ({ queued: t('sms.local.sending'), sent: t('sms.local.sent'), delivered: t('sms.local.delivered'), failed: t('sms.local.not_sent'), refused: local.message || t('sms.local.not_sent') })[local.state];
  li.append(meta);
  return li;
}
function showLocal(local) {
  const list = $('#message-list');
  const old = list.querySelector(`[data-local="${local.id}"]`);
  if (local.thread !== selected) return;
  if (old) old.replaceWith(localBubble(local)); else list.append(localBubble(local));
  $('#messages').scrollTop = $('#messages').scrollHeight;
}

// Pictures: scaled to at most 1280 px and re-encoded as JPEG of at most 300 KB (what carriers take), on a white ground.
function readImage(file) {
  return new Promise((resolve, reject) => {
    // A data: URL: the window's CSP allows no blob: images.
    const reader = new FileReader();
    const img = new Image();
    img.onload = () => {
      const scale = Math.min(1, 1280 / Math.max(img.naturalWidth, img.naturalHeight));
      const canvas = document.createElement('canvas');
      canvas.width = Math.max(1, Math.round(img.naturalWidth * scale)); canvas.height = Math.max(1, Math.round(img.naturalHeight * scale));
      const ctx = canvas.getContext('2d'); ctx.fillStyle = '#fff'; ctx.fillRect(0, 0, canvas.width, canvas.height); ctx.drawImage(img, 0, 0, canvas.width, canvas.height);
      let quality = .85, data;
      do { data = canvas.toDataURL('image/jpeg', quality); quality -= .15; } while (data.length * .75 > 300 * 1024 && quality > .2);
      if (data.length * .75 > 700 * 1024) return reject(Error(t('sms.image_too_large')));
      resolve({ mime: 'image/jpeg', url: data, data: data.slice(data.indexOf(',') + 1) });
    };
    img.onerror = () => reject(Error(t('sms.image_failed')));
    reader.onload = () => { img.src = reader.result; };
    reader.onerror = () => reject(Error(t('sms.image_failed')));
    reader.readAsDataURL(file);
  });
}
$('#attach').addEventListener('click', () => $('#attach-file').click());
$('#attach-file').addEventListener('change', async event => {
  const file = event.target.files?.[0]; event.target.value = '';
  if (!file) return;
  try { attachment = await readImage(file); $('#attachment-image').src = attachment.url; $('#attachment').hidden = false; }
  catch (e) { say('#composer-note', e.message); }
  renderComposer();
});
$('#attachment-remove').addEventListener('click', () => { attachment = null; $('#attachment').hidden = true; renderComposer(); });
$('#body').addEventListener('input', event => {
  const th = event.currentTarget; th.style.height = 'auto'; th.style.height = Math.min(160, th.scrollHeight) + 'px'; renderComposer();
});
$('#body').addEventListener('keydown', event => { if (event.key === 'Enter' && !event.shiftKey) { event.preventDefault(); $('#composer').requestSubmit(); } });
$('#to').addEventListener('input', renderComposer);
$('#new-message').addEventListener('click', () => {
  composing = true; selected = null; renderThreads();
  $('#new-to').hidden = false; $('#conversation-head').hidden = true; $('#messages').hidden = true; $('#empty').hidden = true;
  say('#conversation-status', ''); renderComposer(); $('#to').focus();
});
$('#composer').addEventListener('submit', async event => {
  event.preventDefault();
  const to = recipients(), body = $('#body').value;
  if (!canSend() || !to.length || (!body.trim() && !attachment)) return;
  const sim = $('#sim').hidden ? null : Number($('#sim').value);
  const sent = attachment;
  $('#send').disabled = true;
  try {
    const id = await invoke('sms_send', { to, body, sim, imageMime: sent?.mime ?? null, image: sent?.data ?? null });
    const local = { id, thread: composing ? null : selected, to, body, url: sent?.url, state: 'queued' };
    localSends.set(id, local);
    $('#body').value = ''; $('#body').style.height = ''; attachment = null; $('#attachment').hidden = true;
    showLocal(local);
  } catch (e) { say('#composer-note', String(e)); }
  finally { renderComposer(); }
});
function digits(a) { return a.replace(/\D/g, '').slice(-9); }
async function sendEvent({ id, state, message, done }) {
  const local = localSends.get(id);
  if (!local) return;
  local.state = state; local.message = message;
  showLocal(local);
  if (state === 'refused') { say('#composer-note', message || t('sms.not_sent')); return localSends.delete(id); }
  // The phone recorded it (sent or failed): its own message takes over through the watch.
  if (state === 'sent' || state === 'delivered' || state === 'failed' || done) {
    localSends.delete(id);
    if (state === 'failed') say('#composer-note', t('sms.send_failed'));
    if (local.thread === null) {
      await loadThreads(true);
      const want = local.to.map(digits).sort().join();
      const th = threads.find(th => th.people.map(p => digits(p.address)).sort().join() === want);
      if (th) return open(th.id);
    }
    changed();
  }
}

$('#search').addEventListener('input', renderThreads);
$('#refresh').addEventListener('click', () => changed());
$('#older').addEventListener('click', older);
$('#messages').addEventListener('scroll', event => { if (event.currentTarget.scrollTop < 40 && !$('#older').hidden && !$('#older').disabled) older(); });
document.addEventListener('keydown', event => {
  if (!['ArrowDown', 'ArrowUp'].includes(event.key) || event.target.closest?.('.messages')) return;
  const ids = [...document.querySelectorAll('.thread')].map(b => Number(b.dataset.id));
  if (!ids.length) return;
  event.preventDefault();
  const at = ids.indexOf(selected);
  const next = ids[Math.max(0, Math.min(ids.length - 1, at + (event.key === 'ArrowDown' ? 1 : -1)))];
  if (next !== selected) { open(next); document.querySelector(`.thread[data-id="${next}"]`)?.focus(); }
});

if (invoke) {
  listen('sms-status', e => applyStatus(e.payload));
  listen('sms-changed', () => { changed(); loadSims(); });
  listen('sms-send', e => sendEvent(e.payload));
  status = await invoke('sms_status');
  await loadThreads(false);
  if (status.available) loadThreads(true);
  loadSims();
}
