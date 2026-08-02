//! The complete analysis chain, from PCM samples to 0..1 bar levels.

use crate::dsp::bands::{BandPlan, BandScaleConfig};
use crate::dsp::dcblock::DcBlocker;
use crate::dsp::eq::{EqBand, EqCurve};
use crate::dsp::fastpath::{FastPath, FastPathConfig};
use crate::dsp::mrstft::{MultiResStft, DEFAULT_HOP};
use crate::dsp::post::{PostConfig, PostProcessor};
use crate::dsp::window::WindowKind;

#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub scale: BandScaleConfig,
    pub window: WindowKind,
    pub hop: usize,
    /// Removing DC is what stops phantom sub-bass bars. Exposed as a switch only
    /// so the regression test can demonstrate the failure it prevents — there is
    /// no reason to turn this off in normal use.
    pub dc_block: bool,
    /// `None` disables the low-latency parallel path, leaving bass response
    /// governed purely by the STFT window.
    pub fast_path: Option<FastPathConfig>,
    pub post: PostConfig,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            scale: BandScaleConfig::default(),
            window: WindowKind::default(),
            hop: DEFAULT_HOP,
            dc_block: true,
            fast_path: Some(FastPathConfig::default()),
            post: PostConfig::default(),
        }
    }
}

pub struct Engine {
    sample_rate: f64,
    dc: Option<DcBlocker>,
    stft: MultiResStft,
    fast: Option<FastPath>,
    post: PostProcessor,
    centers: Vec<f32>,
    magnitudes: Vec<f32>,
    scratch: Vec<f32>,
}

impl Engine {
    pub fn new(cfg: &EngineConfig, sample_rate: f64) -> Self {
        let plan = BandPlan::new(&cfg.scale, sample_rate, cfg.window);
        let centers: Vec<f32> = plan.bands.iter().map(|b| b.center as f32).collect();
        let dt = cfg.hop as f32 / sample_rate as f32;

        let fast = cfg.fast_path.as_ref().map(|fc| FastPath::new(&plan, fc));
        let n = plan.len();

        Self {
            sample_rate,
            dc: cfg.dc_block.then(DcBlocker::default),
            stft: MultiResStft::with_hop(plan, cfg.window, cfg.hop),
            fast,
            post: PostProcessor::new(cfg.post.clone(), &centers, dt),
            magnitudes: vec![0.0; n],
            scratch: Vec::new(),
            centers,
        }
    }

    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    /// Band centre frequencies. Needed to sample the EQ and to map a strip
    /// position back to a level.
    pub fn centers(&self) -> &[f32] {
        &self.centers
    }

    /// Install a parametric EQ. Sampled at the band centres once here rather
    /// than per frame — the response only changes when the user edits it.
    pub fn set_eq(&mut self, bands: &[EqBand]) {
        let curve = EqCurve::new(bands, self.sample_rate as f32);
        self.post.set_eq_db(&curve.table(&self.centers));
    }

    /// Retune the release ballistics. See [`PostProcessor::set_decay`].
    pub fn set_decay(&mut self, decay: f32) {
        let n = self.centers.len();
        self.post.set_decay(decay, n);
    }

    pub fn plan(&self) -> &BandPlan {
        self.stft.plan()
    }

    pub fn band_count(&self) -> usize {
        self.magnitudes.len()
    }

    /// Merged band magnitudes (RMS), before dB mapping and ballistics.
    pub fn magnitudes(&self) -> &[f32] {
        &self.magnitudes
    }

    /// Bar brightness, 0..1.
    pub fn levels(&self) -> &[f32] {
        self.post.levels()
    }

    /// Feed interleaved-downmixed mono samples. Returns true when a new analysis
    /// frame was produced and [`Self::levels`] has changed.
    pub fn push(&mut self, samples: &[f32]) -> bool {
        self.scratch.clear();
        self.scratch.extend_from_slice(samples);
        if let Some(dc) = &mut self.dc {
            dc.process_block(&mut self.scratch);
        }

        // The fast path must see every sample, not just hop boundaries — that
        // per-sample resolution is the whole source of its low latency.
        if let Some(fast) = &mut self.fast {
            fast.push(&self.scratch);
        }

        if self.stft.push(&self.scratch) == 0 {
            return false;
        }

        self.magnitudes.copy_from_slice(self.stft.magnitudes());
        if let Some(fast) = &mut self.fast {
            fast.apply(self.stft.magnitudes(), &mut self.magnitudes);
        }
        self.post.process(&self.magnitudes);
        true
    }

    pub fn reset(&mut self) {
        if let Some(dc) = &mut self.dc {
            dc.reset();
        }
        self.stft.reset();
        if let Some(fast) = &mut self.fast {
            fast.reset();
        }
        self.post.reset();
        self.magnitudes.fill(0.0);
    }
}
