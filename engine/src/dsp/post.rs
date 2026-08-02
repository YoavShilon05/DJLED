//! Post-processing: raw band magnitudes to 0..1 brightness.
//!
//! Order matters here. Floor subtraction happens in the linear domain (it is a
//! subtraction of *energy*), everything after it in dB (where tilt and AGC are
//! plain additions, and where perception lives).
//!
//!   1. noise floor tracking + subtraction   — kills idle shimmer
//!   2. linear -> dB                          — perception is logarithmic
//!   3. spectral tilt                         — taste knob for overall slope
//!   4. per-band AGC                          — keeps quiet bands readable
//!   5. range map to 0..1
//!   6. asymmetric ballistics                 — fast attack, slow release
//!   7. spatial smoothing                     — de-jitter without blurring bars

use super::biquad::coefficient;

/// Initial floor estimate as a fraction of the first observed magnitude, i.e.
/// 40 dB down. Combined with the default creep rate this gives a sustained tone
/// roughly four minutes before the floor could climb to meet it — far longer
/// than any real note is held, while still letting the floor find true silence
/// within a few frames once the music stops.
const FLOOR_SEED_FRACTION: f32 = 0.01;

#[derive(Clone, Debug)]
pub struct PostConfig {
    /// Overall spectral slope, dB per octave. Defaults to 0: summing power over
    /// log-spaced bands is already flat for pink noise, and music is roughly
    /// pink, so no correction is needed by default. Positive values lift treble.
    pub tilt_db_per_octave: f32,
    pub tilt_ref_hz: f32,

    /// How fast the tracked floor follows the signal downward. Fast enough to
    /// find the true floor after a loud passage, slow enough not to chase dips.
    pub floor_track_down: f32,
    /// Multiplicative upward creep per frame. Deliberately tiny — the floor must
    /// track the *minimum*, so a sustained tone must not be able to drag it up
    /// and erase itself.
    pub floor_creep_up: f32,
    /// Floor is scaled by this before subtraction; >1 gives margin above the
    /// estimate so residual noise stays clamped at zero.
    pub floor_margin: f32,

    pub db_floor: f32,
    pub db_ceil: f32,

    pub agc_enabled: bool,
    pub agc_tau_secs: f32,
    /// Maximum correction in either direction. Without a clamp, a silent band's
    /// AGC would run away and detonate on the first sound that reaches it.
    pub agc_range_db: f32,
    pub agc_reference_db: f32,

    /// Ballistics, interpolated across the band range (bass -> treble).
    pub attack_ms_bass: f32,
    pub attack_ms_treble: f32,
    pub release_ms_bass: f32,
    pub release_ms_treble: f32,

    /// Neighbour weight in the 3-tap spatial kernel. 0 disables smoothing.
    pub smoothing: f32,
}

impl Default for PostConfig {
    fn default() -> Self {
        Self {
            tilt_db_per_octave: 0.0,
            tilt_ref_hz: 1000.0,
            floor_track_down: 0.3,
            floor_creep_up: 1.000_1,
            floor_margin: 1.5,
            db_floor: -70.0,
            db_ceil: -10.0,
            agc_enabled: true,
            agc_tau_secs: 2.0,
            agc_range_db: 12.0,
            agc_reference_db: -30.0,
            attack_ms_bass: 8.0,
            attack_ms_treble: 1.0,
            release_ms_bass: 250.0,
            release_ms_treble: 120.0,
            smoothing: 0.15,
        }
    }
}

/// Per-frame level retention the tuned release times in [`PostConfig`]
/// correspond to. Decay is expressed as a ratio against this, so the default
/// leaves the bass/treble split exactly as measured rather than flattening it.
pub const REFERENCE_DECAY: f32 = 0.82;

pub struct PostProcessor {
    cfg: PostConfig,
    tilt_db: Vec<f32>,
    /// EQ gain per band, in the analyser's dB. Zero-filled when the EQ is flat.
    eq_db: Vec<f32>,
    attack: Vec<f32>,
    release: Vec<f32>,
    /// Kept so release coefficients can be recomputed when decay changes.
    dt: f32,
    agc_coeff: f32,

    floor: Vec<f32>,
    mean_db: Vec<f32>,
    env: Vec<f32>,
    scratch: Vec<f32>,
    levels: Vec<f32>,
    seeded: bool,
}

