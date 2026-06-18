use crate::analyzer::Peak;
use crate::pitch::Candidate;

pub const F0_MIN: f32 = 27.5;
pub const F0_MAX: f32 = 1050.0;
pub const BIN_CENTS: f32 = 10.0;

const H_MAX: usize = 16;
const HARM_DECAY: f32 = 0.8;
const FREQ_HI: f32 = 6000.0;
const WHITEN_HALF: usize = 60; // spectral bins each side for the broadband-floor estimate
const TAU_SECONDS: f32 = 0.25; // how far back the spectrogram integrates
const K_MAX: usize = 8; // most simultaneous pitches extracted per frame
const SUBTRACT_RETAIN: f32 = 0.1; // leftover energy at a claimed harmonic
const SUB_HALFWIN: usize = 6; // bins cleared each side of a claimed harmonic
const MIN_SALIENCE: f32 = 0.04; // scale-invariant tonal-energy fraction to count a peak
const REL_PEAK: f32 = 0.12; // ...and it must reach this fraction of the strongest peak
const DEDUP_CENTS: f32 = 40.0;
const REFINE_TOL_CENTS: f32 = 35.0;

/// Treats the recent spectrogram as a 2D field. Each frame it whitens the
/// spectrum (removing broadband noise) and integrates it over time, then
/// extracts pitches by iterated harmonic-salience picking with spectral
/// subtraction: the strongest harmonic stack is claimed and its partials
/// removed before the next, so subharmonics and octaves of an already-claimed
/// pitch cannot win, while genuinely separate notes survive. Temporal
/// integration removes the frame-wise jitter that plagued per-frame picking.
pub struct Salience {
    bin_hz: f32,
    alpha: f32,
    nb: usize,
    n_bins: usize,
    f0s: Vec<f32>,
    wbar: Vec<f32>, // time-integrated whitened spectrum
    white: Vec<f32>,
    prefix: Vec<f32>,
    residual: Vec<f32>,
    sal: Vec<f32>, // exposed salience map (first extraction pass)
}

impl Salience {
    pub fn new(fs: f32, dt: f32) -> Self {
        let span_cents = 1200.0 * (F0_MAX / F0_MIN).log2();
        let nb = (span_cents / BIN_CENTS) as usize + 1;
        let f0s: Vec<f32> = (0..nb)
            .map(|k| F0_MIN * (k as f32 * BIN_CENTS / 1200.0).exp2())
            .collect();
        let bin_hz = fs / crate::analyzer::NFFT as f32;
        let n_bins = (FREQ_HI / bin_hz) as usize + 2;
        Salience {
            bin_hz,
            alpha: (-dt / TAU_SECONDS).exp(),
            nb,
            n_bins,
            f0s,
            wbar: vec![0.0; n_bins],
            white: vec![0.0; n_bins],
            prefix: Vec::new(),
            residual: vec![0.0; n_bins],
            sal: vec![0.0; nb],
        }
    }

    pub fn map(&self) -> &[f32] {
        &self.sal
    }

    pub fn decay(&mut self) {
        for w in &mut self.wbar {
            *w *= self.alpha;
        }
        for s in &mut self.sal {
            *s *= self.alpha;
        }
    }

    pub fn process(&mut self, mags: &[f32], peaks: &[Peak], out: &mut Vec<Candidate>) {
        out.clear();
        self.integrate(mags);

        self.residual.copy_from_slice(&self.wbar);
        let inv_total = 1.0 / self.wbar.iter().sum::<f32>().max(1e-12);

        let mut first_max = 0.0f32;
        for iter in 0..K_MAX {
            self.compute_salience(inv_total);
            if iter == 0 {
                // expose the full first-pass map for visualisation
                first_max = self.sal.iter().copied().fold(0.0, f32::max);
            }
            let Some((k, smax)) = self.best_peak() else { break };
            if smax < MIN_SALIENCE.max(REL_PEAK * first_max) {
                break;
            }
            let f0_grid = self.interp_f0(k);
            if out
                .iter()
                .all(|c| (1200.0 * (f0_grid / c.f0).log2()).abs() > DEDUP_CENTS)
            {
                out.push(Candidate {
                    f0: refine(peaks, f0_grid),
                    salience: smax,
                });
            }
            self.subtract(f0_grid);
        }
    }

