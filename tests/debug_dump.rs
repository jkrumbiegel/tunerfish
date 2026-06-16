use tunerfish::analyzer::Analyzer;
use tunerfish::engine::Engine;
use tunerfish::pitch;

const FS: f32 = 48000.0;

fn strum(seconds: f32) -> Vec<f32> {
    let strings = [82.407f64, 110.0, 146.832, 195.998, 246.942, 329.628];
    let n = (seconds * FS) as usize;
    let mut out = vec![0.0f32; n];
    for &f in &strings {
        let mut phase = [0.0f64; 6];
        for i in 0..n {
            let t = i as f32 / FS;
            let decay = (-t / 4.0f32).exp();
            let mut s = 0.0;
            for h in 1..=6usize {
                phase[h - 1] = (phase[h - 1] + std::f64::consts::TAU * f * h as f64 / FS as f64)
                    .rem_euclid(std::f64::consts::TAU);
                s += phase[h - 1].sin() as f32 * 0.15 * decay / h as f32;
            }
            out[i] += s;
        }
    }
    out
}

#[test]
#[ignore]
fn dump_strum() {
    let samples = strum(2.0);
    let mut engine = Engine::new(FS);
    let mut analyzer = Analyzer::new(FS);
    let mut cands = Vec::new();
    let mut frame = 0;
    for &s in &samples {
        engine.push(&[s]);
        if analyzer.feed(s) {
            frame += 1;
            if frame % 4 != 1 {
                continue;
            }
            pitch::detect(&analyzer.peaks, &mut cands);
            let cs: Vec<String> = cands
                .iter()
                .map(|c| format!("{:.2}({:.3})", c.f0, c.salience))
                .collect();
            println!("f{frame} cands: {}", cs.join(" "));
            if frame == 17 {
                let ps: Vec<String> = analyzer
                    .peaks
                    .iter()
                    .map(|p| format!("{:.2}({:.4})", p.freq, p.mag))
                    .collect();
                println!("f17 peaks: {}", ps.join(" "));
            }
            let ts: Vec<String> = engine
                .active_tracks()
                .iter()
                .map(|&(id, f, conf)| format!("#{id}:{f:.2}({conf:.3})"))
                .collect();
            println!("f{frame} tracks: {}", ts.join(" "));
        }
    }
}
