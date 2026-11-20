import { t } from './i18n.js';
// CCS/1 FLOW:WN-92 main-window setting "Relay away from home" -> set_relay(url | null) -> keepers in relay.rs. Off by default.
const invoke = window.__TAURI__?.core.invoke;
const section = document.querySelector('#relay-settings');
const toggle = document.querySelector('#relay-enabled');
const field = document.querySelector('#relay-url');
const error = document.querySelector('#relay-error');

function show(url) {
  toggle.setAttribute('aria-checked', String(url != null));
  if (url != null) field.value = url;
  toggle.disabled = false;
}

function fail(message) {
  error.textContent = message;
  field.setAttribute('aria-invalid', message ? 'true' : 'false');
}

async function apply(url) {
  toggle.disabled = true;
  try { show(await invoke('set_relay', { url })); fail(''); }
  catch (e) {
    // Only a refused URL is "Invalid address"; still starting or a failed save say what they are (WN-139).
    const text = String(e);
    fail(/^(WindConnect jeszcze|nie udało się zapisać|WindConnect is still|couldn't save)/.test(text) ? text : t('relay.invalid', { error: text }));
    show(await invoke('relay_setting').catch(() => null));
  }
}

toggle?.addEventListener('click', () => {
  const on = toggle.getAttribute('aria-checked') !== 'true';
  if (on && !field.value.trim()) { fail(t('relay.address_needed')); field.focus(); return; }
  apply(on ? field.value.trim() : null);
});
field?.addEventListener('input', () => fail(''));
field?.addEventListener('change', () => {
  if (toggle.getAttribute('aria-checked') === 'true' && field.value.trim()) apply(field.value.trim());
});

async function start() {
  if (!invoke || !section) return;
  section.hidden = false;
  // WN-139: until the core has read the saved setting, relay_setting fails; the switch stays disabled meanwhile.
  for (;;) {
    try { show(await invoke('relay_setting')); return; } catch { await new Promise(r => setTimeout(r, 300)); }
  }
}
start().catch(console.error);
