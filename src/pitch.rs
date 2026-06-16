use crate::analyzer::Peak;

pub const F0_MIN: f32 = 27.0;
pub const F0_MAX: f32 = 620.0;
const MAX_HARMONIC: usize = 10;
const MAX_CANDIDATES: usize = 10;
const SEED_HARMONICS: usize = 5;
const SUBTRACT_RETAIN: f32 = 0.3;

#[derive(Clone, Copy, Debug)]
pub struct Candidate {
    pub f0: f32,
    pub salience: f32,
}

fn cents(a: f32, b: f32) -> f32 {
    1200.0 * (a / b).log2()
}

fn tol_cents(h: usize) -> f32 {
    30.0 + 1.2 * (h * h) as f32
}

/// Nearest peak to `target` within tolerance, by cents distance.
fn match_peak(peaks: &[Peak], target: f32, tol: f32) -> Option<(usize, f32)> {
    let i = peaks.partition_point(|p| p.freq < target);
    let mut best: Option<(usize, f32)> = None;
    for j in [i.wrapping_sub(1), i, i + 1] {
        if let Some(p) = peaks.get(j) {
            let d = cents(p.freq, target).abs();
            if d < tol && best.map_or(true, |(_, bd)| d < bd) {
                best = Some((j, d));
            }
        }
    }
    best
}

/// Gaussian closeness weight: a harmonic stack that is offset from the
/// peaks it matches scores far less than one that nails them, otherwise a
/// false f0 (e.g. another string's partial / 5) can outscore the true one
/// by riding the same peaks within tolerance. Sigma widens with h to leave
/// room for string inharmonicity.
fn closeness(d_cents: f32, h: usize) -> f32 {
    let sigma = 8.0 + 0.8 * (h * h) as f32;
    (-0.5 * (d_cents / sigma) * (d_cents / sigma)).exp()
}

struct Scored {
    f0: f32,
    salience: f32,
    matched: Vec<(usize, f32)>,
}

fn score(peaks: &[Peak], mags: &[f32], f0: f32) -> Option<Scored> {
    let mut salience = 0.0;
    let mut matched = Vec::new();
    let mut harmonics = [false; MAX_HARMONIC + 1];
    for h in 1..=MAX_HARMONIC {
        let target = h as f32 * f0;
        if let Some((j, d)) = match_peak(peaks, target, tol_cents(h)) {
            let cl = closeness(d, h);
            salience += mags[j] * cl / (h as f32).powf(0.7);
            matched.push((j, cl));
            harmonics[h] = true;
        }
    }
    if !(harmonics[1] || (harmonics[2] && harmonics[3])) || matched.len() < 2 {
        return None;
    }
    Some(Scored {
        f0: refine_f0(peaks, mags, f0, &matched),
        salience,
        matched,
    })
}

fn weighted_median(pts: &[(f32, f32, f32)]) -> f32 {
    let mut sorted: Vec<(f32, f32)> = pts.iter().map(|&(_, y, w)| (y, w)).collect();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
    let half = 0.5 * sorted.iter().map(|&(_, w)| w).sum::<f32>();
    let mut acc = 0.0;
    for &(y, w) in &sorted {
        acc += w;
        if acc >= half {
            return y;
        }
    }
    sorted.last().map_or(0.0, |&(y, _)| y)
}

/// Weighted fit of f_h = h*f0*sqrt(1 + B*h^2) over matched harmonics,
/// linearized as y_h = f_h/h = f0 + (f0*B/2)*h^2. Harmonics far from the
/// weighted median of y_h are trimmed first: a partial shared with another
/// string reports a blended frequency and would drag the fit. Falls back to
/// a weighted mean when the fit is degenerate or B is implausible.
fn refine_f0(peaks: &[Peak], mags: &[f32], f0: f32, matched: &[(usize, f32)]) -> f32 {
    let mut pts: Vec<(f32, f32, f32)> = Vec::new(); // (h, y, w)
    for &(j, cl) in matched {
        let h = (peaks[j].freq / f0).round();
        if h < 1.0 {
            continue;
        }
        pts.push((h, peaks[j].freq / h, mags[j] * cl));
    }
    if pts.len() >= 2 {
        // median weighted by mag*closeness only: low harmonics are the
        // trustworthy ones, h^2 weighting would let one contaminated high
        // harmonic flip the median and trim the clean points instead
        let med = weighted_median(&pts);
        pts.retain(|&(_, y, _)| cents(y, med).abs() < 8.0);
    }
    for p in &mut pts {
        p.2 *= p.0 * p.0; // fit weight mag*cl*h^2: cents precision scales with h
    }
    if pts.len() >= 3 {
        let (mut sw, mut sx, mut sy, mut sxx, mut sxy) = (0.0f32, 0.0, 0.0, 0.0, 0.0);
        for &(h, y, w) in &pts {
            let x = h * h;
            sw += w;
            sx += w * x;
            sy += w * y;
            sxx += w * x * x;
            sxy += w * x * y;
        }
        let denom = sw * sxx - sx * sx;
        if denom.abs() > 1e-6 {
            let c = (sw * sxy - sx * sy) / denom;
            let a = (sy - c * sx) / sw;
            let b_inharm = 2.0 * c / a;
            if a > 0.0 && (0.0..2e-3).contains(&b_inharm) {
                return a;
            }
        }
    }
    let mut sw = 0.0f32;
    let mut sy = 0.0f32;
    for &(h, y, w) in &pts {
        let w = if h <= 5.0 { w / h } else { 0.0 }; // weight mag*h, drop high h
        sw += w;
        sy += w * y;
    }
    if sw > 0.0 {
        sy / sw
    } else {
        f0
    }
}

/// Iterative polyphonic f0 estimation: repeatedly pick the highest-salience
/// harmonic stack, then attenuate its claimed peaks so weaker concurrent
/// strings can surface.
pub fn detect(peaks: &[Peak], out: &mut Vec<Candidate>) {
    out.clear();
    if peaks.is_empty() {
        return;
    }

    let mut seeds: Vec<f32> = Vec::new();
    for p in peaks {
        for h in 1..=SEED_HARMONICS {
            let f0 = p.freq / h as f32;
            if (F0_MIN..=F0_MAX).contains(&f0) {
                seeds.push(f0);
            }
        }
    }
    seeds.sort_by(f32::total_cmp);
    seeds.dedup_by(|a, b| cents(*a, *b).abs() < 20.0);

    let mut mags: Vec<f32> = peaks.iter().map(|p| p.mag).collect();
    let mut first_best = 0.0f32;

    while out.len() < MAX_CANDIDATES {
        let mut best: Option<Scored> = None;
        for &seed in &seeds {
            if out.iter().any(|c| cents(seed, c.f0).abs() < 35.0) {
                continue;
            }
            if let Some(s) = score(peaks, &mags, seed) {
                if best.as_ref().map_or(true, |b| s.salience > b.salience) {
                    best = Some(s);
                }
            }
        }
        let Some(best) = best else { break };
        if out.is_empty() {
            first_best = best.salience;
        } else if best.salience < (0.05 * first_best).max(5e-5) {
            break;
        }
        for &(j, _) in &best.matched {
            mags[j] *= SUBTRACT_RETAIN;
        }
        out.push(Candidate {
            f0: best.f0,
            salience: best.salience,
        });
    }
}
