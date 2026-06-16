# tunerfish

Polyphonic guitar tuner in the browser. Strum all strings at once: one
scrolling line graph per detected string shows the tuning settle in cents.
Knows nothing about instruments or tunings, so it works for 4-string
ukuleles and basses, 6/7/8-string guitars, or anything with sustained
harmonic strings.

## How it works

The DSP core is Rust compiled to plain WebAssembly (no bindgen):

- STFT with an 8192-sample Hann window, zero-padded to a 16384 FFT,
  2048-sample hop. Peak frequencies are refined with phase-vocoder
  instantaneous frequency estimates for sub-cent precision.
- Polyphonic f0 estimation: harmonic-stack salience scoring with a
  Gaussian closeness weight per matched partial, picked iteratively with
  soft spectral subtraction so concurrent strings surface one by one.
  Per-candidate frequency comes from an outlier-trimmed weighted fit of
  f_h = h f0 sqrt(1 + B h^2), absorbing string inharmonicity and ignoring
  partials shared with other strings.
- A bank of Kalman filters (state: cents and cents/s) tracks each string
  over time: gated nearest-neighbor association, miss-based hold and
  death, revival of a recently dead track when the string is plucked
  again. Onset sharpness decaying into the settled pitch is modeled by
  the velocity state.

The frontend captures the mic through an AudioWorklet, feeds samples to
the WASM engine, and draws one lane per tracked string: note name, cents
offset, and an 8-second scrolling graph with gaps while the string is
silent. The A4 reference is adjustable (default 440 Hz).

Strings tuned exactly an octave or two apart share every partial, so a
simultaneous strum may merge them into one lane; pluck either string
separately and it gets its own track.

## Develop

```sh
./build.sh        # cargo build --target wasm32-unknown-unknown + copy into web/
./serve.py        # HTTPS server with self-signed cert (mic needs a secure context)
cargo test        # native tests against synthesized plucks
```

Open the printed `https://<lan-ip>:8443/` on the phone and accept the
certificate warning once.
