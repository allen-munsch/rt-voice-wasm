class MicWorklet extends AudioWorkletProcessor {
  process(inputs) {
    const input = inputs[0];
    if (input && input.length > 0) {
      const channel = input[0];
      this.port.postMessage({
        samples: Array.from(channel),
        sampleRate: sampleRate,
      });
    }
    return true;
  }
}

registerProcessor('mic-worklet', MicWorklet);
