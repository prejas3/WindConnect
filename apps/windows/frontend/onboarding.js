import { t } from './i18n.js';
// CCS/1 FLOW:pairing-state (the panel's single pairing window) -> onboarding in the main window while no phone is paired;
// step 3 t('main.onboarding-done-title') once a phone was paired here; afterwards the ordinary main window (WC-34).
// SEC:the QR follows the panel's rules (WN-94): canvas only, modules zeroed after drawing, no link in DOM/aria.
const invoke = window.__TAURI__?.core.invoke;
const $ = selector => document.querySelector(selector);
let sawUnpaired = false;
let finished = false;
let lastStep = 0;
let hidHome = false;

function drawQr(canvas, qr) {
  // Integer modules and a four-module quiet zone; black on white in every theme (as in the panel).
  const { width, dark } = qr;
  const scale = Math.max(1, Math.floor(260 / (width + 8)));
  canvas.width = canvas.height = (width + 8) * scale;
  const context = canvas.getContext('2d');
  context.fillStyle = '#fff'; context.fillRect(0, 0, canvas.width, canvas.height);
  context.fillStyle = '#000';
  dark.forEach((value, index) => { if (value) context.fillRect((index % width + 4) * scale, (Math.floor(index / width) + 4) * scale, scale, scale); });
}

function focusFor(step) {
  // The main control of the step gets focus first; the heading when the step has none (screen readers start there).
  if (step === lastStep) return;
  lastStep = step;
  const target = step === 3 ? $('#onboarding-finish') : $('#onboarding-accept:not([hidden])') || $('#onboarding-title');
  queueMicrotask(() => target?.focus({ preventScroll: true }));
}

export function renderOnboarding(state) {
  if (!state || typeof state.phase !== 'string') return;
  // The backend's rule, the one that picks the first window too (WN-113): an owed phone isn't paired.
  const paired = state.paired === true;
  if (!paired) sawUnpaired = true;
  const done = paired && sawUnpaired && !finished && state.connected;
  const active = !finished && (!paired || done);
  $('#onboarding').hidden = !active;
  // Settings that need a phone wait until onboarding is over.
  document.body.classList.toggle('onboarding-active', active);
  // Only undo what onboarding hid (main.js hides #home for the dev gallery too).
  const home = $('#home');
  if (active && !home.hidden) { home.hidden = true; hidHome = true; }
  if (!active && hidHome) { home.hidden = false; hidHome = false; }
  if (!active) { $('#onboarding-qr').width = $('#onboarding-qr').height = 0; if (state.qr?.dark) state.qr.dark.fill(false); return; }

  const phase = state.phase;
  const waiting = phase === 'waiting';
  const sas = phase.startsWith('sas-');
  const asking = phase === 'sas-code';
  const steps = document.querySelectorAll('.onboarding-steps > li');
  const current = done ? 3 : 2;
  steps.forEach((li, i) => { li.classList.toggle('current', i + 1 === current); li.classList.toggle('done', i + 1 < current); });

  $('#onboarding-step-title').textContent = sas ? t('onboarding.compare') : t('main.onboarding-step-title');
  $('#onboarding-hint').textContent = sas
    ? t('onboarding.sas_hint')
    : t('onboarding.qr_hint');
  $('#onboarding-message').textContent = done ? '' : (waiting ? t('onboarding.single_use') : state.message);
  const canvas = $('#onboarding-qr');
  canvas.hidden = !waiting || !state.qr;
  if (!canvas.hidden) drawQr(canvas, state.qr); else canvas.width = canvas.height = 0;
  // Zero the modules once drawn; the IPC JSON and GC copies are beyond reach (WN-94).
  if (state.qr && Array.isArray(state.qr.dark)) state.qr.dark.fill(false);
  const code = $('#onboarding-code');
  code.hidden = !asking || !state.code;
  code.textContent = asking && state.code ? state.code : '';
  $('#onboarding-accept').hidden = !asking;
  $('#onboarding-reject').hidden = !asking;
  // No new code on its own when one expires: the person asks for it (no QR churn in the background).
  $('#onboarding-refresh').hidden = done || asking || ['connecting', 'loading', 'sas-confirming'].includes(phase) || phase === 'connected';
  $('#onboarding-refresh').textContent = sas ? t('onboarding.back_to_qr') : t('main.onboarding-refresh');
  $('#onboarding-sas').hidden = done || !(waiting || phase === 'expired' || phase === 'no-network');

  $('#onboarding-done').hidden = !done;
  $('#onboarding-done-title').textContent = done ? t('onboarding.done_connected', { phone: state.phone || t('common.phone') }) : t('main.onboarding-done-title');
  $('#onboarding-done-text').textContent = done
    ? t('onboarding.done_text')
    : t('main.onboarding-done-text');
  focusFor(current + (asking ? 0.5 : 0));
}

const actions = { 'onboarding-refresh': 'refresh', 'onboarding-sas': 'sas', 'onboarding-accept': 'sas-accept', 'onboarding-reject': 'sas-reject' };
for (const [id, action] of Object.entries(actions)) {
  $(`#${id}`).addEventListener('click', () => invoke?.('pairing_action', { action }).catch(console.error));
}
$('#onboarding-finish').addEventListener('click', async () => {
  finished = true;
  if (invoke) renderOnboarding(await invoke('pairing_snapshot'));
  $('#main')?.focus?.();
});
const autostart = $('#onboarding-autostart');
autostart.addEventListener('click', async () => {
  if (!invoke) return;
  autostart.disabled = true;
  try { autostart.setAttribute('aria-checked', String(await invoke('set_autostart', { enabled: autostart.getAttribute('aria-checked') !== 'true' }))); }
  catch (error) { console.error(error); }
  finally { autostart.disabled = false; }
});

async function start() {
  if (!invoke) return;
  await window.__TAURI__.event.listen('pairing-state', event => renderOnboarding(event.payload));
  try { autostart.setAttribute('aria-checked', String(await invoke('autostart_enabled'))); } catch { autostart.disabled = true; }
  renderOnboarding(await invoke('pairing_snapshot'));
}
start().catch(console.error);