    fn integrate(&mut self, mags: &[f32]) {
        let n = self.n_bins.min(mags.len());
        self.prefix.resize(mags.len() + 1, 0.0);
        self.prefix[0] = 0.0;
        for i in 0..mags.len() {
            self.prefix[i + 1] = self.prefix[i] + mags[i];
        }
        for i in 0..n {
            let lo = i.saturating_sub(WHITEN_HALF);
            let hi = (i + WHITEN_HALF + 1).min(mags.len());
            let floor = (self.prefix[hi] - self.prefix[lo]) / (hi - lo) as f32;
            self.white[i] = (mags[i] - floor).max(0.0);
            self.wbar[i] = self.alpha * self.wbar[i] + (1.0 - self.alpha) * self.white[i];
        }
    }

    fn compute_salience(&mut self, inv_total: f32) {
        for k in 0..self.nb {
            let f0 = self.f0s[k];
            let mut s = 0.0;
            let mut hw = 1.0f32;
            for h in 1..=H_MAX {
                let f = f0 * h as f32;
                if f > FREQ_HI {
                    break;
                }
                let pos = f / self.bin_hz;
                let b = pos as usize;
                if b + 1 < self.n_bins {
                    let frac = pos - b as f32;
                    s += hw * (self.residual[b] * (1.0 - frac) + self.residual[b + 1] * frac);
                }
                hw *= HARM_DECAY;
            }
            self.sal[k] = s * inv_total;
        }
    }

    fn best_peak(&self) -> Option<(usize, f32)> {
        let mut best = None;
        let mut bv = 0.0f32;
        for k in 2..self.nb - 2 {
            let s = self.sal[k];
            if s > bv
                && s >= self.sal[k - 1]
                && s > self.sal[k + 1]
                && s >= self.sal[k - 2]
                && s > self.sal[k + 2]
            {
                bv = s;
                best = Some(k);
            }
        }
        best.map(|k| (k, bv))
    }

    fn interp_f0(&self, k: usize) -> f32 {
        let (a, b, c) = (self.sal[k - 1], self.sal[k], self.sal[k + 1]);
        let denom = a - 2.0 * b + c;
        let delta = if denom.abs() > 1e-9 {
            (0.5 * (a - c) / denom).clamp(-1.0, 1.0)
        } else {
            0.0
        };
        F0_MIN * ((k as f32 + delta) * BIN_CENTS / 1200.0).exp2()
    }

    fn subtract(&mut self, f0: f32) {
        for h in 1..=H_MAX {
            let f = f0 * h as f32;
            if f > FREQ_HI {
                break;
            }
            let center = (f / self.bin_hz) as usize;
            let lo = center.saturating_sub(SUB_HALFWIN);
            let hi = (center + SUB_HALFWIN + 1).min(self.n_bins);
            for b in lo..hi {
                self.residual[b] *= SUBTRACT_RETAIN;
            }
        }
    }
}

/// Magnitude-weighted f0 from the spectral peaks matching the grid estimate's
/// harmonics, recovering sub-cent precision the 10-cent salience grid lacks.
fn refine(peaks: &[Peak], f0_grid: f32) -> f32 {
    let mut num = 0.0;
    let mut den = 0.0;
    for h in 1..=6 {
        let target = f0_grid * h as f32;
        if let Some(p) = nearest(peaks, target, REFINE_TOL_CENTS) {
            num += p.mag * p.freq / h as f32;
            den += p.mag;
        }
    }
    if den > 0.0 {
        num / den
    } else {
        f0_grid
    }
}

fn nearest<'a>(peaks: &'a [Peak], target: f32, tol_cents: f32) -> Option<&'a Peak> {
    let i = peaks.partition_point(|p| p.freq < target);
    let mut best = None;
    let mut best_d = tol_cents;
    for j in [i.wrapping_sub(1), i] {
        if let Some(p) = peaks.get(j) {
            let d = (1200.0 * (p.freq / target).log2()).abs();
            if d < best_d {
                best_d = d;
                best = Some(p);
            }
        }
    }
    best
}
