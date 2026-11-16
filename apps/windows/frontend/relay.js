// CCS/1 FLOW:WN-92 main-window setting "Przekaźnik poza domem" -> set_relay(url | null) -> keepers in relay.rs. Off by default.
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
  catch (e) { fail('Nieprawidłowy adres: ' + e); show(await invoke('relay_setting')); }
}

toggle?.addEventListener('click', () => {
  const on = toggle.getAttribute('aria-checked') !== 'true';
  if (on && !field.value.trim()) { fail('Podaj adres przekaźnika'); field.focus(); return; }
  apply(on ? field.value.trim() : null);
});
field?.addEventListener('input', () => fail(''));
field?.addEventListener('change', () => {
  if (toggle.getAttribute('aria-checked') === 'true' && field.value.trim()) apply(field.value.trim());
});

async function start() {
  if (!invoke || !section) return;
  section.hidden = false;
  show(await invoke('relay_setting'));
}
start().catch(console.error);
