// CCS/1 FLOW:WN-58 gallery-state (pushed on every change, never polled) -> "Ostatnie zdjęcia" tiles; press -> gallery_prefetch;
// move past DRAG_PX -> gallery_drag (native drag of the original); click/Enter -> gallery_open; gallery-fetch -> tile progress.
// SEC:names go in as text only; a thumbnail only as a data: URL of a JPEG (the core checked its magic bytes).
const invoke = window.__TAURI__?.core.invoke;
const listen = window.__TAURI__?.event.listen;
const DRAG_PX = 6;
const THUMB = /^data:image\/jpeg;base64,[A-Za-z0-9+/=]+$/;
const section = document.querySelector('#gallery');
const list = document.querySelector('#gallery-list');
const status = document.querySelector('#gallery-status');
const tiles = new Map();
let statusTimer;

function say(text) {
  clearTimeout(statusTimer);
  status.textContent = text;
  if (text) statusTimer = setTimeout(() => { status.textContent = ''; }, 6000);
}
const failed = value => say(String(value || 'Nie udało się.'));

function label(photo) {
  const kind = photo.screenshot ? 'Zrzut ekranu' : 'Zdjęcie';
  const when = new Date(photo.date_ms).toLocaleString('pl-PL', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' });
  return `${kind} ${photo.name}, ${when}. Przeciągnij na pulpit albo naciśnij Enter, aby otworzyć.`;
}

function tile(photo) {
  let entry = tiles.get(photo.id);
  if (!entry) {
    const item = document.createElement('li');
    const button = document.createElement('button');
    button.className = 'photo';
    button.dataset.id = String(photo.id);
    const img = document.createElement('img');
    img.alt = ''; img.draggable = false; img.decoding = 'async';
    button.append(img);
    item.append(button);
    entry = { item, button, img, thumb: null };
    wire(entry, photo.id);
    tiles.set(photo.id, entry);
  }
  entry.button.setAttribute('aria-label', label(photo));
  entry.button.title = photo.name;
  const thumb = typeof photo.thumb === 'string' && THUMB.test(photo.thumb) ? photo.thumb : null;
  if (thumb !== entry.thumb) {
    entry.thumb = thumb;
    if (thumb) entry.img.src = thumb; else entry.img.removeAttribute('src');
    entry.button.classList.toggle('loading', !thumb);
  }
  return entry.item;
}

// One gesture per press: a move past DRAG_PX drags (once), otherwise the release is a click that opens.
function wire(entry, id) {
  let press = null;
  const button = entry.button;
  button.addEventListener('pointerdown', event => {
    if (event.button !== 0) return;
    press = { x: event.clientX, y: event.clientY, dragging: false };
    invoke?.('gallery_prefetch', { id }).catch(() => {});
  });
  button.addEventListener('pointermove', async event => {
    if (!press || press.dragging || !(event.buttons & 1)) return;
    if (Math.hypot(event.clientX - press.x, event.clientY - press.y) < DRAG_PX) return;
    press.dragging = true;
    try {
      const result = await invoke?.('gallery_drag', { id });
      if (result === 'released') say('Zdjęcie jest gotowe. Przeciągnij je jeszcze raz.');
    } catch (value) { failed(value); }
  });
  // A drag ends with no click of its own; a plain press does.
  button.addEventListener('click', async event => {
    const dragged = press?.dragging;
    press = null;
    if (dragged) { event.preventDefault(); return; }
    try { await invoke?.('gallery_open', { id }); } catch (value) { failed(value); }
  });
  button.addEventListener('pointercancel', () => { press = null; });
  button.addEventListener('dragstart', event => event.preventDefault());
}

export function renderGallery(state) {
  const photos = state?.available && Array.isArray(state.photos) ? state.photos : [];
  section.hidden = photos.length === 0;
  const shown = new Set(photos.map(p => p.id));
  for (const id of [...tiles.keys()]) if (!shown.has(id)) tiles.delete(id);
  list.replaceChildren(...photos.map(tile));
}

function renderFetch({ id, received, size }) {
  const entry = tiles.get(id);
  if (!entry) return;
  const done = !size || received >= size;
  entry.button.classList.toggle('fetching', !done);
  entry.button.style.setProperty('--progress', done ? '0%' : `${Math.round(100 * received / size)}%`);
}

async function start() {
  if (!invoke || !listen) return;
  await listen('gallery-state', event => renderGallery(event.payload));
  await listen('gallery-fetch', event => renderFetch(event.payload));
  await listen('panel-opened', async () => renderGallery(await invoke('gallery_state')));
  renderGallery(await invoke('gallery_state'));
}
start().catch(() => {});
