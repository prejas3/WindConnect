import { t, plural, formatNumber } from './i18n.js';
// CCS/1 PERF:keyed nodes retain focus at <=10 dirty updates/s; no polling or per-row timers.
export function bytes(value) {
  if (!Number.isFinite(value) || value < 0) return '—';
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB']; let i = 0;
  while (value >= 1024 && i < units.length - 1) { value /= 1024; i++; }
  const digits = i && value < 10 ? 1 : 0;
  return `${formatNumber(value, { minimumFractionDigits: digits, maximumFractionDigits: digits })} ${units[i]}`;
}
export function createTransfers(document, action, clock = () => performance.now()) {
  const nodes = new Map(); let announcedAt = -Infinity, announcedState = '', expanded = false, last = { rows: [] };
  const find = id => document.querySelector(`#transfers${id}`);
  find('-clear').addEventListener('click', () => action('clear'));
  // WN-129: a long history no longer fills the panel: running ones and the newest few, the rest behind "Show all".
  const more = document.createElement('button'); more.className = 'button quiet small'; more.id = 'transfers-more'; more.hidden = true;
  more.setAttribute('aria-controls', 'transfers-list'); more.setAttribute('aria-expanded', 'false');
  more.addEventListener('click', () => { expanded = !expanded; render(last); });
  find('-list').after(more);
  function make(row) {
    const li = document.createElement('li'); li.dataset.transfer = row.key || `${row.direction}:${row.id}`;
    const text = document.createElement('div'); text.className = 'transfer-copy';
    const name = document.createElement('span'); name.className = 'name';
    const detail = document.createElement('small');
    const bar = document.createElement('div'); bar.className = 'progress'; bar.setAttribute('role', 'progressbar');
    bar.setAttribute('aria-valuemin', '0'); bar.setAttribute('aria-valuemax', '100'); bar.append(document.createElement('span'));
    const buttons = document.createElement('div'); buttons.className = 'transfer-actions';
    const controls = {};
    for (const [kind, label] of [['cancel', t('common.cancel')], ['retry', t('common.retry')], ['open', t('transfers.open_folder')]]) {
      const button = document.createElement('button'); button.className = 'button quiet small'; button.textContent = label;
      if (kind === 'open') { button.className = 'button quiet small icon'; button.innerHTML = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 7h6l2 2h10v10H3z"/></svg>'; button.title = label; button.dataset.label = label; }
      button.addEventListener('click', () => action(kind, controls.id)); buttons.append(button); controls[kind] = button;
    }
    text.append(name, detail, bar); li.append(text, buttons);
    return { li, name, detail, bar, controls };
  }
  return render;
  function render(state) {
    last = state;
    const all = state.rows || [], active = all.filter(r => ['queued', 'running', 'waiting'].includes(r.state));
    const finished = all.filter(r => !active.includes(r));
    // WN-156: nothing left to collapse (cleared, or few finished): back to the short list, no stray "Show less".
    if (finished.length <= TRANSFERS_SHOWN) expanded = false;
    const rows = expanded ? all : all.filter(r => active.includes(r) || finished.indexOf(r) < TRANSFERS_SHOWN);
    more.hidden = all.length <= rows.length && !expanded;
    more.setAttribute('aria-expanded', String(expanded));
    more.textContent = expanded ? t('transfers.show_less') : t('transfers.show_all', { count: all.length });
    find('').hidden = !rows.length;
    find('-title').textContent = t('transfers.title');
    const summary = active.length ? t('transfers.active', { count: active.length }) : plural('transfers.finished', all.length);
    // Screen-reader announcements concern state, not each byte/speed sample.
    const signature = rows.map(r => `${r.key || r.id}:${r.state}`).join('|');
    if (signature !== announcedState || clock() - announcedAt >= 2000) {
      find('-summary').textContent = summary; announcedState = signature; announcedAt = clock();
    }
    find('-total').hidden = true; // Per-file bars avoid mixing different directions/unknown totals.
    find('-clear').hidden = !all.some(r => ['done', 'failed', 'cancelled'].includes(r.state));
    const present = new Set(); const list = find('-list');
    rows.forEach((row, index) => {
      const key = row.key || `${row.direction}:${row.id}`; present.add(key);
      let node = nodes.get(key); if (!node) { node = make(row); nodes.set(key, node); }
      if (list.children[index] !== node.li) list.insertBefore(node.li, list.children[index] || null);
      const incoming = row.direction === 'incoming';
      const labels = { queued: t('transfers.queued'), running: incoming ? t('transfers.receiving') : t('transfers.sending'), waiting: t('transfers.waiting'), done: incoming ? t('transfers.received') : t('transfers.sent'), failed: t('transfers.failed'), cancelled: t('transfers.cancelled') };
      const known = Number.isFinite(row.size) && row.size >= 0;
      const percent = known ? (row.size ? Math.min(100, Math.floor(row.done * 100 / row.size)) : row.state === 'done' ? 100 : 0) : null;
      node.name.textContent = row.name; node.name.title = row.name;
      const amount = known ? t('bytes.of', { done: bytes(row.done), size: bytes(row.size) }) : bytes(row.done);
      node.detail.textContent = `${incoming ? '↓' : '↑'} ${row.reason || labels[row.state] || row.state} · ${amount}${row.state === 'running' ? ` · ${row.rate == null ? '—' : t('bytes.per_second', { rate: bytes(row.rate) })}` : ''}`;
      node.detail.classList.toggle('problem', row.state === 'failed');
      node.bar.hidden = !['running', 'queued', 'waiting'].includes(row.state);
      node.bar.setAttribute('aria-label', t('transfers.progress_of', { name: row.name })); node.bar.setAttribute('aria-valuetext', amount);
      node.bar.classList.toggle('indeterminate', !known);
      if (known) node.bar.setAttribute('aria-valuenow', String(percent)); else node.bar.removeAttribute('aria-valuenow');
      node.bar.style.setProperty('--done', `${percent || 0}%`);
      node.controls.id = row.id;
      for (const [kind, visible] of [['cancel', ['queued', 'running', 'waiting'].includes(row.state)], ['retry', row.canRetry], ['open', row.canOpen]]) {
        const button = node.controls[kind]; button.hidden = !visible; button.setAttribute('aria-label', t('common.action_for', { action: button.dataset.label || button.textContent, name: row.name }));
      }
    });
    for (const [key, node] of nodes) if (!present.has(key)) { node.li.remove(); nodes.delete(key); }
  }
}
/** WN-129: finished transfers shown before "Show all" (running ones always show). */
export const TRANSFERS_SHOWN = 3;
