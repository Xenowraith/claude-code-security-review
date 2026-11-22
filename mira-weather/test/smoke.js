// End-to-end smoke test: runs the packaged HTML in Chromium with a mocked
// miraCompanion bridge, a simulated glasses host executing the real WASM
// (speaking the companion.rs mailbox protocol), synthetic Open-Meteo replies
// and a mocked GPS fix. Saves a PNG of every glasses tab plus the phone UI.
// Usage: node test/smoke.js SkyGlance.html OUTDIR
const { chromium } = require('playwright');
const fs = require('fs');
const path = require('path');

const htmlPath = process.argv[2] || 'SkyGlance.html';
const outDir = process.argv[3] || 'test-out';
fs.mkdirSync(outDir, { recursive: true });
const html = fs.readFileSync(htmlPath, 'utf8');

// ---------- synthetic Open-Meteo data ----------
function times() {
  const out = [];
  for (let d = 22; d <= 23; d++) for (let h = 0; h < 24; h++) out.push(`2026-11-${d}T${String(h).padStart(2, '0')}:00`);
  return out;
}
function mainJSON() {
  const t = times(), n = t.length;
  const f = (fn) => t.map((_, i) => +fn(i).toFixed(2));
  return {
    current: { time: '2026-11-22T14:30', temperature_2m: 61.4, relative_humidity_2m: 64, apparent_temperature: 59.8,
      is_day: 1, precipitation: 0, weather_code: 2, wind_speed_10m: 9.6, wind_direction_10m: 315, wind_gusts_10m: 18.2, dew_point_2m: 48.9 },
    hourly: { time: t,
      temperature_2m: f(i => 52 + 10 * Math.sin((i % 24 - 9) / 24 * 2 * Math.PI)),
      precipitation_probability: f(i => i >= 16 && i <= 22 ? [10, 25, 45, 70, 60, 35, 15][i - 16] : 3),
      precipitation: f(i => i >= 17 && i <= 21 ? [0.01, 0.04, 0.12, 0.06, 0.02][i - 17] : 0),
      weather_code: f(() => 2),
      wind_speed_10m: f(i => 6 + 8 * Math.abs(Math.sin(i / 5))),
      wind_direction_10m: f(i => (300 + i * 7) % 360),
      wind_gusts_10m: f(i => 12 + 10 * Math.abs(Math.sin(i / 5))),
      uv_index: f(i => Math.max(0, 5 * Math.sin((i % 24 - 7) / 10 * Math.PI))),
      sunshine_duration: f(i => (i % 24 >= 7 && i % 24 <= 16) ? (i % 24 === 15 ? 600 : 3000) : 0) },
    daily: { time: ['2026-11-22', '2026-11-23'], temperature_2m_max: [63.2, 58], temperature_2m_min: [44.6, 40],
      sunrise: ['2026-11-22T06:58', '2026-11-23T06:59'], sunset: ['2026-11-22T16:41', '2026-11-23T16:40'],
      uv_index_max: [3.4, 2], precipitation_sum: [0.25, 0], precipitation_probability_max: [70, 10] },
  };
}
function mapJSON(url) {
  const lats = new URL(url).searchParams.get('latitude').split(',');
  const t = times();
  return lats.map((_, p) => {
    const row = Math.floor(p / 7), col = p % 7;
    return { hourly: { time: t,
      // A rain band drifting east over the next hours, heaviest in its core.
      precipitation: t.map((_, i) => { const front = (i - 14) * 0.6 - 1; const dist = Math.abs(col - front) + Math.abs(row - 3) * 0.3;
        return i >= 14 ? Math.max(0, +(0.3 - dist * 0.09).toFixed(3)) : 0; }),
      wind_speed_10m: t.map((_, i) => +(5 + row * 2 + (i % 6) * 2).toFixed(1)),
      wind_direction_10m: t.map((_, i) => (250 + col * 10 + i * 5) % 360) } };
  });
}

