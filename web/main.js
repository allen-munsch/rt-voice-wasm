// Custom whisper.wasm API (patched emscripten.cpp):
//   Module.init(path) -> context index (1-based, 0 = failure)
//   Module.free(index)
//   Module.full(index, Float32Array, lang, nthreads, translate) -> JSON string

import './whisper/helpers.js';
import WhisperModule from './whisper/whisper.js';

let Module = null;
let ctxIndex = 0;
let transcript = '';

const statusEl = document.getElementById('status');
const transcriptEl = document.getElementById('transcript');
const btnStart = document.getElementById('btn-start');
const btnStop = document.getElementById('btn-stop');
const btnClear = document.getElementById('btn-clear');
const metricsEl = document.getElementById('metrics');

let audioCtx = null;
let workletNode = null;
let audioBuffer = [];
const TARGET_SAMPLE_RATE = 16000;
const WINDOW_SAMPLES = TARGET_SAMPLE_RATE * 4; // 4s
const STEP_SAMPLES = TARGET_SAMPLE_RATE * 2;   // 2s

function setStatus(text) {
  statusEl.textContent = text;
  console.log('[status]', text);
}

function appendTranscript(text) {
  if (text && text.trim()) {
    transcript += text.trim() + ' ';
    transcriptEl.textContent = transcript;
  }
}

function updateMetrics(inferenceMs, audioDurationS) {
  const rtf = (inferenceMs / 1000) / audioDurationS;
  metricsEl.textContent =
    `RTF: ${rtf.toFixed(3)} | Inf: ${inferenceMs.toFixed(0)}ms | Audio: ${audioDurationS.toFixed(1)}s`;
}

async function loadModel() {
  setStatus('Loading WASM module...');
  Module = await WhisperModule();

  setStatus('Downloading model (ggml-tiny.en-q5_1.bin, 31MB)...');
  const resp = await fetch('models/ggml-tiny.en-q5_1.bin');
  if (!resp.ok) throw new Error(`HTTP ${resp.status}`);

  const reader = resp.body.getReader();
  const chunks = [];
  let received = 0;
  const total = +resp.headers.get('Content-Length') || 0;

  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    chunks.push(value);
    received += value.length;
    if (total) setStatus(`Downloading model: ${Math.round(received / total * 100)}%`);
  }

  const data = new Uint8Array(received);
  let pos = 0;
  for (const c of chunks) { data.set(c, pos); pos += c.length; }

  Module.FS.writeFile('/model.bin', data);

  setStatus('Initializing whisper...');
  ctxIndex = Module.init('/model.bin');
  if (!ctxIndex) throw new Error('Model init returned 0');

  setStatus(`Ready (ctx #${ctxIndex}).`);
  btnStart.disabled = false;
}

async function startMic() {
  try {
    audioCtx = new AudioContext({ sampleRate: TARGET_SAMPLE_RATE });
    await audioCtx.audioWorklet.addModule('worklet.js');

    const stream = await navigator.mediaDevices.getUserMedia({
      audio: { channelCount: 1, sampleRate: TARGET_SAMPLE_RATE }
    });

    const source = audioCtx.createMediaStreamSource(stream);
    workletNode = new AudioWorkletNode(audioCtx, 'mic-worklet');
    source.connect(workletNode);

    workletNode.port.onmessage = (event) => {
      const { samples } = event.data;
      for (const s of samples) audioBuffer.push(s);

      if (audioBuffer.length >= WINDOW_SAMPLES) {
        const window = audioBuffer.slice(-WINDOW_SAMPLES);
        const rms = Math.sqrt(window.reduce((a, b) => a + b * b, 0) / window.length);
        if (rms > 0.005) {
          transcribeWindow(new Float32Array(window));
        }
        audioBuffer = audioBuffer.slice(STEP_SAMPLES);
      }
    };

    setStatus('Recording...');
    btnStart.disabled = true;
    btnStop.disabled = false;
  } catch (err) {
    setStatus(`Mic error: ${err.message}`);
    console.error(err);
  }
}

function transcribeWindow(samples) {
  if (!Module || !ctxIndex) return;

  // full runs whisper_full synchronously, returns JSON
  const t0 = performance.now();
  const json = Module.full(ctxIndex, samples, 'en', 1, false);
  const inferenceMs = performance.now() - t0;

  if (json) {
    try {
      const result = JSON.parse(json);
      if (result.segments) {
        for (const seg of result.segments) {
          appendTranscript(seg.text);
        }
      }
    } catch (e) {
      console.error('JSON parse error:', e);
    }
  }

  const audioDurationS = samples.length / TARGET_SAMPLE_RATE;
  updateMetrics(inferenceMs, audioDurationS);
}

function stopMic() {
  if (workletNode) {
    workletNode.port.onmessage = null;
    workletNode.disconnect();
    workletNode = null;
  }
  if (audioCtx) { audioCtx.close(); audioCtx = null; }
  audioBuffer = [];
  setStatus('Stopped.');
  btnStart.disabled = false;
  btnStop.disabled = true;
}

function clearTranscript() {
  transcript = '';
  transcriptEl.textContent = '';
}

btnStart.addEventListener('click', startMic);
btnStop.addEventListener('click', stopMic);
btnClear.addEventListener('click', clearTranscript);

setStatus('Initializing...');
loadModel().catch(err => {
  setStatus(`Init error: ${err.message}`);
  console.error(err);
});
