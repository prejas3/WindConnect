// CCS/1 M:WN-55 the "Wiadomości" window (PROTOCOL.md §9.11). FLOW:sms_status/sms-status -> threads (memory first, then the
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
const time = new Intl.DateTimeFormat('pl-PL', { hour: '2-digit', minute: '2-digit' });
const day = new Intl.DateTimeFormat('pl-PL', { weekday: 'long', day: 'numeric', month: 'long', year: 'numeric' });
const short = new Intl.DateTimeFormat('pl-PL', { day: 'numeric', month: 'short' });
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
  for (const t of threads) {
    if (query && !t.title.toLowerCase().includes(query) && !t.snippet.toLowerCase().includes(query)
      && !t.people.some(p => p.address.includes(query))) continue;
    const li = document.createElement('li');
    const b = document.createElement('button');
    b.className = 'thread' + (t.unread ? ' unread' : '');
    b.setAttribute('role', 'option');
    b.setAttribute('aria-selected', String(t.id === selected));
    b.dataset.id = t.id;
    const a = document.createElement('span'); a.className = 'avatar'; a.setAttribute('aria-hidden', 'true'); avatar(a, t);
    const title = document.createElement('span'); title.className = 'thread-title'; title.textContent = t.title;
    const date = document.createElement('span'); date.className = 'thread-date'; date.textContent = when(t.date_ms);
    const snippet = document.createElement('span'); snippet.className = 'thread-snippet'; snippet.textContent = t.snippet || 'Wiadomość multimedialna';
    b.append(a, title, date, snippet);
    if (t.unread) { const badge = document.createElement('span'); badge.className = 'badge'; badge.textContent = t.unread; badge.setAttribute('aria-label', `${t.unread} nieprzeczytane`); b.append(badge); }
    b.addEventListener('click', () => open(t.id));
    li.append(b);
    fragment.append(li);
  }
  list.replaceChildren(fragment);
  say('#threads-status', !status.connected && !threads.length ? 'Telefon nie jest połączony.'
    : status.connected && !status.available ? 'Telefon nie udostępnia SMS. Włącz „SMS na komputerze” w ustawieniach WindConnect na telefonie.'
    : !threads.length ? 'Brak wątków.' : !status.connected ? 'Telefon rozłączony: widzisz to, co już było odczytane.' : '');
}

async function loadThreads(refresh) {
  try {
    threads = await invoke('sms_threads', { refresh });
    renderThreads();
  } catch (e) { say('#threads-status', String(e)); }
}

const statusLabel = { sent: '', pending: ' · wysyłanie…', failed: ' · nie wysłano' };
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
    if (m.attachments) { const a = document.createElement('span'); a.className = 'attachment'; a.textContent = m.attachments === 1 ? 'Załącznik (pokaże go kolejna wersja)' : `Załączniki: ${m.attachments}`; li.append(a); }
    const meta = document.createElement('span'); meta.className = 'meta'; meta.textContent = time.format(d) + (statusLabel[m.folder] || ''); li.append(meta);
    fragment.append(li);
  }
  $('#message-list').replaceChildren(fragment);
  $('#older').hidden = answer.complete || !status.available;
  box.scrollTop = keepBottom ? box.scrollHeight : box.scrollHeight - fromBottom;
}

function header(thread) {
  $('#conversation-head').hidden = false;
  say('#conversation-title', thread.title);
  const numbers = thread.people.map(p => p.address).join(', ');
  say('#conversation-sub', numbers === thread.title ? `${thread.count} wiadomości` : `${numbers} · ${thread.count} wiadomości`);
  avatar($('#conversation-avatar'), thread);
}

async function open(id) {
  const started = performance.now();
  const ticket = ++opening;
  selected = id;
  const thread = threads.find(t => t.id === id);
  renderThreads();
  if (thread) header(thread);
  $('#empty').hidden = true; $('#messages').hidden = false;
  say('#conversation-status', 'Wczytywanie…');
  try {
    const answer = await invoke('sms_thread', { id });
    if (ticket !== opening) return;
    renderMessages(thread, answer, true);
    say('#conversation-status', answer.cached ? 'Telefon rozłączony: to, co już było odczytane.' : '');
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
    if (id === selected) renderMessages(threads.find(t => t.id === id), answer, false);
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
        const thread = threads.find(t => t.id === selected);
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
  listen('sms-changed', () => changed());
  status = await invoke('sms_status');
  await loadThreads(false);
  if (status.available) loadThreads(true);
}