impl PostProcessor {
    /// `centers` are band centre frequencies; `dt` is the analysis hop period.
    pub fn new(cfg: PostConfig, centers: &[f32], dt: f32) -> Self {
        let n = centers.len();
        let last = (n.saturating_sub(1)).max(1) as f32;

        let tilt_db = centers
            .iter()
            .map(|&f| cfg.tilt_db_per_octave * (f / cfg.tilt_ref_hz).log2())
            .collect();

        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let (attack, release) = (0..n)
            .map(|i| {
                let t = i as f32 / last;
                (
                    coefficient(lerp(cfg.attack_ms_bass, cfg.attack_ms_treble, t) / 1000.0, dt),
                    coefficient(lerp(cfg.release_ms_bass, cfg.release_ms_treble, t) / 1000.0, dt),
                )
            })
            .unzip();

        Self {
            agc_coeff: coefficient(cfg.agc_tau_secs, dt),
            tilt_db,
            eq_db: vec![0.0; n],
            attack,
            release,
            dt,
            floor: vec![0.0; n],
            mean_db: vec![cfg.agc_reference_db; n],
            env: vec![0.0; n],
            scratch: vec![0.0; n],
            levels: vec![0.0; n],
            seeded: false,
            cfg,
        }
    }

    pub fn config(&self) -> &PostConfig {
        &self.cfg
    }

    /// Brightness per band, 0..1.
    pub fn levels(&self) -> &[f32] {
        &self.levels
    }

    /// Install a pre-sampled EQ response, one dB value per band.
    ///
    /// A wrong-length table is ignored rather than panicking: it can only come
    /// from a UI that has fallen out of step with the band plan, and dropping
    /// the EQ is a far better failure than taking the analyser down.
    pub fn set_eq_db(&mut self, table: &[f32]) {
        if table.len() == self.eq_db.len() {
            self.eq_db.copy_from_slice(table);
        } else {
            self.eq_db.fill(0.0);
        }
    }

    /// Retune the release ballistics.
    ///
    /// `decay` is the fraction of the previous frame a band keeps, which is the
    /// definition the editor presents. It becomes a time constant via
    /// `tau = -dt / ln(decay)` and is applied as a *ratio* against
    /// [`REFERENCE_DECAY`], so the tuned bass-slower-than-treble split survives
    /// instead of collapsing to one number, and the default is a no-op.
    pub fn set_decay(&mut self, decay: f32, centers_len: usize) {
        debug_assert_eq!(centers_len, self.release.len());
        let scale = decay_scale(decay);
        let last = (self.release.len().saturating_sub(1)).max(1) as f32;
        for i in 0..self.release.len() {
            let t = i as f32 / last;
            let base = self.cfg.release_ms_bass
                + (self.cfg.release_ms_treble - self.cfg.release_ms_bass) * t;
            self.release[i] = coefficient(base * scale / 1000.0, self.dt);
        }
    }

