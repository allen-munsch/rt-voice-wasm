---
name: whisper-realtime-streaming
description: Build and debug real-time voice streaming with whisper.cpp in Rust - Twilio Media Streams WebSocket server, StreamingPipeline window/overlap, browser mic capture to 16k, agent routing. Verified on rt-voice-wasm phone receptionist.
---

# Whisper realtime streaming (phone receptionist)

## Architecture
- `src/bin/server.rs`: Twilio Media Streams WS listener on 8080. Flow: base64 mu-law 8k payloads -> `decode_mulaw_8k_to_16k` (src/audio.rs) -> `StreamingPipeline::with_speed(16000, factor)` -> `WhisperContext::transcribe` -> JSON events back over WS.
- Browser (`web/receptionist.html`): getUserMedia -> Web Audio -> JS downsampler -> WS. Server runs Greeting -> Routing -> Respond/Transfer/Escalate/Closing state machine (src/agent.rs).

## Gotchas (each cost real debugging time)
1. Browser ignores a forced sample rate. `new AudioContext({sampleRate: 16000})` does NOT yield 16k audio; use the native rate (often 44100) and decimate in JS before sending.
2. Call flush() on disconnect. StreamingPipeline buffers a window; if the WS closes without flush, trailing speech is silently dropped. Call `pipeline.flush()` in the call handler on close, then transcribe the remainder.
3. Transport (RawWsTransport): writer is a SplitSink in a dedicated std::thread running `tokio Handle::current().block_on()`; enqueue events via `std::sync::mpsc::Sender` (not a tokio unbounded sender). SinkExt::send() already polls poll_flush internally, so an explicit .flush() after .send() is redundant. Final events (full_transcript) must be enqueued BEFORE the Sender drops or the writer thread dies with the socket unpinned. Wire format is {"event": <kind>, "text": <text>} - Rust field is `kind`, JSON key is `event`; browser parses `msg.event`.
4. Sliding window overlap: keep the tail via split_off at `step_samples` (NOT buffer.len() - step_samples, which keeps the wrong chunk). Window length sets the latency ceiling (2s window / 1s step for phone latency); compute is not the bottleneck on host (RTF ~0.02 for tiny.en).

## Verification
Run `cargo test` - the suite covers stream windowing, transport message handling, the audio path, engine tests (whisper + moonshine behind the SttEngine trait), and a golden-audio test; a green run proves the pipeline changes hold together. Moonshine engine tests need `LD_LIBRARY_PATH=build/moonshine` (dlopen-loaded via libloading - direct linking crashes with `free(): invalid pointer` from the whisper ggml / ONNX Runtime allocator conflict).
