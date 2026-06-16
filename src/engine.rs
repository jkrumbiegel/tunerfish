use crate::analyzer::{Analyzer, HOP};
use crate::pitch::{self, Candidate};
use crate::tracker::Bank;

const RMS_GATE: f32 = 2e-4;
pub const MAX_OUT_TRACKS: usize = 16;
pub const OUT_STRIDE: usize = 6;
pub const OUT_HEADER: usize = 4;
pub const OUT_LEN: usize = OUT_HEADER + MAX_OUT_TRACKS * OUT_STRIDE;

pub struct Engine {
    analyzer: Analyzer,
    bank: Bank,
    candidates: Vec<Candidate>,
    frames: u64,
    fs: f32,
}

impl Engine {
    pub fn new(fs: f32) -> Self {
        Engine {
            analyzer: Analyzer::new(fs),
            bank: Bank::new(HOP as f32 / fs),
            candidates: Vec::new(),
            frames: 0,
            fs,
        }
    }

    pub fn time(&self) -> f32 {
        self.frames as f32 * HOP as f32 / self.fs
    }

    pub fn rms(&self) -> f32 {
        self.analyzer.rms
    }

    /// Returns the number of analysis frames produced by this chunk.
    pub fn push(&mut self, samples: &[f32]) -> usize {
        let mut produced = 0;
        for &s in samples {
            if self.analyzer.feed(s) {
                self.frames += 1;
                if self.analyzer.rms > RMS_GATE {
                    pitch::detect(&self.analyzer.peaks, &mut self.candidates);
                } else {
                    self.candidates.clear();
                }
                self.bank.update(&self.candidates, self.time());
                produced += 1;
            }
        }
        produced
    }

    pub fn tracks_created(&self) -> u32 {
        self.bank.tracks_created()
    }

    pub fn active_tracks(&self) -> Vec<(u32, f32, f32)> {
        self.bank
            .active()
            .map(|t| (t.id, t.freq(), t.conf))
            .collect()
    }

    /// Layout: [n_tracks, time_s, rms, n_candidates] then per track:
    /// [id, 1.0, freq_hz, conf, vel_cents_per_s, age_s]
    pub fn write_out(&self, buf: &mut [f32; OUT_LEN]) {
        let now = self.time();
        let mut n = 0;
        for t in self.bank.active().take(MAX_OUT_TRACKS) {
            let base = OUT_HEADER + n * OUT_STRIDE;
            buf[base] = t.id as f32;
            buf[base + 1] = 1.0;
            buf[base + 2] = t.freq();
            buf[base + 3] = t.conf;
            buf[base + 4] = t.vel;
            buf[base + 5] = now - t.born;
            n += 1;
        }
        buf[0] = n as f32;
        buf[1] = now;
        buf[2] = self.analyzer.rms;
        buf[3] = self.candidates.len() as f32;
    }
}