    pub fn process(&mut self, magnitudes: &[f32]) -> &[f32] {
        debug_assert_eq!(magnitudes.len(), self.levels.len());

        // Seed well below the first frame rather than at it. Seeding *at* the
        // signal would make `mag - margin*floor` negative on frame one, so any
        // steady tone present at startup would be subtracted to black and stay
        // there. Seeding at zero is equally wrong: the upward creep is
        // multiplicative, so a floor of exactly zero can never rise.
        if !self.seeded {
            for (f, &m) in self.floor.iter_mut().zip(magnitudes) {
                *f = m * FLOOR_SEED_FRACTION;
            }
            self.seeded = true;
        }

        let span = (self.cfg.db_ceil - self.cfg.db_floor).max(1e-6);

        // Indexed rather than zipped: this walks eight parallel per-band arrays
        // at once, and a chain of that many zips reads far worse than the index.
        #[allow(clippy::needless_range_loop)]
        for i in 0..magnitudes.len() {
            let mag = magnitudes[i];

            // 1. Track the floor: snap down toward quiet, creep up very slowly.
            let floor = &mut self.floor[i];
            if mag < *floor {
                *floor += self.cfg.floor_track_down * (mag - *floor);
            } else {
                *floor *= self.cfg.floor_creep_up;
            }
            let clean = (mag - self.cfg.floor_margin * *floor).max(0.0);

            // 2-3. dB, then tilt.
            let mut db = 20.0 * (clean + 1e-9).log10() + self.tilt_db[i];

            // 4. AGC. The mean only updates on frames with real content, so a
            // silent band cannot drift its way into a huge boost.
            if self.cfg.agc_enabled {
                if db > self.cfg.db_floor {
                    self.mean_db[i] += self.agc_coeff * (db - self.mean_db[i]);
                }
                db += (self.cfg.agc_reference_db - self.mean_db[i])
                    .clamp(-self.cfg.agc_range_db, self.cfg.agc_range_db);
            }

            // 4b. User EQ. After the AGC, which would otherwise treat an
            // authored boost as drift and undo it; before the range map, which
            // is what keeps a boost from lifting silence off the floor.
            db += self.eq_db[i];

            // 5. Map the useful dB window onto 0..1.
            let target = ((db - self.cfg.db_floor) / span).clamp(0.0, 1.0);

            // 6. Ballistics.
            let env = &mut self.env[i];
            let c = if target > *env { self.attack[i] } else { self.release[i] };
            *env += c * (target - *env);
            self.scratch[i] = *env;
        }

        // 7. Spatial smoothing, edges clamped so the first and last bars are not
        // dimmed by neighbours that do not exist.
        let w = self.cfg.smoothing;
        if w > 0.0 && self.scratch.len() >= 3 {
            let n = self.scratch.len();
            let c = 1.0 - 2.0 * w;
            for i in 0..n {
                let l = self.scratch[i.saturating_sub(1)];
                let r = self.scratch[(i + 1).min(n - 1)];
                self.levels[i] = w * l + c * self.scratch[i] + w * r;
            }
        } else {
            self.levels.copy_from_slice(&self.scratch);
        }

        &self.levels
    }

    pub fn reset(&mut self) {
        self.floor.fill(0.0);
        self.mean_db.fill(self.cfg.agc_reference_db);
        self.env.fill(0.0);
        self.levels.fill(0.0);
        self.seeded = false;
    }
}

