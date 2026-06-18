const NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'];
const HISTORY = 8; // seconds of graph
const LEAD_FRAC = 1 / 3; // keep the leading tip this far in from the right edge
const HOP_GAP = 0.13; // seconds without data that breaks the line
const VIEW_HALF = 110; // cents shown above/below the centred note (~2 semitones total)
const KEEP_SELECTED = 0.55; // hold the followed track until another is this much stronger
const ACTIVE_GAP = 0.3; // seconds of silence before the readout dims

let wasm = null;
let a4 = 440;
let audioCtx = null;
let stream = null;
let wakeLock = null;
let running = false;

let lastFrameTime = 0; // engine time of newest analysis frame
let lastFrameWall = 0;
let lastDrawWall = performance.now();

const history = []; // {t, absCents, id} of the strongest pitch over time
let selectedId = -1;
let lastFreq = 0;
let lastDataT = -Infinity;
let centerCents = null; // y-centre in abs cents (rel 440); glides toward the pitch

const canvas = document.getElementById('scope');
const statusEl = document.getElementById('status');
const toggleEl = document.getElementById('toggle');
const levelEl = document.getElementById('levelbar');
const a4valEl = document.getElementById('a4val');

async function loadWasm() {
  try {
    const { instance } = await WebAssembly.instantiateStreaming(fetch('tunerfish.wasm'));
    wasm = instance.exports;
  } catch {
    const buf = await (await fetch('tunerfish.wasm')).arrayBuffer();
    const { instance } = await WebAssembly.instantiate(buf);
    wasm = instance.exports;
  }
}

// abs cents are relative to 440 Hz so stored points survive an A4 change
const refCents = () => 1200 * Math.log2(a4 / 440);
const noteCents = (midi) => (midi - 69) * 100 + refCents();
const midiFromCents = (c) => Math.round(69 + (c - refCents()) / 100);
const noteName = (midi) => `${NAMES[((midi % 12) + 12) % 12]}${Math.floor(midi / 12) - 1}`;

function selectStrongest(out, n) {
  let best = -1;
  let bestConf = 0;
  let selFreq = 0;
  let selConf = 0;
  for (let i = 0; i < n; i++) {
    const base = 4 + i * 6;
    const id = out[base];
    const conf = out[base + 3];
    if (conf > bestConf) {
      bestConf = conf;
      best = id;
    }
    if (id === selectedId) {
      selConf = conf;
      selFreq = out[base + 2];
    }
  }
  if (best < 0) return null;
  // keep following the current track unless another is clearly stronger
  if (selConf >= KEEP_SELECTED * bestConf) {
    return { id: selectedId, freq: selFreq };
  }
  const base = 4 + indexOfId(out, n, best) * 6;
  return { id: best, freq: out[base + 2] };
}

function indexOfId(out, n, id) {
  for (let i = 0; i < n; i++) if (out[4 + i * 6] === id) return i;
  return 0;
}

function readFrame() {
  const out = new Float32Array(wasm.memory.buffer, wasm.out_ptr(), wasm.out_len());
  const n = out[0];
  lastFrameTime = out[1];
  lastFrameWall = performance.now();
  levelEl.style.width = `${Math.min(100, Math.round(out[2] * 600))}%`;

  const sel = selectStrongest(out, n);
  if (!sel) return;
  selectedId = sel.id;
  lastFreq = sel.freq;
  lastDataT = lastFrameTime;
  history.push({ t: lastFrameTime, absCents: 1200 * Math.log2(sel.freq / 440), id: sel.id });
}

function feed(samples) {
  if (!wasm || !running) return;
  const cap = wasm.in_cap();
  let off = 0;
  while (off < samples.length) {
    const n = Math.min(cap, samples.length - off);
    new Float32Array(wasm.memory.buffer, wasm.in_ptr(), n).set(samples.subarray(off, off + n));
    if (wasm.push_samples(n) > 0) readFrame();
    off += n;
  }
}

const offsetRGB = (o) => (Math.abs(o) < 3 ? [61, 220, 132] : Math.abs(o) < 10 ? [240, 180, 41] : [244, 100, 78]);
const offsetColor = (o) => `rgb(${offsetRGB(o).join(',')})`;
const offsetRGBA = (o, a) => `rgba(${offsetRGB(o).join(',')}, ${a})`;

