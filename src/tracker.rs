use crate::pitch::Candidate;

const GATE_CENTS: f32 = 80.0;
const REACQUIRE_CENTS: f32 = 150.0;
const REVIVE_CENTS: f32 = 50.0;
const REVIVE_SECONDS: f32 = 10.0;
const DROP_SECONDS: f32 = 15.0;
const ACTIVATE_HITS: u32 = 4;
const HOLD_MISSES: u32 = 2;
const DEAD_MISSES: u32 = 8;
const JITTER_MAX: f32 = 18.0; // cents; a track this unsteady is noise, don't show it
const JITTER_INIT: f32 = 40.0; // new tracks start "unsteady" and must prove otherwise
const ACCEL_SIGMA: f32 = 200.0; // cents/s^2 process noise
const VEL_DAMP: f32 = 0.9;
const MEAS_SIGMA: f32 = 2.0; // cents, at full confidence

fn cents_of(freq: f32) -> f32 {
    1200.0 * (freq / 440.0).log2()
}

#[derive(Clone, Debug)]
pub struct Track {
    pub id: u32,
    pub cents: f32,
    pub vel: f32,
    p: [[f32; 2]; 2],
    pub conf: f32,
    pub hits: u32,
    pub misses: u32,
    pub last_seen: f32,
    pub born: f32,
    jitter: f32,
}

impl Track {
    pub fn freq(&self) -> f32 {
        440.0 * (self.cents / 1200.0).exp2()
    }

    pub fn active(&self) -> bool {
        self.hits >= ACTIVATE_HITS && self.misses <= HOLD_MISSES && self.jitter < JITTER_MAX
    }

    fn dead(&self) -> bool {
        self.misses > DEAD_MISSES
    }

    fn predict(&mut self, dt: f32) {
        self.cents += self.vel * dt;
        self.vel *= VEL_DAMP;
        let q = ACCEL_SIGMA * ACCEL_SIGMA;
        let (dt2, dt3, dt4) = (dt * dt, dt * dt * dt, dt * dt * dt * dt);
        let [[p00, p01], [p10, p11]] = self.p;
        self.p = [
            [
                p00 + dt * (p10 + p01) + dt2 * p11 + q * dt4 / 4.0,
                VEL_DAMP * (p01 + dt * p11) + q * dt3 / 2.0,
            ],
            [
                VEL_DAMP * (p10 + dt * p11) + q * dt3 / 2.0,
                VEL_DAMP * VEL_DAMP * p11 + q * dt2,
            ],
        ];
    }

    fn correct(&mut self, meas_cents: f32, r: f32) {
        let s = self.p[0][0] + r;
        let k0 = self.p[0][0] / s;
        let k1 = self.p[1][0] / s;
        let innov = meas_cents - self.cents;
        self.jitter = 0.7 * self.jitter + 0.3 * innov.abs();
        self.cents += k0 * innov;
        self.vel += k1 * innov;
        let [[p00, p01], [_, p11]] = self.p;
        self.p = [
            [(1.0 - k0) * p00, (1.0 - k0) * p01],
            [self.p[1][0] - k1 * p00, p11 - k1 * p01],
        ];
    }
}

pub struct Bank {
    pub tracks: Vec<Track>,
    next_id: u32,
    dt: f32,
}

impl Bank {
    pub fn new(dt: f32) -> Self {
        Bank {
            tracks: Vec::new(),
            next_id: 1,
            dt,
        }
    }

    pub fn active(&self) -> impl Iterator<Item = &Track> {
        self.tracks.iter().filter(|t| t.active())
    }

    pub fn tracks_created(&self) -> u32 {
        self.next_id - 1
    }

