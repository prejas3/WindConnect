// CCS/1 SEC:only settings/status cross IPC; clipboard payload stays in the native adapter. WN-37: the history list carries a one-line preview or a thumbnail, never the full content; copy goes by id.
const invoke = window.__TAURI__?.core.invoke;
const EMPTY_ON = 'Tu pojawią się ostatnio przesłane schowki. Treści oznaczone jako wrażliwe (np. hasła z menedżera) nigdy tu nie trafiają.';
const EMPTY_OFF = 'Synchronizacja wyłączona: historia nie jest zapisywana. Przypięte pozycje zostają.';
function render(state) {
  document.querySelectorAll('[data-clipboard-controls]').forEach(element => { element.hidden = false; });
  document.querySelectorAll('[data-clipboard-toggle]').forEach(element => { element.setAttribute('aria-checked', String(state.enabled)); element.disabled = false; });
  document.querySelectorAll('[data-clipboard-status]').forEach(element => { element.textContent = state.status; });
  // WN-37: sync off records nothing (and cleared the unpinned history); say so where the list is empty.
  document.querySelectorAll('[data-history-empty]').forEach(element => { element.textContent = state.enabled ? EMPTY_ON : EMPTY_OFF; });
  // WN-36: the image limit (5 / 16 / 50 MB), one pressed button per group.
  document.querySelectorAll('[data-clipboard-limit] button').forEach(button => button.setAttribute('aria-pressed', String(Number(button.dataset.limit) === state.image_limit_mb)));
  document.querySelectorAll('[data-clipboard-shortcut]').forEach(element => { element.textContent = state.shortcut_error ? 'Ctrl + Alt + Shift + C jest zajęty. Użyj przycisku Schowek.' : 'Wyślij: Ctrl + Alt + Shift + C'; });
}
async function refresh() { if (invoke) render(await invoke('clipboard_snapshot')); }
function failed() { document.querySelectorAll('[data-clipboard-status]').forEach(element => { element.textContent = 'Nie udało się wykonać operacji schowka.'; }); }
document.querySelectorAll('[data-clipboard-send]').forEach(button => button.addEventListener('click', () => invoke?.('clipboard_action', { action: 'send' }).catch(failed)));
document.querySelectorAll('[data-clipboard-limit] button').forEach(button => button.addEventListener('click', async () => {
  if (!invoke) return;
  try { await invoke('clipboard_action', { action: 'image-limit', limit: Number(button.dataset.limit) }); await refresh(); }
  catch { failed(); }
}));
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
// WN-37: clipboard history. Text only through textContent; actions by id.
const SVG = 'http://www.w3.org/2000/svg';
const PIN = 'M9 4h6l-1 6 3 3H7l3-3zM12 13v7';
const REMOVE = 'm7 7 10 10M17 7 7 17';
const TEXT_GLYPH = 'M6 7h12M6 12h12M6 17h8';
function icon(d, className) {
  const svg = document.createElementNS(SVG, 'svg'); svg.setAttribute('viewBox', '0 0 24 24'); svg.setAttribute('aria-hidden', 'true');
  if (className) svg.setAttribute('class', className);
  const path = document.createElementNS(SVG, 'path'); path.setAttribute('d', d); svg.append(path); return svg;
}
const time = new Intl.DateTimeFormat('pl-PL', { hour: '2-digit', minute: '2-digit' });
function meta(entry) {
  const where = entry.direction === 'to-phone' ? 'Do telefonu' : 'Z telefonu';
  const parts = [where, time.format(new Date(entry.at))];
  if (entry.kind === 'image') parts.splice(1, 0, `${entry.width}×${entry.height}`);
  if (entry.pinned) parts.push('przypięte');
  return parts.join(' · ');
}
function item(entry) {
  const li = document.createElement('li'); li.dataset.id = String(entry.id); li.dataset.pinned = String(entry.pinned);
  const copy = document.createElement('button'); copy.className = 'history-copy'; copy.dataset.historyCopy = '';
  const text = document.createElement('span'); const name = document.createElement('span'); name.className = 'name';
  const small = document.createElement('small');
  if (entry.kind === 'image') {
    const img = document.createElement('img'); img.alt = ''; img.src = entry.thumbnail; copy.append(img);
    name.textContent = 'Obraz';
  } else { copy.append(icon(TEXT_GLYPH, 'glyph')); name.textContent = entry.preview; }
  small.textContent = entry.ready ? meta(entry) : 'Obraz dochodzi…';
  copy.disabled = !entry.ready;
  copy.setAttribute('aria-label', `Kopiuj: ${entry.kind === 'image' ? `obraz ${entry.width}×${entry.height}` : entry.preview}. ${small.textContent}`);
  text.append(name, small); copy.append(text);
  const pin = document.createElement('button'); pin.className = 'history-action'; pin.dataset.historyPin = '';
  pin.setAttribute('aria-pressed', String(entry.pinned)); pin.setAttribute('aria-label', entry.pinned ? 'Odepnij' : 'Przypnij'); pin.title = pin.getAttribute('aria-label');
  pin.append(icon(PIN));
  const remove = document.createElement('button'); remove.className = 'history-action'; remove.dataset.historyDelete = '';
  remove.setAttribute('aria-label', 'Usuń z historii'); remove.title = 'Usuń z historii'; remove.append(icon(REMOVE));
  li.append(copy, pin, remove); return li;
}
function renderHistory(entries) {
  document.querySelectorAll('[data-clipboard-history]').forEach(section => {
    section.hidden = false;
    const list = section.querySelector('[data-history-list]');
    // Keep focus on the same control of the same entry across a re-render (keyboard use).
    const focused = list.contains(document.activeElement) ? document.activeElement : null;
    const keep = focused && { id: focused.closest('li')?.dataset.id, role: ['historyCopy', 'historyPin', 'historyDelete'].find(key => key in focused.dataset) };
    list.replaceChildren(...entries.map(item));
    section.querySelector('[data-history-empty]').hidden = entries.length > 0;
    section.querySelector('[data-history-clear]').hidden = !entries.some(entry => !entry.pinned);
    if (keep?.id) {
      const role = { historyCopy: 'history-copy', historyPin: 'history-pin', historyDelete: 'history-delete' }[keep.role];
      list.querySelector(`li[data-id="${CSS.escape(keep.id)}"] [data-${role}]`)?.focus();
    }
  });
}
async function refreshHistory() { if (invoke) renderHistory(await invoke('clipboard_history')); }
document.querySelectorAll('[data-history-list]').forEach(list => list.addEventListener('click', event => {
  const button = event.target.closest('button'); const li = button?.closest('li');
  if (!invoke || !li || button.disabled) return;
  const id = Number(li.dataset.id);
  let call;
  if ('historyCopy' in button.dataset) call = invoke('clipboard_action', { action: 'history-copy', id });
  else if ('historyPin' in button.dataset) call = invoke('clipboard_action', { action: 'history-pin', id, pinned: button.getAttribute('aria-pressed') !== 'true' });
  else if ('historyDelete' in button.dataset) call = invoke('clipboard_action', { action: 'history-delete', id });
  call?.catch(() => { failed(); refreshHistory().catch(failed); });
}));
document.querySelectorAll('[data-history-clear]').forEach(button => button.addEventListener('click', () => invoke?.('clipboard_action', { action: 'history-clear' }).catch(failed)));
if (invoke) {
  window.__TAURI__?.event?.listen('clipboard-history', event => renderHistory(event.payload));
  window.addEventListener('focus', () => refreshHistory().catch(failed));
  refreshHistory().catch(failed);
}