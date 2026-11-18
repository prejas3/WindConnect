// CCS/1 PURPOSE:WN-65 design review: light and dark screenshots of every Windows window (panel with each section,
// panel while pairing, main window, SMS, tray menu) for comparing with the mockups. Stubbed IPC in headless Chromium,
// nothing asserted beyond "no page errors". Out: UI_ARTIFACTS or %TEMP%/windconnect-wn65-screens. DEP:Playwright.
const assert = require('node:assert/strict');
const http = require('node:http');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const root = path.resolve(__dirname, '../frontend');
const out = process.env.UI_ARTIFACTS || path.join(os.tmpdir(), 'windconnect-wn65-screens');
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.ttf': 'font/ttf', '.svg': 'image/svg+xml' };
const server = http.createServer(async (req, res) => {
  const f = path.resolve(root, '.' + new URL(req.url, 'http://x').pathname);
  if (!f.startsWith(root + path.sep)) { res.writeHead(403).end(); return; }
  try { res.setHeader('Content-Type', types[path.extname(f)] || 'text/plain'); res.end(await fs.readFile(f)); }
  catch { res.writeHead(404).end(); }
});
const cover = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAEAAAABACAIAAAAlC+aJAAABZElEQVR4nNXaW26DMBhE4VP221V2Q93BVL7FgGw/97wEFCD6IgblHzlfvz/fSQjt9bZTtmWnvkE/57MzTuZ4bd1ur8349N219fj22nKUS61Pcqn1ZNwBqT7tDnj1hEut5xYhpT4jQlY9NUJiPcml1qc9xF495QuY9dAjZNWnPcRePWMWsuqpP2RifW4RUup5j9M2PY9xWqjPHKedevo4rdVTImTWh3Mn/vd6Tp3YoM+2E0v0rDuxR8+iE6v0eXdim55HJxbqmZ3YqU/vxFo9PUJaPSVCZn04dGKDnm0nluhZd2KPPotOrNKzWGJS6XmP0zZ9HuO0UM8cp516eoS0+pQImfV8Vuql+pw6sUHPthNL9Kw7sUefRSdW6Xl3YpueRycW6jM7sVPPjJBTT4+QVp+2Uu/VM9YHrHq2nViiz7oTe/Ss/3bp0bNYYlLp8x6nbXoe47RQz4yQU58eIa2e+kss1gN/r7FRvsaSXc4AAAAASUVORK5CYII=';
const now = Date.now();
const devices = [
  { key: 'a'.repeat(64), name: 'Pixel 9', fingerprint: 'K4TQ-9WZM-2HXP', paired_at: 1794300000, connected: true, owed: false },
  { key: 'b'.repeat(64), name: 'Galaxy S25', fingerprint: 'M2AB-7QZC-1KDE', paired_at: 1794200000, connected: false, owed: false },
];
const connected = {
  phase: 'connected', phone: 'Pixel 9', qr: null, message: '', skipped_records: 0, code: null, connected: true, devices,
  battery: { level: 82, charging: false }, find: { available: true, state: 'idle', last: null, ring_ms: null },
  media: { app: 'Spotify', title: 'Kaonashi', artist: 'Daria Zawiałow', album: 'Helsinki', art: cover, playing: true, position_ms: 61000, duration_ms: 214000, actions: 15, speed_permille: 1000, received_ms: now },
  phone_volume: { level: 40, muted: false, controllable: true }, call_toasts: true,
  missed_calls: [{ who: 'Ola', number: '+48 600 100 200', date_ms: now - 3600e3 }],
  notifications: [{ tag: 'n1', app: 'Signal', title: 'Ola', text: 'Będziesz o 19? Biorę klucze od Piotra', reply: 'Odpowiedz', status: null }],
  sms: true, touchpad: { allowed: true, active: false, message: '' },
};
const width = 21, qr = { width, dark: Array.from({ length: width * width }, (_, i) => (i * 7) % 3 === 0) };
const pairing = { phase: 'waiting', phone: null, qr, message: '', skipped_records: 0, code: null, connected: false, devices: [] };
const files = { phone: 'Pixel 9', active: 1, activeSize: 100, activeDone: 64, rows: [
  { key: 'o:1', id: 1, direction: 'outgoing', name: 'IMG_2041–2063.zip', state: 'running', done: 64e6, size: 100e6, rate: 9.4e6, canRetry: false, canOpen: false },
] };
const history = [{ id: 1, kind: 'text', preview: 'https://example.com/plan', direction: 'to-phone', at: now - 60e3, pinned: true, ready: true }];
const thumbs = ["data:image/jpeg;base64,/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAYEBQYFBAYGBQYHBwYIChAKCgkJChQODwwQFxQYGBcUFhYaHSUfGhsjHBYWICwgIyYnKSopGR8tMC0oMCUoKSj/2wBDAQcHBwoIChMKChMoGhYaKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCj/wAARCABAAEADASIAAhEBAxEB/8QAHwAAAQUBAQEBAQEAAAAAAAAAAAECAwQFBgcICQoL/8QAtRAAAgEDAwIEAwUFBAQAAAF9AQIDAAQRBRIhMUEGE1FhByJxFDKBkaEII0KxwRVS0fAkM2JyggkKFhcYGRolJicoKSo0NTY3ODk6Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWGh4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl5ufo6erx8vP09fb3+Pn6/8QAHwEAAwEBAQEBAQEBAQAAAAAAAAECAwQFBgcICQoL/8QAtREAAgECBAQDBAcFBAQAAQJ3AAECAxEEBSExBhJBUQdhcRMiMoEIFEKRobHBCSMzUvAVYnLRChYkNOEl8RcYGRomJygpKjU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hpanN0dXZ3eHl6goOEhYaHiImKkpOUlZaXmJmaoqOkpaanqKmqsrO0tba3uLm6wsPExcbHyMnK0tPU1dbX2Nna4uPk5ebn6Onq8vP09fb3+Pn6/9oADAMBAAIRAxEAPwDvkepkeqKvUqvXy8j2ZTL6PUyvVBXqZXrKRhKZfR6mR6z1kpxmx3rJowlI0DNimmb3rPM3vTDN70lE55SMNXqVXqgslSLJXWztlI0FkqVZKz1kpxmrNowlI0DNimmas8ze9NM3vQomEpF8zU0zVnmb3ppm96pRMJSKSyVIslUFkpfO960aO2UjQM1NM3vWeZvemmb3oUTGUjQM3vTTN71nmb3ppm96pRMZSL5mppm96oGb3ppm96pRMJMj86mmas8ze9IZveqUTtci+ZqQzVnmb3ppmqlEykzQM1NM1Z5m96QzVSiYyZfM1NM1UDN700zVXKZSZ//Z", "data:image/jpeg;base64,/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAYEBQYFBAYGBQYHBwYIChAKCgkJChQODwwQFxQYGBcUFhYaHSUfGhsjHBYWICwgIyYnKSopGR8tMC0oMCUoKSj/2wBDAQcHBwoIChMKChMoGhYaKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCj/wAARCABAAEADASIAAhEBAxEB/8QAHwAAAQUBAQEBAQEAAAAAAAAAAAECAwQFBgcICQoL/8QAtRAAAgEDAwIEAwUFBAQAAAF9AQIDAAQRBRIhMUEGE1FhByJxFDKBkaEII0KxwRVS0fAkM2JyggkKFhcYGRolJicoKSo0NTY3ODk6Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWGh4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl5ufo6erx8vP09fb3+Pn6/8QAHwEAAwEBAQEBAQEBAQAAAAAAAAECAwQFBgcICQoL/8QAtREAAgECBAQDBAcFBAQAAQJ3AAECAxEEBSExBhJBUQdhcRMiMoEIFEKRobHBCSMzUvAVYnLRChYkNOEl8RcYGRomJygpKjU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hpanN0dXZ3eHl6goOEhYaHiImKkpOUlZaXmJmaoqOkpaanqKmqsrO0tba3uLm6wsPExcbHyMnK0tPU1dbX2Nna4uPk5ebn6Onq8vP09fb3+Pn6/9oADAMBAAIRAxEAPwD0oze9MM1UTN70wze9fLKJ4smXjN70wze9UTN70wze9WonPJl4ze9MM1UTL70ebVqBzy1L3mUvmVR8yl8yt4wIsVzNTTNVAzU0ze9YqJ3yZeM1MMtUTLR5lWomEtS95tL5tUfNpfNraMCLF4SUolqj5tL5tdEYCsUDNTTLVEze9J5vvXMonUy95tL5lUfN96Xza1jAixe82l82qPm+9Hm+9bRgKxf8yjzao+bS+b71tGIrGZ5tL5tUfNpfNrmUDosXvNpfNqh5tL5tbRiKxe82l82qPm0ebWsYisX/ADaPNqj5tL5vvWqiKx//2Q==", "data:image/jpeg;base64,/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAYEBQYFBAYGBQYHBwYIChAKCgkJChQODwwQFxQYGBcUFhYaHSUfGhsjHBYWICwgIyYnKSopGR8tMC0oMCUoKSj/2wBDAQcHBwoIChMKChMoGhYaKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCj/wAARCABAAEADASIAAhEBAxEB/8QAHwAAAQUBAQEBAQEAAAAAAAAAAAECAwQFBgcICQoL/8QAtRAAAgEDAwIEAwUFBAQAAAF9AQIDAAQRBRIhMUEGE1FhByJxFDKBkaEII0KxwRVS0fAkM2JyggkKFhcYGRolJicoKSo0NTY3ODk6Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWGh4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl5ufo6erx8vP09fb3+Pn6/8QAHwEAAwEBAQEBAQEBAQAAAAAAAAECAwQFBgcICQoL/8QAtREAAgECBAQDBAcFBAQAAQJ3AAECAxEEBSExBhJBUQdhcRMiMoEIFEKRobHBCSMzUvAVYnLRChYkNOEl8RcYGRomJygpKjU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hpanN0dXZ3eHl6goOEhYaHiImKkpOUlZaXmJmaoqOkpaanqKmqsrO0tba3uLm6wsPExcbHyMnK0tPU1dbX2Nna4uPk5ebn6Onq8vP09fb3+Pn6/9oADAMBAAIRAxEAPwD0Lzad5tUPNpfNojE+NsX/ADaUS1Q82lEtdMYisX/No87FUTNio2n966IoFG5eef3qF5/eqLT+9RPPW0UaxpjfNpRLVDzaXza4IxHyl/zaDNiqHm4pjT+9bxQKNy80/vUTT+9UWn96iaatUjWMC80/vUTTe9UWm96iab3rVG0YFnzaPNxVDzsd6Y03vXHFGXLcvtP71E03vVFpqjaatEaRgXmm96iab3qk03vUbTe9aI2jAutN71E01UmmqNpq1RrGBbab3qNpqomb3pjTe9caMlAutNUbTVSM3vTDNWiNFAutNUbTVSM3vTGm96tGqiXGmqNpapmWmNLWiNFE/9k=", "data:image/jpeg;base64,/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAYEBQYFBAYGBQYHBwYIChAKCgkJChQODwwQFxQYGBcUFhYaHSUfGhsjHBYWICwgIyYnKSopGR8tMC0oMCUoKSj/2wBDAQcHBwoIChMKChMoGhYaKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCj/wAARCABAAEADASIAAhEBAxEB/8QAHwAAAQUBAQEBAQEAAAAAAAAAAAECAwQFBgcICQoL/8QAtRAAAgEDAwIEAwUFBAQAAAF9AQIDAAQRBRIhMUEGE1FhByJxFDKBkaEII0KxwRVS0fAkM2JyggkKFhcYGRolJicoKSo0NTY3ODk6Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWGh4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl5ufo6erx8vP09fb3+Pn6/8QAHwEAAwEBAQEBAQEBAQAAAAAAAAECAwQFBgcICQoL/8QAtREAAgECBAQDBAcFBAQAAQJ3AAECAxEEBSExBhJBUQdhcRMiMoEIFEKRobHBCSMzUvAVYnLRChYkNOEl8RcYGRomJygpKjU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hpanN0dXZ3eHl6goOEhYaHiImKkpOUlZaXmJmaoqOkpaanqKmqsrO0tba3uLm6wsPExcbHyMnK0tPU1dbX2Nna4uPk5ebn6Onq8vP09fb3+Pn6/9oADAMBAAIRAxEAPwDbeb3qFpveqbTe9RNNX1UTyYwLbzVC81VGmqJpq1ibxgWnmqPzCTVXzM04PWnNY3jAtB6eHqqHp4ek5nRGJG01QtLVVpaiaWuZHFGJaaWmeZmqnmZpwetOaxtGJbD04PVUPTg9S5m8YlsPTw9VA9OD1LkdEYmY0tM8zNVfMzSh6nmscMYlsPTg9VA9OD1LkbRiWw9PD1UD04PUuRvGJbD04PVQPTg9S5G0UZAenB6qB6cHqXI4oxLYenB6ph6cHqeY1ii4HpQ9VBJThJScjaKLYenB6qCSlElTzGsUf//Z", "data:image/jpeg;base64,/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAYEBQYFBAYGBQYHBwYIChAKCgkJChQODwwQFxQYGBcUFhYaHSUfGhsjHBYWICwgIyYnKSopGR8tMC0oMCUoKSj/2wBDAQcHBwoIChMKChMoGhYaKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCj/wAARCABAAEADASIAAhEBAxEB/8QAHwAAAQUBAQEBAQEAAAAAAAAAAAECAwQFBgcICQoL/8QAtRAAAgEDAwIEAwUFBAQAAAF9AQIDAAQRBRIhMUEGE1FhByJxFDKBkaEII0KxwRVS0fAkM2JyggkKFhcYGRolJicoKSo0NTY3ODk6Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWGh4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl5ufo6erx8vP09fb3+Pn6/8QAHwEAAwEBAQEBAQEBAQAAAAAAAAECAwQFBgcICQoL/8QAtREAAgECBAQDBAcFBAQAAQJ3AAECAxEEBSExBhJBUQdhcRMiMoEIFEKRobHBCSMzUvAVYnLRChYkNOEl8RcYGRomJygpKjU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hpanN0dXZ3eHl6goOEhYaHiImKkpOUlZaXmJmaoqOkpaanqKmqsrO0tba3uLm6wsPExcbHyMnK0tPU1dbX2Nna4uPk5ebn6Onq8vP09fb3+Pn6/9oADAMBAAIRAxEAPwDWD08PVMPTw9fUOR7sUXA9PD1TD04PUOR0RRdD0vm1T8yk82s5TN07Fwy0hlqn5tIZa55TKuZ4enh6pB6eJK6HI4oouB6Xzap+ZSebWcpmqZc82gy1S82k82sJTKuXDLSebVPzaTza55THcrCSl8yqfmUnm11uZzpl3zaTzapebR5tYymO5c82k82qfm0nm1hKQ7lzzKTzap+bSebWMpDuQebSebVPzaTza6pSMLlzzaPNqkZaPNrFyHcueZSeZVPzaTzfespSC5c8yjzKp+ZSeb71k2PmP//Z"];
const gallery = { available: true, photos: Array.from({ length: 5 }, (_, i) => ({ id: i + 1, name: `IMG_20${41 + i}.jpg`, date_ms: now - i * 3600e3, screenshot: i === 4, thumb: thumbs[i] })) };
const threads = [{ id: 1, title: 'Ola', snippet: 'Będziesz o 19?', date_ms: now - 600e3, unread: 1, count: 12, people: [{ address: '+48600100200' }] },
  { id: 2, title: 'InPost', snippet: 'Paczka czeka w paczkomacie KRA21M', date_ms: now - 7200e3, unread: 0, count: 3, people: [{ address: 'InPost' }] }];

