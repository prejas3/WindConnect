const invoke = window.__TAURI__?.core.invoke;
const systemTheme = matchMedia('(prefers-color-scheme: dark)');
let themeChoice = 'system';
try { themeChoice = localStorage.getItem('windconnect-theme') || 'system'; } catch { /* Storage may be disabled by host policy. */ }
if (!['light', 'dark', 'system'].includes(themeChoice)) themeChoice = 'system';

function applyTheme() {
  document.documentElement.dataset.theme = themeChoice === 'system' ? (systemTheme.matches ? 'dark' : 'light') : themeChoice;
  document.querySelectorAll('[data-theme-choice]').forEach(button => button.setAttribute('aria-pressed', String(button.dataset.themeChoice === themeChoice)));
  if (invoke) invoke('apply_window_theme', { theme: document.documentElement.dataset.theme })
    .then(enabled => { document.body.dataset.mica = String(enabled); })
    .catch(error => { document.body.dataset.mica = 'false'; console.error(error); });
}
applyTheme();
systemTheme.addEventListener('change', applyTheme);
document.querySelectorAll('[data-theme-choice]').forEach(button => button.addEventListener('click', () => {
  themeChoice = button.dataset.themeChoice;
  try { localStorage.setItem('windconnect-theme', themeChoice); } catch { /* In-memory preference still works. */ }
  applyTheme();
}));

let toastTimer;
export function toast(message) {
  const element = document.querySelector('#toast');
  clearTimeout(toastTimer);
  element.textContent = message;
  element.hidden = false;
  toastTimer = setTimeout(() => { element.hidden = true; }, 3500);
}

async function windowAction(action) {
  if (!invoke) return;
  try { await invoke('window_action', { action }); }
  catch (error) { console.error(error); toast('Nie udało się zmienić okna.'); }
}
document.querySelectorAll('[data-window]').forEach(button => button.addEventListener('click', () => windowAction(button.dataset.window)));
document.querySelector('#titlebar').addEventListener('mousedown', event => {
  if (event.button === 0 && !event.target.closest('button')) windowAction(event.detail === 2 ? 'maximize' : 'drag');
});

async function start() {
  const status = document.querySelector('#status');
  try {
    // CCS/1 RULE:release native command always denies the gallery, including URL overrides.
    const localPreview = !invoke && ['localhost', '127.0.0.1'].includes(location.hostname);
    const devMode = invoke ? await invoke('ui_dev_mode') : localPreview;
    if (invoke) {
      const version = await invoke('core_version');
      status.textContent = `wind-core ${version} · gotowy`;
    } else {
      status.textContent = 'Podgląd interfejsu · bez połączenia z wind-core';
    }
    if (devMode) {
      const { mountGallery } = await import('./gallery.js');
      mountGallery(toast);
      const toggle = document.querySelector('#gallery-toggle');
      const gallery = document.querySelector('#gallery');
      const home = document.querySelector('#home');
      toggle.hidden = false;
      toggle.addEventListener('click', () => {
        gallery.hidden = !gallery.hidden;
        home.hidden = !gallery.hidden;
        toggle.setAttribute('aria-pressed', String(!gallery.hidden));
      });
      if (new URLSearchParams(location.search).has('gallery')) toggle.click();
    }
  } catch (error) {
    status.textContent = 'Nie udało się uruchomić wind-core.';
    console.error(error);
  }
}
start();
