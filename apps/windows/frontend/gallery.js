import { t, applyI18n } from './i18n.js';
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
    <div class="gallery-intro"><h2 id="gallery-title" data-i18n="gallery.gallery-title" data-i18n-pos="start"><span class="dev-badge" data-i18n="gallery.dev"></span></h2><p data-i18n="gallery.stany_i_zachowanie_elementow"></p></div>
    <div class="gallery-grid">
      <section class="surface sample"><h3 data-i18n="gallery.przyciski"></h3><div class="button-row">
        <button class="button primary" data-feedback="gallery.przyklad_przycisk_glowny" data-i18n="gallery.wyslij">${icon('send')}</button>
        <button class="button" data-feedback="gallery.przyklad_przycisk_drugorzedny" data-i18n="gallery.dodaj_urzadzenie"></button>
        <button class="button quiet" data-feedback="gallery.przyklad_przycisk_tekstowy" data-i18n="gallery.wiecej"></button>
        <button class="button danger" id="open-dialog" data-i18n="gallery.open-dialog"></button><button class="button" disabled data-i18n="gallery.niedostepny"></button>
      </div><div class="tiles">
        <button class="tile" data-feedback="gallery.przyklad_wysylanie_pliku" data-i18n="gallery.wyslij">${icon('send')}</button>
        <button class="tile" id="clipboard-tile" aria-pressed="true" data-i18n="gallery.clipboard-tile">${icon('clipboard')}</button>
        <button class="tile" data-feedback="gallery.przyklad_sygnal_telefonu" data-i18n="gallery.dzwon">${icon('bell')}</button>
        <button class="tile" disabled data-i18n="gallery.touchpad">${icon('touch')}</button>
      </div><p data-i18n="gallery.wlasne_ikony_z_makiety"></p></section>
      <section class="surface sample"><h3 data-i18n="gallery.przelaczniki_i_wybor"></h3>
        <div class="control-row"><div id="clipboard-label" data-i18n="gallery.clipboard-label" data-i18n-pos="start"><small data-i18n="gallery.tylko_przyklad_stanu_kontrolki"></small></div><button class="switch" role="switch" aria-checked="true" aria-labelledby="clipboard-label"></button></div>
        <div class="control-row"><span id="notifications-label" data-i18n="gallery.notifications-label"></span><button class="switch" role="switch" aria-checked="false" aria-labelledby="notifications-label"></button></div>
        <div class="control-row"><span id="disabled-label" data-i18n="gallery.disabled-label"></span><button class="switch" role="switch" aria-checked="false" aria-labelledby="disabled-label" disabled></button></div>
        <label class="check-label" data-i18n="gallery.uruchamiaj_razem_z_systemem"><input class="checkbox" type="checkbox" checked></label>
      </section>
      <section class="surface sample"><h3 data-i18n="gallery.pola_i_walidacja"></h3><form id="sample-form" novalidate>
        <label class="field" for="device-name" data-i18n="gallery.nazwa_urzadzenia" data-i18n-pos="start"><input id="device-name" data-i18n-placeholder="gallery.device-name_placeholder" maxlength="64" autocomplete="off" aria-describedby="name-hint name-error"><span class="field-hint" id="name-hint" data-i18n="gallery.name-hint"></span><span id="name-error" class="field-hint field-error" hidden data-i18n="gallery.name-error"></span></label>
        <label class="field" for="device-id" data-i18n="gallery.identyfikator_tylko_odczyt" data-i18n-pos="start"><input id="device-id" data-i18n-value="gallery.device-id_value" readonly></label>
        <label class="field" for="disabled-field" data-i18n="gallery.pole_niedostepne" data-i18n-pos="start"><input id="disabled-field" data-i18n-value="gallery.disabled-field_value" disabled></label>
        <label class="field" for="sample-note" data-i18n="gallery.wiadomosc" data-i18n-pos="start"><textarea id="sample-note" data-i18n-placeholder="gallery.sample-note_placeholder" maxlength="500"></textarea></label>
        <button class="button primary" type="submit" data-i18n="gallery.sprawdz_nazwe"></button>
      </form></section>
      <section class="surface sample"><h3 data-i18n="gallery.lista_urzadzen"></h3>
        <div class="device-list" role="listbox" data-i18n-aria-label="gallery.przykladowe_urzadzenia">
          <div class="device-option" role="option" tabindex="0" aria-selected="true">${icon('phone')}<span><strong data-i18n="gallery.galaxy_s25_ultra"></strong><small data-i18n="gallery.przyklad_telefon"></small></span><span class="selection">${icon('check')}</span></div>
          <div class="device-option" role="option" tabindex="-1" aria-selected="false">${icon('pc')}<span><strong data-i18n="gallery.komputer_w_domu"></strong><small data-i18n="gallery.przyklad_windows_11"></small></span><span class="selection">${icon('check')}</span></div>
          <div class="device-option" role="option" tabindex="-1" aria-selected="false" aria-disabled="true">${icon('pc')}<span><strong data-i18n="gallery.niedostepne_urzadzenie"></strong><small data-i18n="gallery.opcja_wylaczona"></small></span></div>
        </div><p data-i18n="gallery.strzalki_home_end_wybieraja"></p>
      </section>
      <section class="surface sample"><h3 data-i18n="gallery.status_i_postep"></h3><div class="button-row"><span class="status-pill success" data-i18n="gallery.polaczono"></span><span class="status-pill warning" data-i18n="gallery.szukam_telefonu"></span><span class="status-pill offline" data-i18n="gallery.poza_zasiegiem"></span></div>
        <div id="sample-progress" class="progress" role="progressbar" data-i18n-aria-label="gallery.sample-progress_label" aria-valuemin="0" aria-valuemax="100" aria-valuenow="64"><span></span></div>
        <p id="progress-label" data-i18n="gallery.progress-label"></p><div class="button-row"><button class="button small" id="complete-progress" data-i18n="gallery.complete-progress"></button><button class="button quiet small" id="reset-progress" data-i18n="gallery.reset-progress"></button></div>
      </section>
      <section class="surface sample"><h3 data-i18n="gallery.ruch_i_typografia"></h3><div class="motion-preview" id="motion-preview"><strong data-i18n="gallery.galaxy_s25_ultra"></strong><p data-i18n="gallery.zazolc_gesla_jazn_0123456789"></p></div><button class="button" id="replay-motion" data-i18n="gallery.replay-motion"></button><p data-i18n="gallery.dm_sans_wejscie_160"></p></section>
      <section class="surface sample wide"><h3 data-i18n="gallery.tokeny_kolorow"></h3><div class="token-grid"><div class="swatch surface-token"><i></i><span data-i18n="gallery.powierzchnia"></span></div><div class="swatch tile-token"><i></i><span data-i18n="gallery.kafelki"></span></div><div class="swatch accent-token"><i></i><span data-i18n="gallery.akcent"></span></div><div class="swatch text-token"><i></i><span data-i18n="gallery.tekst"></span></div></div><p data-i18n="gallery.paleta_morska_i_grafitowa"></p></section>
    </div>`;

  applyI18n(gallery);
  gallery.querySelectorAll('[data-feedback]').forEach(button => button.addEventListener('click', () => toast(t(button.dataset.feedback))));
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
    if (!validate()) input.focus(); else toast(t('gallery.name_ok'));
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
    progress.dataset.complete = 'true'; progress.setAttribute('aria-valuenow', '100'); label.textContent = t('gallery.progress_done');
  });
  gallery.querySelector('#reset-progress').addEventListener('click', () => {
    progress.dataset.complete = 'false'; progress.setAttribute('aria-valuenow', '64'); label.textContent = t('gallery.progress');
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
  overlay.querySelector('[data-confirm]').addEventListener('click', () => { close(); toast(t('gallery.confirmed')); });
  overlay.addEventListener('click', event => { if (event.target === overlay) close(); });
  overlay.addEventListener('keydown', event => {
    if (event.key === 'Escape') { event.preventDefault(); close(); }
    if (event.key !== 'Tab') return;
    const buttons = [...dialog.querySelectorAll('button')];
    if (event.shiftKey && document.activeElement === buttons[0]) { event.preventDefault(); buttons.at(-1).focus(); }
    else if (!event.shiftKey && document.activeElement === buttons.at(-1)) { event.preventDefault(); buttons[0].focus(); }
  });
}