    pub fn update(&mut self, candidates: &[Candidate], now: f32) {
        for t in &mut self.tracks {
            if !t.dead() {
                t.predict(self.dt);
            }
        }

        let best_sal = candidates
            .iter()
            .map(|c| c.salience)
            .fold(0.0f32, f32::max);

        let mut order: Vec<usize> = (0..candidates.len()).collect();
        order.sort_by(|&a, &b| candidates[b].salience.total_cmp(&candidates[a].salience));

        let mut claimed = vec![false; self.tracks.len()];
        let mut unmatched: Vec<usize> = Vec::new();

        for ci in order {
            let c = &candidates[ci];
            let mc = cents_of(c.f0);
            let mut best: Option<(usize, f32)> = None;
            for (ti, t) in self.tracks.iter().enumerate() {
                if claimed[ti] || t.dead() {
                    continue;
                }
                let d = (mc - t.cents).abs();
                if d < GATE_CENTS && best.map_or(true, |(_, bd)| d < bd) {
                    best = Some((ti, d));
                }
            }
            if let Some((ti, _)) = best {
                claimed[ti] = true;
                let t = &mut self.tracks[ti];
                let conf_ratio = (c.salience / best_sal).max(0.05);
                let r = (MEAS_SIGMA * MEAS_SIGMA) * (1.0 + 0.5 / conf_ratio);
                t.correct(mc, r);
                t.conf = 0.7 * t.conf + 0.3 * c.salience;
                t.hits += 1;
                t.misses = 0;
                t.last_seen = now;
            } else {
                unmatched.push(ci);
            }
        }

        // re-acquire pass: a fast slide or vibrato can jump past the gate;
        // snap a just-lost track to the nearby candidate instead of
        // letting it die while a fresh track spawns next to it
        let mut spawnable: Vec<usize> = Vec::new();
        for ci in unmatched {
            let c = &candidates[ci];
            let mc = cents_of(c.f0);
            let mut best: Option<(usize, f32)> = None;
            for (ti, t) in self.tracks.iter().enumerate() {
                if claimed[ti] || t.dead() || t.hits < ACTIVATE_HITS {
                    continue;
                }
                let d = (mc - t.cents).abs();
                if d < REACQUIRE_CENTS && best.map_or(true, |(_, bd)| d < bd) {
                    best = Some((ti, d));
                }
            }
            if let Some((ti, _)) = best {
                claimed[ti] = true;
                let t = &mut self.tracks[ti];
                t.cents = mc;
                t.vel = 0.0;
                t.p = [[16.0, 0.0], [0.0, 900.0]];
                t.conf = 0.7 * t.conf + 0.3 * c.salience;
                t.hits += 1;
                t.misses = 0;
                t.last_seen = now;
            } else {
                spawnable.push(ci);
            }
        }

        for (ti, t) in self.tracks.iter_mut().enumerate() {
            if !claimed[ti] && !t.dead() {
                t.misses += 1;
                t.conf *= 0.7;
            }
        }

        for ci in spawnable {
            let c = &candidates[ci];
            let mc = cents_of(c.f0);
            if c.salience < (0.15 * best_sal).max(1e-4) {
                continue;
            }
            if let Some(t) = self.tracks.iter_mut().find(|t| {
                t.dead() && (now - t.last_seen) < REVIVE_SECONDS && (mc - t.cents).abs() < REVIVE_CENTS
            }) {
                t.cents = mc;
                t.vel = 0.0;
                t.p = [[16.0, 0.0], [0.0, 400.0]];
                t.conf = c.salience;
                t.hits = ACTIVATE_HITS; // a re-pluck resumes its line immediately
                t.misses = 0;
                t.jitter = 0.0;
                t.last_seen = now;
                continue;
            }
            if self.is_harmonic_ghost(mc, c.salience) {
                continue;
            }
            self.tracks.push(Track {
                id: self.next_id,
                cents: mc,
                vel: 0.0,
                p: [[25.0, 0.0], [0.0, 900.0]],
                conf: c.salience,
                hits: 1,
                misses: 0,
                last_seen: now,
                born: now,
                jitter: JITTER_INIT,
            });
            self.next_id += 1;
        }

        // one-frame flukes die immediately; dead tracks linger for revival, then drop
        self.tracks
            .retain(|t| !(t.hits < 2 && t.misses > 0) && !(t.dead() && now - t.last_seen > DROP_SECONDS));
    }

    /// Suppress spawning at an integer frequency ratio of a stronger live
    /// track: those are usually leaked harmonics/subharmonics, not strings.
    /// Octaves share every partial so leakage is strong; at 3x only every
    /// third partial is shared, so a real string there clears a lower bar.
    fn is_harmonic_ghost(&self, mc: f32, salience: f32) -> bool {
        for t in self.tracks.iter().filter(|t| !t.dead()) {
            for (h, thresh) in [(2.0f32, 0.45f32), (3.0, 0.18), (4.0, 0.45)] {
                let interval = 1200.0 * h.log2();
                let above = (mc - t.cents - interval).abs() < 35.0;
                let below = (t.cents - mc - interval).abs() < 35.0;
                if (above && salience < thresh * t.conf) || (below && salience < 0.6 * t.conf) {
                    return true;
                }
            }
        }
        false
    }
}
