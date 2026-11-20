// CCS/1 FLOW:WN-43 main-window setting "Notifications from the phone" -> phone_notifications -> pairing-state.phone_notifications.
// The phone's notifications as Windows toasts (notify/1); default on, the phone asks for its own consent (WN-42).
const invoke = window.__TAURI__?.core.invoke;
const toggle = document.querySelector('#phone-notifications');

function render(state) {
  if (!state || typeof state.phone_notifications !== 'boolean') return;
  document.querySelector('#notify-settings').hidden = false;
  toggle.setAttribute('aria-checked', String(state.phone_notifications));
  toggle.disabled = false;
}

toggle?.addEventListener('click', async () => {
  if (!invoke) return;
  const next = toggle.getAttribute('aria-checked') !== 'true';
  toggle.disabled = true;
  toggle.setAttribute('aria-checked', String(next));
  try { await invoke('phone_notifications', { enabled: next }); }
  catch (error) { console.error(error); toggle.setAttribute('aria-checked', String(!next)); }
  finally { toggle.disabled = false; }
});

async function start() {
  if (!invoke || !toggle) return;
  await window.__TAURI__.event.listen('pairing-state', event => render(event.payload));
  render(await invoke('pairing_snapshot'));
}
start().catch(console.error);

// CCS/1 FLOW:WN-45 "Quiet hours" -> notify_quiet_hours(on, from, to) -> pairing-state.quiet_hours. Minutes after local
// midnight, 15-minute steps; through midnight allowed; from == to means none.
const quietToggle = document.querySelector('#quiet-hours');
const times = document.querySelector('#quiet-times');
let quiet = null;
const hhmm = m => `${String(Math.floor(m / 60)).padStart(2, '0')}:${String(m % 60).padStart(2, '0')}`;
function renderQuiet(state) {
  if (!state || !state.quiet_hours || !quietToggle) return;
  quiet = { ...state.quiet_hours };
  quietToggle.disabled = false;
  quietToggle.setAttribute('aria-checked', String(quiet.on));
  times.hidden = !quiet.on;
  for (const which of ['from', 'to']) {
    const spin = times.querySelector(`[data-quiet="${which}"] [role="spinbutton"]`);
    spin.textContent = hhmm(quiet[which]);
    spin.setAttribute('aria-valuenow', String(quiet[which]));
    spin.setAttribute('aria-valuetext', hhmm(quiet[which]));
  }
}
async function sendQuiet(next) {
  const before = quiet;
  renderQuiet({ quiet_hours: next });
  try { await invoke('notify_quiet_hours', next); }
  catch (error) { console.error(error); renderQuiet({ quiet_hours: before }); }
}
const shifted = (which, by) => ({ ...quiet, [which]: ((quiet[which] + by) % 1440 + 1440) % 1440 });
quietToggle?.addEventListener('click', () => { if (quiet && invoke) sendQuiet({ ...quiet, on: !quiet.on }); });
times?.querySelectorAll('.time-stepper').forEach(box => {
  const which = box.dataset.quiet;
  box.querySelectorAll('[data-step]').forEach(b => b.addEventListener('click', () => { if (quiet && invoke) sendQuiet(shifted(which, Number(b.dataset.step))); }));
  box.querySelector('[role="spinbutton"]').addEventListener('keydown', event => {
    const by = { ArrowUp: 15, ArrowRight: 15, ArrowDown: -15, ArrowLeft: -15, PageUp: 60, PageDown: -60 }[event.key];
    if (!by || !quiet || !invoke) return;
    event.preventDefault();
    sendQuiet(shifted(which, by));
  });
});
async function startQuiet() {
  if (!invoke || !quietToggle) return;
  await window.__TAURI__.event.listen('pairing-state', event => renderQuiet(event.payload));
  renderQuiet(await invoke('pairing_snapshot'));
}
startQuiet().catch(console.error);
