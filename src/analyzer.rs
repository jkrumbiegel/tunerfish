use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};
use std::f32::consts::PI;
use std::sync::Arc;

pub const NWIN: usize = 8192;
pub const NFFT: usize = 16384;
pub const HOP: usize = 2048;

const FREQ_LO: f32 = 25.0;
const FREQ_HI: f32 = 6000.0;
const MAX_PEAKS: usize = 64;

#[derive(Clone, Copy, Debug)]
pub struct Peak {
    pub freq: f32,
    pub mag: f32,
}

pub struct Analyzer {
    fs: f32,
    window: Vec<f32>,
    wsum: f32,
    ring: Vec<f32>,
    wpos: usize,
    filled: usize,
    since_hop: usize,
    fft: Arc<dyn RealToComplex<f32>>,
    fft_in: Vec<f32>,
    spec: Vec<Complex<f32>>,
    mags: Vec<f32>,
    phases: Vec<f32>,
    prev_phases: Vec<f32>,
    has_prev: bool,
    bin_lo: usize,
    bin_hi: usize,
    pub peaks: Vec<Peak>,
    pub rms: f32,
}

impl Analyzer {
    pub fn new(fs: f32) -> Self {
        let window: Vec<f32> = (0..NWIN)
            .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / NWIN as f32).cos())
            .collect();
        let wsum: f32 = window.iter().sum();
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(NFFT);
        let nbins = NFFT / 2 + 1;
        let bin_lo = ((FREQ_LO / fs * NFFT as f32).ceil() as usize).max(4);
        let bin_hi = ((FREQ_HI / fs * NFFT as f32).floor() as usize).min(nbins - 4);
        Analyzer {
            fs,
            window,
            wsum,
            ring: vec![0.0; NWIN],
            wpos: 0,
            filled: 0,
            since_hop: 0,
            fft,
            fft_in: vec![0.0; NFFT],
            spec: vec![Complex::default(); nbins],
            mags: vec![0.0; nbins],
            phases: vec![0.0; nbins],
            prev_phases: vec![0.0; nbins],
            has_prev: false,
            bin_lo,
            bin_hi,
            peaks: Vec::with_capacity(MAX_PEAKS),
            rms: 0.0,
        }
    }

    pub fn mags(&self) -> &[f32] {
        &self.mags
    }

    pub fn bin_hz(&self) -> f32 {
        self.fs / NFFT as f32
    }

    /// Feed one sample; returns true when a new analysis frame (self.peaks) is ready.
    pub fn feed(&mut self, sample: f32) -> bool {
        self.ring[self.wpos] = sample;
        self.wpos = (self.wpos + 1) % NWIN;
        self.filled = (self.filled + 1).min(NWIN);
        self.since_hop += 1;
        if self.filled < NWIN || self.since_hop < HOP {
            return false;
        }
        self.since_hop = 0;
        self.analyze();
        true
    }

    fn analyze(&mut self) {
        let mut sq = 0.0f32;
        for i in 0..NWIN {
            let s = self.ring[(self.wpos + i) % NWIN];
            sq += s * s;
            self.fft_in[i] = s * self.window[i];
        }
        self.rms = (sq / NWIN as f32).sqrt();
        self.fft_in[NWIN..].fill(0.0);
        let _ = self.fft.process(&mut self.fft_in, &mut self.spec);

        let norm = 2.0 / self.wsum;
        for i in 0..self.spec.len() {
            self.mags[i] = self.spec[i].norm() * norm;
        }
        for i in self.bin_lo..=self.bin_hi {
            self.phases[i] = self.spec[i].im.atan2(self.spec[i].re);
        }

        self.find_peaks();
        std::mem::swap(&mut self.phases, &mut self.prev_phases);
        self.has_prev = true;
    }

    fn find_peaks(&mut self) {
        self.peaks.clear();
        // no neighborhood-mean noise floor here: with many strings sounding,
        // nearby mainlobes inflate any local average and real partials get
        // rejected; the relative-to-max cut plus the peak cap is enough
        let max_mag = self.mags[self.bin_lo..=self.bin_hi]
            .iter()
            .fold(0.0f32, |a, &b| a.max(b));
        let thresh = (max_mag * 2e-3).max(1e-5);
        let mut raw: Vec<Peak> = Vec::new();
        for i in self.bin_lo..=self.bin_hi {
            let m = self.mags[i];
            if m < thresh {
                continue;
            }
            let local_max = (1..=3).all(|d| m >= self.mags[i + d] && m > self.mags[i - d]);
            if !local_max {
                continue;
            }
            raw.push(Peak {
                freq: self.refine_freq(i),
                mag: m,
            });
        }
        raw.sort_by(|a, b| b.mag.total_cmp(&a.mag));
        raw.truncate(MAX_PEAKS);
        raw.sort_by(|a, b| a.freq.total_cmp(&b.freq));
        self.peaks = raw;
    }

    fn refine_freq(&self, i: usize) -> f32 {
        let (a, b, c) = (
            self.mags[i - 1].max(1e-12).ln(),
            self.mags[i].max(1e-12).ln(),
            self.mags[i + 1].max(1e-12).ln(),
        );
        let denom = a - 2.0 * b + c;
        let delta = if denom.abs() > 1e-9 {
            (0.5 * (a - c) / denom).clamp(-1.0, 1.0)
        } else {
            0.0
        };

        if self.has_prev {
            let expected = 2.0 * PI * HOP as f32 * i as f32 / NFFT as f32;
            let mut dphi = self.phases[i] - self.prev_phases[i] - expected;
            dphi -= 2.0 * PI * (dphi / (2.0 * PI)).round();
            let dev_bins = dphi * NFFT as f32 / (2.0 * PI * HOP as f32);
            if (dev_bins - delta).abs() < 1.5 {
                return (i as f32 + dev_bins) * self.fs / NFFT as f32;
            }
        }
        (i as f32 + delta) * self.fs / NFFT as f32
    }
}