function draw() {
  requestAnimationFrame(draw);
  const wallNow = performance.now();
  const dt = Math.min((wallNow - lastDrawWall) / 1000, 0.1);
  lastDrawWall = wallNow;
  const now = lastFrameTime + Math.min((wallNow - lastFrameWall) / 1000, 1.0);

  const cutoff = now - HISTORY - 0.5;
  while (history.length && history[0].t < cutoff) history.shift();
  if (history.length > 0) statusEl.style.display = 'none';

  const dpr = devicePixelRatio || 1;
  const w = Math.round(canvas.clientWidth * dpr);
  const h = Math.round(canvas.clientHeight * dpr);
  if (w === 0 || h === 0) return;
  if (canvas.width !== w || canvas.height !== h) {
    canvas.width = w;
    canvas.height = h;
  }
  const ctx = canvas.getContext('2d');
  ctx.clearRect(0, 0, w, h);

  const tip = history.length ? history[history.length - 1] : null;
  const active = now - lastDataT < ACTIVE_GAP;

  // the view glides toward the heard pitch; no snapping to notes. Catch up
  // fast over big jumps (a new string) but stay smooth for fine wobble.
  if (tip && active) {
    if (centerCents === null) {
      centerCents = tip.absCents;
    } else {
      const delta = tip.absCents - centerCents;
      const rate = Math.abs(delta) > 120 ? 14 : 5;
      centerCents += delta * (1 - Math.exp(-rate * dt));
    }
  }
  if (centerCents === null) return; // nothing heard yet

  const nearMidi = midiFromCents(tip ? tip.absCents : centerCents);
  const offset = tip ? tip.absCents - noteCents(nearMidi) : 0;

  const margin = 14 * dpr;
  const yOf = (c) => h / 2 - ((c - centerCents) / VIEW_HALF) * (h / 2 - margin);
  const xNow = w * (1 - LEAD_FRAC);
  const xOf = (t) => xNow - (now - t) * (xNow / HISTORY);

  // note gridlines; the one nearest the pitch is emphasised and turns green
  // when locked, so the target reads clearly without the view snapping
  const loMidi = midiFromCents(centerCents - VIEW_HALF) - 1;
  const hiMidi = midiFromCents(centerCents + VIEW_HALF) + 1;
  ctx.textBaseline = 'middle';
  for (let m = loMidi; m <= hiMidi; m++) {
    const y = yOf(noteCents(m));
    const isNear = m === nearMidi;
    ctx.strokeStyle = isNear
      ? active
        ? offsetRGBA(offset, 0.55)
        : 'rgba(125, 134, 148, 0.5)'
      : 'rgba(125, 134, 148, 0.18)';
    ctx.lineWidth = (isNear ? 1.6 : 1) * dpr;
    ctx.beginPath();
    ctx.moveTo(0, y);
    ctx.lineTo(w, y);
    ctx.stroke();
    ctx.fillStyle = isNear ? 'rgba(223, 230, 240, 0.9)' : 'rgba(125, 134, 148, 0.55)';
    ctx.font = `${(isNear ? 13 : 11) * dpr}px system-ui`;
    ctx.fillText(noteName(m), 6 * dpr, y - 9 * dpr);
  }

  drawSalience(ctx, w, h, dpr, yOf, xNow);

  // the pitch line
  ctx.strokeStyle = '#4ea1ff';
  ctx.lineWidth = 2.5 * dpr;
  ctx.lineJoin = 'round';
  ctx.lineCap = 'round';
  ctx.beginPath();
  let prevT = -Infinity;
  let prevId = null;
  let prevCents = 0;
  for (const p of history) {
    const x = xOf(p.t);
    const y = yOf(p.absCents);
    // break on silence, a track switch, or an implausible one-frame jump
    // (>150 cents in ~43 ms is a detection glitch, not a played slide)
    const jump = Math.abs(p.absCents - prevCents) > 150;
    if (p.t - prevT > HOP_GAP || p.id !== prevId || jump) ctx.moveTo(x, y);
    else ctx.lineTo(x, y);
    prevT = p.t;
    prevId = p.id;
    prevCents = p.absCents;
  }
  ctx.stroke();

  // tip marker and the big readout
  if (tip) {
    if (active) {
      ctx.fillStyle = offsetColor(offset);
      ctx.beginPath();
      ctx.arc(xOf(tip.t), yOf(tip.absCents), 5 * dpr, 0, 2 * Math.PI);
      ctx.fill();
    }
    drawReadout(ctx, w, dpr, nearMidi, offset, active);
  }
}

// The integrated salience profile the detector works from, drawn as faint
// bars in the right-hand lead strip so its alignment with the line is visible.
function drawSalience(ctx, w, h, dpr, yOf, xNow) {
  if (!wasm || typeof wasm.salience_ptr !== 'function') return;
  const n = wasm.salience_len();
  if (!n) return;
  const sal = new Float32Array(wasm.memory.buffer, wasm.salience_ptr(), n);
  const f0min = wasm.salience_f0_min();
  const bc = wasm.salience_bin_cents();
  let smax = 1e-9;
  for (let k = 0; k < n; k++) if (sal[k] > smax) smax = sal[k];
  const strip = w - xNow;
  for (let k = 0; k < n; k++) {
    const a = sal[k] / smax;
    if (a < 0.06) continue;
    const absCents = (k * bc) + 1200 * Math.log2(f0min / 440);
    const y = yOf(absCents);
    if (y < 0 || y > h) continue;
    ctx.fillStyle = `rgba(78, 161, 255, ${(a * 0.4).toFixed(3)})`;
    ctx.fillRect(xNow, y - dpr, a * strip, 2 * dpr);
  }
}

