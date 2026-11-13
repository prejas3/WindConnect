// CCS/1 PERF:keyed nodes retain focus at <=10 dirty updates/s; no polling or per-row timers.
export function bytes(value) {
  if (!Number.isFinite(value) || value < 0) return '—';
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB']; let i = 0;
  while (value >= 1024 && i < units.length - 1) { value /= 1024; i++; }
  return `${value.toFixed(i && value < 10 ? 1 : 0)} ${units[i]}`;
}
export function createTransfers(document, action, clock = () => performance.now()) {
  const nodes = new Map(); let announcedAt = -Infinity, announcedState = '';
  const find = id => document.querySelector(`#transfers${id}`);
  find('-clear').addEventListener('click', () => action('clear'));
  function make(row) {
    const li = document.createElement('li'); li.dataset.transfer = row.key || `${row.direction}:${row.id}`;
    const text = document.createElement('div'); text.className = 'transfer-copy';
    const name = document.createElement('span'); name.className = 'name';
    const detail = document.createElement('small');
    const bar = document.createElement('div'); bar.className = 'progress'; bar.setAttribute('role', 'progressbar');
    bar.setAttribute('aria-valuemin', '0'); bar.setAttribute('aria-valuemax', '100'); bar.append(document.createElement('span'));
    const buttons = document.createElement('div'); buttons.className = 'transfer-actions';
    const controls = {};
    for (const [kind, label] of [['cancel', 'Anuluj'], ['retry', 'Ponów'], ['open', 'Otwórz folder']]) {
      const button = document.createElement('button'); button.className = 'button quiet small'; button.textContent = label;
      button.addEventListener('click', () => action(kind, controls.id)); buttons.append(button); controls[kind] = button;
    }
    text.append(name, detail, bar); li.append(text, buttons);
    return { li, name, detail, bar, controls };
  }
  return function render(state) {
    const rows = state.rows || [], active = rows.filter(r => ['queued', 'running', 'waiting'].includes(r.state));
    find('').hidden = !rows.length;
    find('-title').textContent = 'Transfery plików';
    const summary = active.length ? `${active.length} w toku` : `${rows.length} zakończone`;
    // Screen-reader announcements concern state, not each byte/speed sample.
    const signature = rows.map(r => `${r.key || r.id}:${r.state}`).join('|');
    if (signature !== announcedState || clock() - announcedAt >= 2000) {
      find('-summary').textContent = summary; announcedState = signature; announcedAt = clock();
    }
    find('-total').hidden = true; // Per-file bars avoid mixing different directions/unknown totals.
    find('-clear').hidden = !rows.some(r => ['done', 'failed', 'cancelled'].includes(r.state));
    const present = new Set(); const list = find('-list');
    rows.forEach((row, index) => {
      const key = row.key || `${row.direction}:${row.id}`; present.add(key);
      let node = nodes.get(key); if (!node) { node = make(row); nodes.set(key, node); }
      if (list.children[index] !== node.li) list.insertBefore(node.li, list.children[index] || null);
      const incoming = row.direction === 'incoming';
      const labels = { queued: 'W kolejce', running: incoming ? 'Odbieranie' : 'Wysyłanie', waiting: 'Czeka na połączenie', done: incoming ? 'Odebrano' : 'Wysłano', failed: 'Błąd transferu', cancelled: 'Anulowano' };
      const known = Number.isFinite(row.size) && row.size >= 0;
      const percent = known ? (row.size ? Math.min(100, Math.floor(row.done * 100 / row.size)) : row.state === 'done' ? 100 : 0) : null;
      node.name.textContent = row.name; node.name.title = row.name;
      const amount = known ? `${bytes(row.done)} z ${bytes(row.size)}` : bytes(row.done);
      node.detail.textContent = `${incoming ? '↓' : '↑'} ${row.reason || labels[row.state] || row.state} · ${amount}${row.state === 'running' ? ` · ${row.rate == null ? '—' : bytes(row.rate) + '/s'}` : ''}`;
      node.detail.classList.toggle('problem', row.state === 'failed');
      node.bar.hidden = !['running', 'queued', 'waiting'].includes(row.state);
      node.bar.setAttribute('aria-label', `Postęp: ${row.name}`); node.bar.setAttribute('aria-valuetext', amount);
      node.bar.classList.toggle('indeterminate', !known);
      if (known) node.bar.setAttribute('aria-valuenow', String(percent)); else node.bar.removeAttribute('aria-valuenow');
      node.bar.style.setProperty('--done', `${percent || 0}%`);
      node.controls.id = row.id;
      for (const [kind, visible] of [['cancel', ['queued', 'running', 'waiting'].includes(row.state)], ['retry', row.canRetry], ['open', row.canOpen]]) {
        const button = node.controls[kind]; button.hidden = !visible; button.setAttribute('aria-label', `${button.textContent}: ${row.name}`);
      }
    });
    for (const [key, node] of nodes) if (!present.has(key)) { node.li.remove(); nodes.delete(key); }
  };
}
