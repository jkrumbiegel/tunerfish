use tunerfish::engine::Engine;

const FS: f32 = 48000.0;

fn cents(a: f32, b: f32) -> f32 {
    1200.0 * (a / b).log2()
}

struct Pluck {
    freq: f32,
    amp: f32,
    onset_cents: f32, // starts this sharp, settles exponentially
}

fn synth(plucks: &[Pluck], seconds: f32) -> Vec<f32> {
    let n = (seconds * FS) as usize;
    let mut out = vec![0.0f32; n];
    let mut rng = 12345u64;
    let mut rand = move || {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (rng >> 33) as f32 / (1u64 << 31) as f32 - 0.5
    };
    for p in plucks {
        // f64 phase accumulators wrapped to [0, tau): f32 accumulation drifts
        // the effective frequency by more than a cent over a second
        let mut phase: Vec<f64> = (0..6).map(|_| (rand() * std::f32::consts::TAU) as f64).collect();
        for i in 0..n {
            let t = i as f32 / FS;
            let detune = p.onset_cents * (-t / 0.3).exp();
            let f = (p.freq * (detune / 1200.0).exp2()) as f64;
            let decay = (-t / 4.0).exp();
            let mut s = 0.0;
            for h in 1..=6usize {
                phase[h - 1] = (phase[h - 1] + std::f64::consts::TAU * f * h as f64 / FS as f64)
                    .rem_euclid(std::f64::consts::TAU);
                s += phase[h - 1].sin() as f32 * p.amp * decay / h as f32;
            }
            out[i] += s;
        }
    }
    for s in &mut out {
        *s += rand() * 1e-4;
    }
    out
}

fn run(samples: &[f32]) -> Engine {
    let mut e = Engine::new(FS);
    e.push(samples);
    e
}

#[test]
fn silence_yields_no_tracks() {
    let samples = vec![0.0f32; FS as usize];
    let e = run(&samples);
    assert_eq!(e.active_tracks().len(), 0);
}

#[test]
fn single_string_accurate_to_one_cent() {
    let samples = synth(
        &[Pluck { freq: 196.0, amp: 0.2, onset_cents: 0.0 }],
        1.5,
    );
    let e = run(&samples);
    let tracks = e.active_tracks();
    assert_eq!(tracks.len(), 1, "tracks: {tracks:?}");
    assert!(
        cents(tracks[0].1, 196.0).abs() < 1.0,
        "off by {} cents",
        cents(tracks[0].1, 196.0)
    );
}

#[test]
fn onset_settle_is_followed() {
    let samples = synth(
        &[Pluck { freq: 110.0, amp: 0.3, onset_cents: 25.0 }],
        2.0,
    );
    let e = run(&samples);
    let tracks = e.active_tracks();
    assert_eq!(tracks.len(), 1, "tracks: {tracks:?}");
    assert!(
        cents(tracks[0].1, 110.0).abs() < 2.0,
        "did not settle, off by {} cents",
        cents(tracks[0].1, 110.0)
    );
}

#[test]
fn bass_low_e_detected() {
    let samples = synth(
        &[Pluck { freq: 41.2, amp: 0.3, onset_cents: 0.0 }],
        2.0,
    );
    let e = run(&samples);
    let tracks = e.active_tracks();
    assert_eq!(tracks.len(), 1, "tracks: {tracks:?}");
    assert!(
        cents(tracks[0].1, 41.2).abs() < 2.0,
        "off by {} cents",
        cents(tracks[0].1, 41.2)
    );
}

#[test]
fn glide_stays_one_track() {
    // sung-style slide: 220 Hz down to 165 Hz over 1.5 s, then held
    let n = (2.5 * FS) as usize;
    let mut samples = vec![0.0f32; n];
    let mut phase = [0.0f64; 5];
    for (i, out) in samples.iter_mut().enumerate() {
        let t = i as f32 / FS;
        let f = if t < 1.5 {
            220.0 * (165.0f32 / 220.0).powf(t / 1.5)
        } else {
            165.0
        } as f64;
        let mut s = 0.0;
        for h in 1..=5usize {
            phase[h - 1] = (phase[h - 1] + std::f64::consts::TAU * f * h as f64 / FS as f64)
                .rem_euclid(std::f64::consts::TAU);
            s += phase[h - 1].sin() as f32 * 0.25 / h as f32;
        }
        *out = s;
    }
    let e = run(&samples);
    let tracks = e.active_tracks();
    assert_eq!(tracks.len(), 1, "tracks: {tracks:?}");
    assert!(
        cents(tracks[0].1, 165.0).abs() < 2.0,
        "off by {} cents",
        cents(tracks[0].1, 165.0)
    );
    assert!(
        e.tracks_created() <= 2,
        "glide spawned {} tracks",
        e.tracks_created()
    );
}

#[test]
fn full_strum_standard_tuning() {
    let strings = [82.407, 110.0, 146.832, 195.998, 246.942, 329.628];
    let plucks: Vec<Pluck> = strings
        .iter()
        .map(|&f| Pluck { freq: f, amp: 0.15, onset_cents: 0.0 })
        .collect();
    let samples = synth(&plucks, 2.0);
    let e = run(&samples);
    let tracks = e.active_tracks();

    // every track must correspond to a real string (no ghosts)
    for &(id, f, _) in &tracks {
        let nearest = strings
            .iter()
            .map(|&s| cents(f, s).abs())
            .fold(f32::INFINITY, f32::min);
        assert!(nearest < 5.0, "ghost track {id} at {f} Hz, {nearest} cents from any string");
    }

    // the four lower strings have unshadowed partials and must all be found
    for &s in &strings[..4] {
        let found = tracks.iter().any(|&(_, f, _)| cents(f, s).abs() < 4.0);
        assert!(found, "string at {s} Hz not detected; tracks: {tracks:?}");
    }
    assert!(tracks.len() >= 4, "tracks: {tracks:?}");
}
