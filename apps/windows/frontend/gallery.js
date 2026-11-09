// CCS/1 IO:dev-only examples | SEC:no transport, trust, filesystem or device actions.
const paths = {
  send: '<path d="M12 16V4M7 9l5-5 5 5M5 17v3h14v-3"/>',
  clipboard: '<rect x="6" y="5" width="12" height="16" rx="2"/><rect x="9" y="3" width="6" height="4" rx="1"/><path d="M9 12h6M9 16h4"/>',
  bell: '<path d="M6 10a6 6 0 0 1 12 0v4l2 3H4l2-3zM10 21h4"/>',
  touch: '<rect x="4" y="3" width="16" height="18" rx="3"/><path d="M4 15h16M12 15v6M9 8l2 2 4-4"/>',
  phone: '<rect x="6" y="2" width="12" height="20" rx="3"/><path d="M10 5h4M11 19h2"/>',
  pc: '<rect x="3" y="4" width="18" height="12" rx="2"/><path d="M8 21h8M12 16v5"/>',
  check: '<path d="m5 12 4 4L19 6"/>'
};
const icon = name => `<svg viewBox="0 0 24 24" aria-hidden="true">${paths[name]}</svg>`;

export function mountGallery(toast) {
  const gallery = document.querySelector('#gallery');
  gallery.innerHTML = `
    <div class="gallery-intro"><h2 id="gallery-title">Galeria kontrolek <span class="dev-badge">DEV</span></h2><p>Stany i zachowanie elementów interfejsu. Wszystkie dane są przykładowe.</p></div>
    <div class="gallery-grid">
      <section class="surface sample"><h3>Przyciski</h3><div class="button-row">
        <button class="button primary" data-feedback="Przykład: przycisk główny">${icon('send')}Wyślij</button>
        <button class="button" data-feedback="Przykład: przycisk drugorzędny">Dodaj urządzenie</button>
        <button class="button quiet" data-feedback="Przykład: przycisk tekstowy">Więcej</button>
        <button class="button danger" id="open-dialog">Rozłącz</button><button class="button" disabled>Niedostępny</button>
      </div><div class="tiles">
        <button class="tile" data-feedback="Przykład: wysyłanie pliku">${icon('send')}Wyślij</button>
        <button class="tile" id="clipboard-tile" aria-pressed="true">${icon('clipboard')}Schowek</button>
        <button class="tile" data-feedback="Przykład: sygnał telefonu">${icon('bell')}Dzwoń</button>
        <button class="tile" disabled>${icon('touch')}Touchpad</button>
      </div><p>Własne ikony z makiety WN-20. Tab / Enter / Spacja.</p></section>
      <section class="surface sample"><h3>Przełączniki i wybór</h3>
        <div class="control-row"><div id="clipboard-label">Automatyczny schowek<small>Tylko przykład stanu kontrolki</small></div><button class="switch" role="switch" aria-checked="true" aria-labelledby="clipboard-label"></button></div>
        <div class="control-row"><span id="notifications-label">Powiadomienia</span><button class="switch" role="switch" aria-checked="false" aria-labelledby="notifications-label"></button></div>
        <div class="control-row"><span id="disabled-label">Niedostępny</span><button class="switch" role="switch" aria-checked="false" aria-labelledby="disabled-label" disabled></button></div>
        <label class="check-label"><input class="checkbox" type="checkbox" checked>Uruchamiaj razem z systemem (przykład)</label>
      </section>
      <section class="surface sample"><h3>Pola i walidacja</h3><form id="sample-form" novalidate>
        <label class="field" for="device-name">Nazwa urządzenia<input id="device-name" placeholder="Np. Komputer w domu" maxlength="64" autocomplete="off" aria-describedby="name-hint name-error"><span class="field-hint" id="name-hint">1–64 znaki. Dane pozostają w tej galerii.</span><span id="name-error" class="field-hint field-error" hidden>Wpisz nazwę urządzenia.</span></label>
        <label class="field" for="device-id">Identyfikator (tylko odczyt)<input id="device-id" value="PRZYKŁAD-7F2A" readonly></label>
        <label class="field" for="disabled-field">Pole niedostępne<input id="disabled-field" value="Brak uprawnienia" disabled></label>
        <label class="field" for="sample-note">Wiadomość<textarea id="sample-note" placeholder="Zażółć gęślą jaźń…" maxlength="500"></textarea></label>
        <button class="button primary" type="submit">Sprawdź nazwę</button>
      </form></section>
      <section class="surface sample"><h3>Lista urządzeń</h3>
        <div class="device-list" role="listbox" aria-label="Przykładowe urządzenia">
          <div class="device-option" role="option" tabindex="0" aria-selected="true">${icon('phone')}<span><strong>Galaxy S25 Ultra</strong><small>Przykład · telefon</small></span><span class="selection">${icon('check')}</span></div>
          <div class="device-option" role="option" tabindex="-1" aria-selected="false">${icon('pc')}<span><strong>Komputer w domu</strong><small>Przykład · Windows 11</small></span><span class="selection">${icon('check')}</span></div>
          <div class="device-option" role="option" tabindex="-1" aria-selected="false" aria-disabled="true">${icon('pc')}<span><strong>Niedostępne urządzenie</strong><small>Opcja wyłączona</small></span></div>
        </div><p>Strzałki ↑ ↓, Home / End wybierają i przenoszą fokus. Wyłączona opcja jest pomijana.</p>
      </section>
      <section class="surface sample"><h3>Status i postęp</h3><div class="button-row"><span class="status-pill success">Połączono</span><span class="status-pill warning">Szukam telefonu</span><span class="status-pill offline">Poza zasięgiem</span></div>
        <div id="sample-progress" class="progress" role="progressbar" aria-label="Przykładowy transfer" aria-valuemin="0" aria-valuemax="100" aria-valuenow="64"><span></span></div>
        <p id="progress-label">Przykład · 154 z 240 MB · 64%</p><div class="button-row"><button class="button small" id="complete-progress">Zakończ przykład</button><button class="button quiet small" id="reset-progress">Resetuj</button></div>
      </section>
      <section class="surface sample"><h3>Ruch i typografia</h3><div class="motion-preview" id="motion-preview"><strong>Galaxy S25 Ultra</strong><p>Zażółć gęślą jaźń. 0123456789 · 82%</p></div><button class="button" id="replay-motion">Odtwórz wejście</button><p>DM Sans · wejście 160 ms / 8 px · naciśnięcie 90 ms / 0,98. Ustawienie ograniczonego ruchu systemu wyłącza animacje.</p></section>
      <section class="surface sample wide"><h3>Tokeny kolorów</h3><div class="token-grid"><div class="swatch surface-token"><i></i><span>Powierzchnia</span></div><div class="swatch tile-token"><i></i><span>Kafelki</span></div><div class="swatch accent-token"><i></i><span>Akcent</span></div><div class="swatch text-token"><i></i><span>Tekst</span></div></div><p>Paleta morska i grafitowa z WN-20. Oba motywy można przełączyć u góry.</p></section>
    </div>`;

  gallery.querySelectorAll('[data-feedback]').forEach(button => button.addEventListener('click', () => toast(button.dataset.feedback)));
  gallery.querySelectorAll('[role="switch"]').forEach(button => button.addEventListener('click', () => {
    button.setAttribute('aria-checked', String(button.getAttribute('aria-checked') !== 'true'));
  }));
  gallery.querySelector('#clipboard-tile').addEventListener('click', event => {
    const button = event.currentTarget;
    button.setAttribute('aria-pressed', String(button.getAttribute('aria-pressed') !== 'true'));
  });
  const input = gallery.querySelector('#device-name');
  const error = gallery.querySelector('#name-error');
  function validate() {
    const valid = input.value.trim().length > 0;
    input.setAttribute('aria-invalid', String(!valid));
    error.hidden = valid;
    return valid;
  }
  input.addEventListener('input', () => { if (input.hasAttribute('aria-invalid')) validate(); });
  gallery.querySelector('#sample-form').addEventListener('submit', event => {
    event.preventDefault();
    if (!validate()) input.focus(); else toast('Przykład: nazwa jest poprawna.');
  });
  const list = gallery.querySelector('[role="listbox"]');
  const options = [...list.querySelectorAll('[role="option"]:not([aria-disabled="true"])')];
  function select(option, focus = true) {
    options.forEach(item => { item.setAttribute('aria-selected', String(item === option)); item.tabIndex = item === option ? 0 : -1; });
    if (focus) option.focus();
  }
  options.forEach(option => option.addEventListener('click', () => select(option)));
  list.addEventListener('keydown', event => {
    const index = options.indexOf(document.activeElement);
    if (index < 0) return;
    const targets = { ArrowDown: (index + 1) % options.length, ArrowUp: (index - 1 + options.length) % options.length, Home: 0, End: options.length - 1, Enter: index, ' ': index };
    if (!(event.key in targets)) return;
    event.preventDefault(); select(options[targets[event.key]]);
  });
  const progress = gallery.querySelector('#sample-progress');
  const label = gallery.querySelector('#progress-label');
  gallery.querySelector('#complete-progress').addEventListener('click', () => {
    progress.dataset.complete = 'true'; progress.setAttribute('aria-valuenow', '100'); label.textContent = 'Przykład zakończony · 240 MB · 100%';
  });
  gallery.querySelector('#reset-progress').addEventListener('click', () => {
    progress.dataset.complete = 'false'; progress.setAttribute('aria-valuenow', '64'); label.textContent = 'Przykład · 154 z 240 MB · 64%';
  });
  gallery.querySelector('#replay-motion').addEventListener('click', () => {
    const preview = gallery.querySelector('#motion-preview'); preview.classList.remove('playing'); void preview.offsetWidth; preview.classList.add('playing');
  });

  const overlay = document.querySelector('#overlay');
  const dialog = overlay.querySelector('[role="dialog"]');
  const main = document.querySelector('#main');
  const titlebar = document.querySelector('#titlebar');
  let previousFocus;
  function close() {
    overlay.hidden = true; main.inert = false; titlebar.inert = false;
    document.body.classList.remove('modal-open'); previousFocus?.focus();
  }
  gallery.querySelector('#open-dialog').addEventListener('click', () => {
    previousFocus = document.activeElement; overlay.hidden = false;
    main.inert = true; titlebar.inert = true; document.body.classList.add('modal-open');
    dialog.querySelector('[data-dismiss]').focus();
  });
  overlay.querySelector('[data-dismiss]').addEventListener('click', close);
  overlay.querySelector('[data-confirm]').addEventListener('click', () => { close(); toast('Przykład: potwierdzono.'); });
  overlay.addEventListener('click', event => { if (event.target === overlay) close(); });
  overlay.addEventListener('keydown', event => {
    if (event.key === 'Escape') { event.preventDefault(); close(); }
    if (event.key !== 'Tab') return;
    const buttons = [...dialog.querySelectorAll('button')];
    if (event.shiftKey && document.activeElement === buttons[0]) { event.preventDefault(); buttons.at(-1).focus(); }
    else if (!event.shiftKey && document.activeElement === buttons.at(-1)) { event.preventDefault(); buttons[0].focus(); }
  });
}