function stub({ state, files, history, gallery, threads }) {
  window.__listeners = {};
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => ({
      pairing_snapshot: state, files_snapshot: files, panel_snapshot: { shortcut_error: null },
      clipboard_snapshot: { enabled: true, status: 'Schowek dostarczony do telefonu', shortcut_error: false, image_limit_mb: 16 },
      clipboard_history: history, gallery_state: gallery, core_version: '0.77.0', ui_dev_mode: false,
      sms_status: { connected: true, available: true }, sms_threads: threads, sms_sims: [{ id: 1, label: 'SIM 1', default: true }],
      sms_thread: { cached: false, messages: [] },
    })[cmd] ?? (cmd.startsWith('sms_') || cmd === 'clipboard_action' ? null : null) },
    event: { listen: async (name, cb) => { window.__listeners[name] = cb; return () => {}; } },
  };
}

(async () => {
  await fs.mkdir(out, { recursive: true });
  await new Promise(r => server.listen(0, '127.0.0.1', r));
  const base = `http://127.0.0.1:${server.address().port}`;
  const browser = await chromium.launch({ headless: true });
  try {
    for (const theme of ['light', 'dark']) {
      const shots = [
        ['panel-connected', 'panel.html', connected, { width: 380, height: 760 }],
        ['panel-pairing', 'panel.html', pairing, { width: 380, height: 760 }],
        ['main-empty', 'index.html', pairing, { width: 920, height: 720 }],
        ['sms', 'sms.html', connected, { width: 960, height: 680 }],
        ['tray-menu', 'tray-menu.html', connected, { width: 280, height: 250 }],
      ];
      for (const [name, file, state, viewport] of shots) {
        const page = await browser.newPage({ locale: 'pl-PL', viewport, colorScheme: theme });
        const errors = []; page.on('pageerror', e => errors.push(e.message));
        await page.addInitScript(stub, { state: state, files: files, history: history, gallery: gallery, threads: threads });
        await page.goto(`${base}/${file}`);
        await page.waitForTimeout(600);
        await page.screenshot({ path: path.join(out, `${theme}-${name}.png`), fullPage: true });
        assert.deepEqual(errors, [], `${theme} ${name}: page errors`);
        await page.close();
      }
    }
    console.log(`Screenshots: ${out}`);
  } finally {
    await browser.close();
    server.close();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
