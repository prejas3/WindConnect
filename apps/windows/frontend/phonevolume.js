// CCS/1 FLOW:WN-48 pairing-state.phone_volume -> "Głośność telefonu" in the panel; drag/keys/mute -> phone_volume(level?, muted?).
// RULE:no bounce-back: while dragging, and for HOLD_MS after the last local change, a pushed level (often the echo of our own
// Set) doesn't move the slider; after that the latest pushed level wins. Sends at most every 100 ms (the latest wins).
const invoke = window.__TAURI__?.core.invoke;
export const HOLD_MS = 500;
const section = document.querySelector('#phone-volume');
const slider = document.querySelector('#phone-volume-slider');
const mute = document.querySelector('#phone-mute');
let shown = null;        // what the slider shows: { level, muted }
let pushed = null;       // the latest from the phone
let localAt = -Infinity; // the last local change
let dragging = false;
let controllable = false;
let pending = null, timer = null, lastSent = -Infinity;

function paint() {
  if (!shown) return;
  slider.style.setProperty('--level', `${shown.level}%`);
  slider.setAttribute('aria-valuenow', String(shown.level));
  slider.setAttribute('aria-valuetext', shown.muted ? `${shown.level}%, wyciszony` : `${shown.level}%`);
  document.querySelector('#phone-volume-value').textContent = shown.muted ? 'Wyciszony' : `${shown.level}%`;
  mute.setAttribute('aria-pressed', String(shown.muted));
  mute.setAttribute('aria-label', shown.muted ? 'Wyłącz wyciszenie telefonu' : 'Wycisz telefon');
  section.classList.toggle('muted', shown.muted);
}

function send(change) {
  pending = { ...(pending || {}), ...change };
  const flush = () => {
    timer = null;
    const body = pending; pending = null; lastSent = performance.now();
    invoke?.('phone_volume', { level: body.level ?? null, muted: body.muted ?? null }).catch(e => console.error(String(e).slice(0, 80)));
  };
  const wait = Math.max(0, 100 - (performance.now() - lastSent));
  if (!timer) timer = setTimeout(flush, wait);
}

function local(change) {
  if (!controllable || !shown) return;
  localAt = performance.now();
  shown = { ...shown, ...change };
  paint();
  send(change);
}

export function renderPhoneVolume(volume) {
  section.hidden = !volume;
  if (!volume) { shown = pushed = null; return; }
  controllable = !!volume.controllable;
  slider.setAttribute('aria-disabled', String(!controllable));
  mute.disabled = !controllable;
  document.querySelector('#phone-volume-note').hidden = controllable;
  pushed = { level: volume.level, muted: volume.muted };
  settle();
}

// The latest pushed level wins once we're not dragging and HOLD_MS passed since the last local change.
function settle() {
  if (!pushed) return;
  const since = performance.now() - localAt;
  if (!shown || (!dragging && since >= HOLD_MS)) { shown = { ...pushed }; paint(); return; }
  setTimeout(settle, Math.max(16, HOLD_MS - since));
}

const levelAt = x => { const box = slider.getBoundingClientRect(); return Math.round(Math.min(1, Math.max(0, (x - box.left) / box.width)) * 100); };
slider.addEventListener('pointerdown', event => {
  if (!controllable) return;
  dragging = true; slider.setPointerCapture(event.pointerId); local({ level: levelAt(event.clientX) });
});
slider.addEventListener('pointermove', event => { if (dragging) local({ level: levelAt(event.clientX) }); });
const stop = () => { if (dragging) { dragging = false; localAt = performance.now(); settle(); } };
slider.addEventListener('pointerup', stop);
slider.addEventListener('pointercancel', stop);
slider.addEventListener('keydown', event => {
  if (!controllable || !shown) return;
  const by = { ArrowRight: 5, ArrowUp: 5, ArrowLeft: -5, ArrowDown: -5, PageUp: 10, PageDown: -10 }[event.key];
  const to = event.key === 'Home' ? 0 : event.key === 'End' ? 100 : by === undefined ? null : Math.min(100, Math.max(0, shown.level + by));
  if (to === null) return;
  event.preventDefault();
  local({ level: to });
  settle();
});
mute.addEventListener('click', () => { if (shown) { local({ muted: !shown.muted }); settle(); } });