(async () => {
  const browser = await chromium.launch();
  const context = await browser.newContext({ geolocation: { latitude: 39.7392, longitude: -104.9903 },
    permissions: ['geolocation'], viewport: { width: 420, height: 1400 } });
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', e => errors.push(String(e)));
  page.on('console', m => { if (m.type() === 'error') errors.push(m.text()); });
  const requests = [];
  await page.route('**/*', async route => {
    const url = route.request().url();
    requests.push(url.split('?')[0]);
    if (url.startsWith('https://sky.test/')) return route.fulfill({ contentType: 'text/html', body: html });
    if (url.includes('bigdatacloud')) return route.fulfill({ json: { city: 'Denver', principalSubdivision: 'Colorado' } });
    if (url.includes('api.open-meteo.com')) {
      const isMap = new URL(url).searchParams.get('latitude').includes(',');
      return route.fulfill({ json: isMap ? mapJSON(url) : mainJSON() });
    }
    return route.abort();
  });

  await page.addInitScript(() => {
    // ---------- mocked phone bridge ----------
    const queue = []; const handlers = []; const store = new Map();
    window.__queue = queue; window.__fromApp = [];
    window.miraCompanion = {
      sendToApp: (obj) => { queue.push(JSON.stringify(obj)); window.__rebuild && window.__rebuild(); return Promise.resolve(); },
      onFromApp: (h) => { handlers.push(h); return () => {}; },
      storeGet: async k => store.has(k) ? store.get(k) : null,
      storeSet: async (k, v) => { store.set(k, v); },
      storeRemove: async k => { store.delete(k); },
    };
    window.__deliver = (json) => { const m = JSON.parse(json); window.__fromApp.push(m); handlers.forEach(h => h(m)); };

    // ---------- simulated glasses host ----------
    window.__startGlasses = async () => {
      const b64 = document.querySelector('script[type="application/mira-wasm"]').textContent.trim();
      const bytes = Uint8Array.from(atob(b64), c => c.charCodeAt(0));
      const canvas = document.createElement('canvas'); canvas.width = 640; canvas.height = 480; canvas.id = 'glasses';
      canvas.style.cssText = 'position:fixed;left:0;top:0;opacity:0;pointer-events:none';
      document.body.appendChild(canvas);
      const g = canvas.getContext('2d');
      let mem, seq = 0, rev = 1, snap = new Uint8Array(0), stats = { prims: 0, texts: 0, maxPrims: 0, maxTexts: 0 };
      const enc = new TextEncoder(), dec = new TextDecoder();
      const replies = new Map(); let nextId = 1; const outMsgs = new Map();
      function rebuild() {
        seq++; rev++;
        snap = queue.length ? enc.encode(seq + '\0' + queue[0]) : enc.encode('0\0');
      }
      window.__rebuild = () => { if (queue.length === 1) rebuild(); };
      rebuild();
      const gray = c => { const v = Math.max(0, Math.min(15, c)) * 17; return `rgb(${v},${v},${v})`; };
      const prim = () => { stats.prims++; };
      const FONTS = [24, 18, 32, 40];
      const env = {
        begin: () => { g.fillStyle = '#000'; g.fillRect(0, 0, 640, 480); stats.prims = 0; stats.texts = 0; },
        present: () => { stats.maxPrims = Math.max(stats.maxPrims, stats.prims); stats.maxTexts = Math.max(stats.maxTexts, stats.texts); window.__lastFrame = { ...stats }; },
        text: (x, y, w, h, c, font, align, spacing, p, n) => {
          prim(); stats.texts++;
          const s = dec.decode(new Uint8Array(mem.buffer, p, n));
          const px = FONTS[font] || 24; g.font = `${px}px sans-serif`; g.fillStyle = gray(c); g.textBaseline = 'top';
          g.save(); g.beginPath(); g.rect(x, y, w, h); g.clip();
          s.split('\n').forEach((line, i) => {
            const lw = g.measureText(line).width;
            const lx = align === 1 ? x + (w - lw) / 2 : align === 2 ? x + w - lw : x;
            g.fillText(line, lx, y + i * spacing + 2);
          });
          g.restore();
        },
        fill_rect: (x, y, w, h, c) => { prim(); g.fillStyle = gray(c); g.fillRect(x, y, w, h); },
        stroke_rect: (x, y, w, h, l, c) => { prim(); g.strokeStyle = gray(c); g.lineWidth = l; g.strokeRect(x + l / 2, y + l / 2, w - l, h - l); },
        fill_round_rect: (x, y, w, h, r, c) => { prim(); g.fillStyle = gray(c); g.beginPath(); g.roundRect(x, y, Math.max(0, w), Math.max(0, h), Math.min(r, w / 2, h / 2)); g.fill(); },
        stroke_round_rect: (x, y, w, h, r, l, c) => { prim(); g.strokeStyle = gray(c); g.lineWidth = l; g.beginPath(); g.roundRect(x, y, w, h, r); g.stroke(); },
        fill_circle: (x, y, r, c) => { prim(); g.fillStyle = gray(c); g.beginPath(); g.arc(x, y, r, 0, 7); g.fill(); },
        stroke_circle: (x, y, r, l, c) => { prim(); g.strokeStyle = gray(c); g.lineWidth = l; g.beginPath(); g.arc(x, y, r - l / 2, 0, 7); g.stroke(); },
        line: (a, b, c2, d, w, c) => { prim(); g.strokeStyle = gray(c); g.lineWidth = w; g.lineCap = 'round'; g.beginPath(); g.moveTo(a, b); g.lineTo(c2, d); g.stroke(); },
        fill_triangle: (a, b, c2, d, e, f, c) => { prim(); g.fillStyle = gray(c); g.beginPath(); g.moveTo(a, b); g.lineTo(c2, d); g.lineTo(e, f); g.fill(); },
        foreground_interval_ms: () => 0,
        content_cursor: (x, y, f) => { window.__cursor = [x, y, f]; },
        content_info: () => 0,
        gesture_policy: () => 1,
        app_launch: () => 0,
        ble_request: (method, p, n) => {
          const req = new Uint8Array(mem.buffer.slice(p, p + n)); const id = nextId++;
          const dv = new DataView(req.buffer);
          let reply = new Uint8Array(0);
          if (method === 0x64) { reply = new Uint8Array([1, 0, 0, 0]); }
          else if (method === 0x62) {
            const r = dv.getUint32(0, true), off = dv.getUint16(4, true);
            const out = new Uint8Array(8 + Math.min(376, Math.max(0, snap.length - off)));
            const o = new DataView(out.buffer);
            o.setUint32(0, rev, true); o.setUint16(4, off, true);
            if (off === 0 && r === rev) { o.setUint16(6, 0xffff, true); reply = out.slice(0, 8); }
            else { o.setUint16(6, snap.length, true); out.set(snap.slice(off, off + 376), 8); reply = out; }
          } else if (method === 0x63) {
            if (req[0] === 0) { const s = dv.getUint32(1, true); if (s === seq && queue.length) { queue.shift(); rebuild(); } }
            else if (req[0] === 1) {
              const mid = dv.getUint32(1, true), off = dv.getUint16(5, true), total = dv.getUint16(7, true);
              const buf = outMsgs.get(mid) || new Uint8Array(total); buf.set(req.slice(9), off); outMsgs.set(mid, buf);
              if (off + n - 9 >= total) { outMsgs.delete(mid); window.__deliver(dec.decode(buf)); }
            }
          }
          replies.set(id, reply); return id;
        },
        ble_response_len: id => replies.has(id) ? replies.get(id).length : -2,
        ble_response_read: (id, out, cap) => { const r = replies.get(id); if (!r) return -2; if (r.length > cap) return -4;
          new Uint8Array(mem.buffer, out, r.length).set(r); replies.delete(id); return r.length; },
      };
      const imports = WebAssembly.Module.imports(new WebAssembly.Module(bytes));
      const missing = imports.filter(i => !env[i.name]).map(i => i.name);
      if (missing.length) throw new Error('host lacks imports: ' + missing.join(','));
      const { instance } = await WebAssembly.instantiate(bytes, { mira: env });
      mem = instance.exports.memory; window.__app = instance.exports; window.__stats = stats;
      instance.exports.app_init();
      setInterval(() => instance.exports.app_render(150), 150);
    };
  });

  await page.goto('https://sky.test/app.html');
  await page.evaluate(() => window.__startGlasses());
  // Wait until the phone has fetched and the glasses have drained the queue.
  try {
    await page.waitForFunction(() => window.__queue.length === 0 && document.getElementById('status').textContent.startsWith('Updated'), null, { timeout: 30000 });
  } catch (e) {
    console.log('DEBUG', await page.evaluate(() => ({ status: document.getElementById('status').textContent, queue: window.__queue.length,
      head: (window.__queue[0] || '').slice(0, 80), fromApp: window.__fromApp })), errors);
    throw e;
  }
  await page.waitForTimeout(800);

  const shots = ['now', 'hourly', 'wind', 'map'];
  const report = {};
  for (let i = 0; i < shots.length; i++) {
    if (i > 0) await page.evaluate(() => window.__app.app_event(2));
    await page.waitForTimeout(300);
    const png = await page.evaluate(() => document.getElementById('glasses').toDataURL('image/png'));
    fs.writeFileSync(path.join(outDir, `glasses-${i}-${shots[i]}.png`), Buffer.from(png.split(',')[1], 'base64'));
    report[shots[i]] = await page.evaluate(() => ({ frame: window.__lastFrame, cursor: window.__cursor }));
  }
  // Tap on hourly tab toggles the page; tap on map pauses; tap on Now asks the phone to refresh.
  await page.evaluate(() => window.__app.app_event(2)); // wrap to Now
  await page.evaluate(() => window.__app.app_event(1)); // back to map
  await page.waitForTimeout(2500);
  await page.evaluate(() => window.__app.app_event(0)); // pause
  const pngMap = await page.evaluate(() => document.getElementById('glasses').toDataURL('image/png'));
  fs.writeFileSync(path.join(outDir, 'glasses-4-map-paused.png'), Buffer.from(pngMap.split(',')[1], 'base64'));
  await page.evaluate(() => { window.__app.app_event(1); window.__app.app_event(1); window.__app.app_event(0); });
  await page.waitForTimeout(300);
  const pngH2 = await page.evaluate(() => document.getElementById('glasses').toDataURL('image/png'));
  fs.writeFileSync(path.join(outDir, 'glasses-5-hourly-page2.png'), Buffer.from(pngH2.split(',')[1], 'base64'));
  await page.evaluate(() => { window.__app.app_event(1); window.__app.app_event(0); });
  await page.waitForTimeout(1500);
  const fromApp = await page.evaluate(() => window.__fromApp.map(m => m.type));
  await page.screenshot({ path: path.join(outDir, 'phone.png'), fullPage: true });
  console.log(JSON.stringify({ report, fromApp, maxFrame: await page.evaluate(() => window.__stats),
    weatherRequests: requests.filter(u => !u.startsWith('http://sky.test')), errors }, null, 1));
  await browser.close();
})().catch(e => { console.error(e); process.exit(1); });