function drawReadout(ctx, w, dpr, nearMidi, offset, active) {
  ctx.globalAlpha = active ? 1 : 0.5;
  ctx.textAlign = 'center';
  ctx.textBaseline = 'alphabetic';
  ctx.fillStyle = '#dfe6f0';
  ctx.font = `700 ${44 * dpr}px system-ui`;
  ctx.fillText(noteName(nearMidi), w / 2, 56 * dpr);
  ctx.fillStyle = offsetColor(offset);
  ctx.font = `600 ${22 * dpr}px system-ui`;
  ctx.fillText(`${offset >= 0 ? '+' : ''}${offset.toFixed(1)}¢`, w / 2, 86 * dpr);
  ctx.globalAlpha = 1;
  ctx.textAlign = 'left';
}

async function start() {
  if (!wasm) await loadWasm();
  audioCtx = new AudioContext();
  wasm.init(audioCtx.sampleRate);
  stream = await navigator.mediaDevices.getUserMedia({
    audio: { echoCancellation: false, noiseSuppression: false, autoGainControl: false },
  });
  await audioCtx.audioWorklet.addModule('worklet.js');
  const src = audioCtx.createMediaStreamSource(stream);
  const node = new AudioWorkletNode(audioCtx, 'capture');
  node.port.onmessage = (e) => feed(e.data);
  // a silent sink keeps the worklet pulled by the rendering graph
  const sink = audioCtx.createGain();
  sink.gain.value = 0;
  src.connect(node).connect(sink).connect(audioCtx.destination);
  await audioCtx.resume();
  try {
    wakeLock = await navigator.wakeLock?.request('screen');
  } catch {}
  running = true;
  lastFrameWall = performance.now();
  toggleEl.textContent = 'Stop';
  toggleEl.classList.add('running');
  statusEl.textContent = 'Listening… play a note.';
}

async function stop() {
  running = false;
  stream?.getTracks().forEach((t) => t.stop());
  await audioCtx?.close().catch(() => {});
  wakeLock?.release().catch(() => {});
  audioCtx = null;
  stream = null;
  wakeLock = null;
  selectedId = -1;
  toggleEl.textContent = 'Start';
  toggleEl.classList.remove('running');
}

toggleEl.addEventListener('click', () => {
  (running ? stop() : start()).catch((err) => {
    statusEl.style.display = '';
    statusEl.textContent = `Could not start: ${err.message}`;
    stop();
  });
});

document.addEventListener('visibilitychange', () => {
  if (!document.hidden && running && navigator.wakeLock) {
    navigator.wakeLock.request('screen').then((l) => (wakeLock = l)).catch(() => {});
  }
});

function setA4(v) {
  a4 = Math.min(480, Math.max(400, v));
  a4valEl.textContent = String(a4);
}
document.getElementById('a4down').addEventListener('click', () => setA4(a4 - 1));
document.getElementById('a4up').addEventListener('click', () => setA4(a4 + 1));

// ?demo: feed a synthesized monophonic tuning session through the engine
async function demo() {
  await loadWasm();
  const FS = 48000;
  wasm.init(FS);
  running = true;
  statusEl.style.display = 'none';
  const cents = (c) => Math.pow(2, c / 1200);
  // one A2 string, plucked three times, tuned closer each time, last with vibrato
  const plucks = [
    { start: 0.2, dur: 2.4, cents: -22, onset: 16 },
    { start: 2.9, dur: 2.4, cents: -8, onset: 12 },
    { start: 5.6, dur: 2.4, cents: 1, onset: 10, vibrato: 4 },
  ];
  const phase = [0, 0, 0, 0, 0];
  const chunk = 2048;
  const buf = new Float32Array(chunk);
  const total = 8 * FS;
  for (let off = 0; off < total; off += chunk) {
    for (let i = 0; i < chunk; i++) {
      const t = (off + i) / FS;
      let s = 0;
      for (const p of plucks) {
        const vt = t - p.start;
        if (vt < 0 || vt > p.dur) continue;
        const vib = p.vibrato ? p.vibrato * Math.sin(2 * Math.PI * 5 * vt) : 0;
        const f = 110 * cents(p.cents + vib + p.onset * Math.exp(-vt / 0.25));
        const amp = 0.25 * Math.exp(-vt / 2.5) * Math.min(1, vt / 0.01);
        for (let h = 1; h <= 5; h++) {
          phase[h - 1] = (phase[h - 1] + (2 * Math.PI * f * h) / FS) % (2 * Math.PI);
          s += (amp / h) * Math.sin(phase[h - 1]);
        }
      }
      buf[i] = s + (Math.random() - 0.5) * 2e-3;
    }
    feed(buf);
  }
}

if (new URLSearchParams(location.search).has('demo')) {
  demo().catch((err) => (statusEl.textContent = `Demo failed: ${err.message}`));
} else {
  loadWasm().catch((err) => {
    statusEl.textContent = `Failed to load DSP module: ${err.message}`;
  });
}
requestAnimationFrame(draw);