/// Release-time multiplier for a per-frame retention, relative to the tuned
/// default. 1.0 at [`REFERENCE_DECAY`] by construction.
fn decay_scale(decay: f32) -> f32 {
    // tau = -dt/ln(d); dt cancels in the ratio, leaving the log ratio alone.
    let tau_of = |d: f32| -1.0 / d.clamp(1e-4, 0.9999).ln();
    (tau_of(decay) / tau_of(REFERENCE_DECAY)).clamp(0.0, 20.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 256.0 / 48_000.0;

    fn centers(n: usize) -> Vec<f32> {
        (0..n).map(|i| 40.0 * 1.13f32.powi(i as i32)).collect()
    }

    fn processor(cfg: PostConfig) -> PostProcessor {
        let c = centers(48);
        PostProcessor::new(cfg, &c, DT)
    }

    fn run(p: &mut PostProcessor, mags: &[f32], frames: usize) -> Vec<f32> {
        let mut out = Vec::new();
        for _ in 0..frames {
            out = p.process(mags).to_vec();
        }
        out
    }

    #[test]
    fn steady_noise_floor_decays_to_black() {
        let mut p = processor(PostConfig::default());
        let noise = vec![1e-5f32; 48];
        let out = run(&mut p, &noise, 2000);
        assert!(
            out.iter().all(|&v| v < 0.02),
            "constant noise still glowing: max {}",
            out.iter().cloned().fold(0.0f32, f32::max)
        );
    }

    /// The critical counterpart: floor subtraction must not erase a *sustained*
    /// tone. If the floor could chase the signal, held notes would fade out.
    #[test]
    fn sustained_tone_survives_floor_tracking() {
        let mut p = processor(PostConfig { agc_enabled: false, ..Default::default() });
        let mut mags = vec![1e-5f32; 48];
        mags[20] = 0.5;
        let out = run(&mut p, &mags, 2000);
        assert!(out[20] > 0.5, "sustained tone faded to {}", out[20]);
    }

    #[test]
    fn attack_is_faster_than_release() {
        let mut p = processor(PostConfig { agc_enabled: false, smoothing: 0.0, ..Default::default() });
        let quiet = vec![1e-6f32; 48];
        let loud = {
            let mut m = vec![1e-6f32; 48];
            m[40] = 0.5;
            m
        };
        run(&mut p, &quiet, 500);

        let after_attack = run(&mut p, &loud, 3)[40];
        let peak = run(&mut p, &loud, 200)[40];
        let after_release = run(&mut p, &quiet, 3)[40];

        assert!(after_attack > 0.5 * peak, "attack too slow: {after_attack} vs peak {peak}");
        assert!(after_release > 0.5 * peak, "release too fast: {after_release} vs peak {peak}");
    }

    /// AGC on a silent band must stay bounded, or the first sound to arrive
    /// slams the strip to full brightness.
    #[test]
    fn agc_is_bounded_on_silence() {
        let mut p = processor(PostConfig::default());
        run(&mut p, &[0.0; 48], 5000);
        let out = run(&mut p, &[0.0; 48], 1);
        assert!(out.iter().all(|&v| v < 0.01), "silence drifted up to {out:?}");
    }

    /// The reason the EQ sits after the AGC. Applied before it, the AGC would
    /// treat an authored boost as drift and unwind it over its time constant.
    #[test]
    fn eq_survives_the_agc() {
        let mags = vec![1e-3f32; 48];

        let mut flat = processor(PostConfig::default());
        let plain = run(&mut flat, &mags, 3000)[20];

        let mut boosted = processor(PostConfig::default());
        let mut table = vec![0.0f32; 48];
        table[20] = 12.0;
        boosted.set_eq_db(&table);
        let lifted = run(&mut boosted, &mags, 3000)[20];

        assert!(lifted > plain + 0.1, "boost was flattened: {lifted} vs {plain}");
    }

    /// The reason the EQ sits before the range map. Applied to the normalised
    /// level instead, where zero *is* the floor, a boost would make an idle
    /// strip glow.
    #[test]
    fn eq_cannot_lift_silence() {
        let mut p = processor(PostConfig::default());
        p.set_eq_db(&[24.0; 48]);
        let out = run(&mut p, &[0.0; 48], 500);
        assert!(out.iter().all(|&v| v < 1e-3), "silence glowed at {:?}", &out[..4]);
    }

    #[test]
    fn a_wrong_length_eq_table_is_dropped_not_fatal() {
        let mut p = processor(PostConfig::default());
        p.set_eq_db(&[12.0; 8]); // stale UI, band count changed underneath it
        let out = run(&mut p, &[1e-3; 48], 200);
        assert!(out.iter().all(|&v| v.is_finite()));
    }

    #[test]
    fn decay_changes_how_fast_bands_fall() {
        let loud = {
            let mut m = vec![1e-6f32; 48];
            m[40] = 0.5;
            m
        };
        let quiet = vec![1e-6f32; 48];

        let fall = |decay: f32| {
            let mut p = processor(PostConfig { agc_enabled: false, smoothing: 0.0, ..Default::default() });
            p.set_decay(decay, 48);
            run(&mut p, &loud, 400);
            run(&mut p, &quiet, 20)[40]
        };

        let snappy = fall(0.3);
        let sluggish = fall(0.97);
        assert!(snappy < sluggish, "higher decay fell faster: {snappy} vs {sluggish}");
        assert!(snappy < 0.2, "a low decay still hung at {snappy}");
    }

    /// The default must be a no-op, or every existing tuning shifts underneath.
    #[test]
    fn the_reference_decay_leaves_ballistics_untouched() {
        assert!((decay_scale(REFERENCE_DECAY) - 1.0).abs() < 1e-5);

        let tuned = processor(PostConfig { agc_enabled: false, ..Default::default() });
        let mut explicit = processor(PostConfig { agc_enabled: false, ..Default::default() });
        explicit.set_decay(REFERENCE_DECAY, 48);
        for (a, b) in tuned.release.iter().zip(&explicit.release) {
            assert!((a - b).abs() < 1e-6, "release changed: {a} vs {b}");
        }
    }

    #[test]
    fn output_is_always_in_unit_range() {
        let mut p = processor(PostConfig::default());
        for mags in [vec![0.0; 48], vec![1e9; 48], vec![f32::MIN_POSITIVE; 48]] {
            let out = run(&mut p, &mags, 50);
            assert!(
                out.iter().all(|&v| (0.0..=1.0).contains(&v) && v.is_finite()),
                "escaped 0..1 for input {}",
                mags[0]
            );
        }
    }
}
