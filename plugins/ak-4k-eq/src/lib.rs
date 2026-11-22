use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

pub mod dsp;
mod editor;

pub struct Ak4kEq {
    params: Arc<EqParams>,
    processor: Option<dsp::Processor>,
    meters: Arc<Meters>,
}

/// Gain-reduction readings passed from the audio thread to the panel (dB, positive).
#[derive(Default)]
pub struct Meters {
    comp: AtomicU32,
    gate: AtomicU32,
}

impl Meters {
    fn set(&self, comp: f32, gate: f32) {
        self.comp.store(comp.to_bits(), Ordering::Relaxed);
        self.gate.store(gate.to_bits(), Ordering::Relaxed);
    }
    pub fn get(&self) -> (f32, f32) {
        (f32::from_bits(self.comp.load(Ordering::Relaxed)), f32::from_bits(self.gate.load(Ordering::Relaxed)))
    }
}

#[derive(Params)]
pub struct EqParams {
    #[persist = "editor-state-v2"]
    pub editor_state: Arc<EguiState>,

    #[id = "hpf"]
    pub hpf: FloatParam,
    #[id = "lpf"]
    pub lpf: FloatParam,

    #[id = "lf_gain"]
    pub lf_gain: FloatParam,
    #[id = "lf_freq"]
    pub lf_freq: FloatParam,
    #[id = "lf_bell"]
    pub lf_bell: BoolParam,

    #[id = "lmf_gain"]
    pub lmf_gain: FloatParam,
    #[id = "lmf_freq"]
    pub lmf_freq: FloatParam,
    #[id = "lmf_q"]
    pub lmf_q: FloatParam,

    #[id = "hmf_gain"]
    pub hmf_gain: FloatParam,
    #[id = "hmf_freq"]
    pub hmf_freq: FloatParam,
    #[id = "hmf_q"]
    pub hmf_q: FloatParam,

    #[id = "hf_gain"]
    pub hf_gain: FloatParam,
    #[id = "hf_freq"]
    pub hf_freq: FloatParam,
    #[id = "hf_bell"]
    pub hf_bell: BoolParam,

    #[id = "eq_in"]
    pub eq_in: BoolParam,
    #[id = "analog"]
    pub analog: BoolParam,
    #[id = "output"]
    pub output: FloatParam,

    #[id = "dyn_in"]
    pub dyn_in: BoolParam,
    #[id = "dyn_pre_eq"]
    pub dyn_pre_eq: BoolParam,

    #[id = "comp_ratio"]
    pub comp_ratio: FloatParam,
    #[id = "comp_thresh"]
    pub comp_thresh: FloatParam,
    #[id = "comp_release"]
    pub comp_release: FloatParam,
    #[id = "comp_fast"]
    pub comp_fast: BoolParam,
    #[id = "comp_lin_rel"]
    pub comp_lin_rel: BoolParam,

    #[id = "gate_range"]
    pub gate_range: FloatParam,
    #[id = "gate_thresh"]
    pub gate_thresh: FloatParam,
    #[id = "gate_hold"]
    pub gate_hold: FloatParam,
    #[id = "gate_release"]
    pub gate_release: FloatParam,
    #[id = "gate_fast"]
    pub gate_fast: BoolParam,
    #[id = "gate_exp"]
    pub gate_exp: BoolParam,
}

fn freq_param(name: &str, default: f32, min: f32, max: f32) -> FloatParam {
    FloatParam::new(
        name,
        default,
        FloatRange::Skewed { min, max, factor: FloatRange::skew_factor(-1.5) },
    )
    .with_smoother(SmoothingStyle::Logarithmic(40.0))
    .with_value_to_string(formatters::v2s_f32_hz_then_khz(1))
    .with_string_to_value(formatters::s2v_f32_hz_then_khz())
}

fn gain_param(name: &str, range: f32) -> FloatParam {
    FloatParam::new(name, 0.0, FloatRange::Linear { min: -range, max: range })
        .with_smoother(SmoothingStyle::Linear(40.0))
        .with_step_size(0.1)
        .with_unit(" dB")
}

fn q_param(name: &str) -> FloatParam {
    FloatParam::new(
        name,
        1.0,
        FloatRange::Skewed { min: 0.5, max: 2.5, factor: FloatRange::skew_factor(-0.8) },
    )
    .with_smoother(SmoothingStyle::Linear(40.0))
    .with_step_size(0.01)
}

fn time_param(name: &str, default: f32, min: f32, max: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Skewed { min, max, factor: FloatRange::skew_factor(-1.5) })
        .with_smoother(SmoothingStyle::Linear(40.0))
        .with_value_to_string(Arc::new(|v: f32| if v < 1.0 { format!("{:.0} ms", v * 1000.0) } else { format!("{v:.2} s") }))
}

fn level_param(name: &str, default: f32, min: f32, max: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min, max })
        .with_smoother(SmoothingStyle::Linear(40.0))
        .with_step_size(0.1)
        .with_unit(" dB")
}

impl Default for EqParams {
    fn default() -> Self {
        Self {
            editor_state: editor::default_state(),

            hpf: freq_param("HPF", dsp::HPF_OFF_HZ, dsp::HPF_OFF_HZ, 350.0).with_value_to_string(Arc::new(|v: f32| {
                if v <= dsp::HPF_OFF_HZ + 0.5 {
                    "Off".to_string()
                } else {
                    format!("{v:.0} Hz")
                }
            })),
            lpf: freq_param("LPF", dsp::LPF_OFF_HZ, 3_000.0, dsp::LPF_OFF_HZ).with_value_to_string(Arc::new(|v: f32| {
                if v >= dsp::LPF_OFF_HZ - 50.0 {
                    "Off".to_string()
                } else {
                    format!("{:.1} kHz", v / 1000.0)
                }
            })),

            lf_gain: gain_param("LF Gain", 15.0),
            lf_freq: freq_param("LF Freq", 100.0, 30.0, 450.0),
            lf_bell: BoolParam::new("LF Bell", false),

            lmf_gain: gain_param("LMF Gain", 15.0),
            lmf_freq: freq_param("LMF Freq", 600.0, 200.0, 2_500.0),
            lmf_q: q_param("LMF Q"),

            hmf_gain: gain_param("HMF Gain", 15.0),
            hmf_freq: freq_param("HMF Freq", 2_500.0, 600.0, 7_000.0),
            hmf_q: q_param("HMF Q"),

            hf_gain: gain_param("HF Gain", 15.0),
            hf_freq: freq_param("HF Freq", 8_000.0, 1_500.0, 16_000.0),
            hf_bell: BoolParam::new("HF Bell", false),

            eq_in: BoolParam::new("EQ In", true),
            analog: BoolParam::new("Analog", true),
            output: gain_param("Output", 18.0),

            dyn_in: BoolParam::new("Dyn In", true),
            dyn_pre_eq: BoolParam::new("Dyn Pre EQ", false),

            comp_ratio: FloatParam::new(
                "Comp Ratio",
                1.0,
                FloatRange::Skewed { min: 1.0, max: dsp::COMP_RATIO_LIMIT, factor: FloatRange::skew_factor(-1.2) },
            )
            .with_smoother(SmoothingStyle::Linear(40.0))
            .with_value_to_string(Arc::new(|v: f32| {
                if v >= dsp::COMP_RATIO_LIMIT - 0.05 {
                    "Limit".to_string()
                } else if v <= 1.01 {
                    "Off".to_string()
                } else {
                    format!("{v:.1}:1")
                }
            })),
            comp_thresh: level_param("Comp Thresh", 0.0, -20.0, 10.0),
            comp_release: time_param("Comp Release", 0.3, 0.1, 4.0),
            comp_fast: BoolParam::new("Comp Fast Attack", false),
            comp_lin_rel: BoolParam::new("Comp Linear Release", false),

            gate_range: level_param("Gate Range", 0.0, 0.0, 40.0),
            gate_thresh: level_param("Gate Thresh", -10.0, -30.0, 10.0),
            gate_hold: time_param("Gate Hold", 0.0, 0.0, 4.0),
            gate_release: time_param("Gate Release", 0.3, 0.1, 4.0),
            gate_fast: BoolParam::new("Gate Fast Attack", false),
            gate_exp: BoolParam::new("Gate Expander", false),
        }
    }
}

impl EqParams {
    /// Current (unsmoothed) settings, used for drawing the curve.
    pub fn settings(&self) -> dsp::Settings {
        dsp::Settings {
            hpf_hz: self.hpf.value(),
            lpf_hz: self.lpf.value(),
            lf_gain_db: self.lf_gain.value(),
            lf_hz: self.lf_freq.value(),
            lf_bell: self.lf_bell.value(),
            lmf_gain_db: self.lmf_gain.value(),
            lmf_hz: self.lmf_freq.value(),
            lmf_q: self.lmf_q.value(),
            hmf_gain_db: self.hmf_gain.value(),
            hmf_hz: self.hmf_freq.value(),
            hmf_q: self.hmf_q.value(),
            hf_gain_db: self.hf_gain.value(),
            hf_hz: self.hf_freq.value(),
            hf_bell: self.hf_bell.value(),
            eq_in: self.eq_in.value(),
            analog: self.analog.value(),
            output_db: self.output.value(),
            dyn_in: self.dyn_in.value(),
            dyn_pre_eq: self.dyn_pre_eq.value(),
            comp_ratio: self.comp_ratio.value(),
            comp_thresh_db: self.comp_thresh.value(),
            comp_release_s: self.comp_release.value(),
            comp_fast: self.comp_fast.value(),
            comp_lin_rel: self.comp_lin_rel.value(),
            gate_range_db: self.gate_range.value(),
            gate_thresh_db: self.gate_thresh.value(),
            gate_hold_s: self.gate_hold.value(),
            gate_release_s: self.gate_release.value(),
            gate_fast: self.gate_fast.value(),
            gate_exp: self.gate_exp.value(),
        }
    }

    /// Smoothed settings, advanced by `n` samples (audio thread).
    fn smoothed_settings(&self, n: u32) -> dsp::Settings {
        dsp::Settings {
            hpf_hz: self.hpf.smoothed.next_step(n),
            lpf_hz: self.lpf.smoothed.next_step(n),
            lf_gain_db: self.lf_gain.smoothed.next_step(n),
            lf_hz: self.lf_freq.smoothed.next_step(n),
            lf_bell: self.lf_bell.value(),
            lmf_gain_db: self.lmf_gain.smoothed.next_step(n),
            lmf_hz: self.lmf_freq.smoothed.next_step(n),
            lmf_q: self.lmf_q.smoothed.next_step(n),
            hmf_gain_db: self.hmf_gain.smoothed.next_step(n),
            hmf_hz: self.hmf_freq.smoothed.next_step(n),
            hmf_q: self.hmf_q.smoothed.next_step(n),
            hf_gain_db: self.hf_gain.smoothed.next_step(n),
            hf_hz: self.hf_freq.smoothed.next_step(n),
            hf_bell: self.hf_bell.value(),
            eq_in: self.eq_in.value(),
            analog: self.analog.value(),
            output_db: self.output.smoothed.next_step(n),
            dyn_in: self.dyn_in.value(),
            dyn_pre_eq: self.dyn_pre_eq.value(),
            comp_ratio: self.comp_ratio.smoothed.next_step(n),
            comp_thresh_db: self.comp_thresh.smoothed.next_step(n),
            comp_release_s: self.comp_release.smoothed.next_step(n),
            comp_fast: self.comp_fast.value(),
            comp_lin_rel: self.comp_lin_rel.value(),
            gate_range_db: self.gate_range.smoothed.next_step(n),
            gate_thresh_db: self.gate_thresh.smoothed.next_step(n),
            gate_hold_s: self.gate_hold.smoothed.next_step(n),
            gate_release_s: self.gate_release.smoothed.next_step(n),
            gate_fast: self.gate_fast.value(),
            gate_exp: self.gate_exp.value(),
        }
    }
}

impl Default for Ak4kEq {
    fn default() -> Self {
        Self { params: Arc::new(EqParams::default()), processor: None, meters: Arc::new(Meters::default()) }
    }
}

const BLOCK: usize = 32;

impl Plugin for Ak4kEq {
    const NAME: &'static str = "AK 4K EQ";
    const VENDOR: &'static str = "Atmos Kings";
    const URL: &'static str = "https://atmos-kings.com";
    const EMAIL: &'static str = "info@atmos-kings.com";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(1),
            main_output_channels: NonZeroU32::new(1),
            ..AudioIOLayout::const_default()
        },
    ];

    const SAMPLE_ACCURATE_AUTOMATION: bool = false;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create(self.params.clone(), self.meters.clone())
    }

    fn initialize(
        &mut self,
        layout: &AudioIOLayout,
        config: &BufferConfig,
        context: &mut impl InitContext<Self>,
    ) -> bool {
        let channels = layout.main_output_channels.map(|c| c.get()).unwrap_or(2) as usize;
        let mut p = dsp::Processor::new(config.sample_rate, channels);
        p.set(&self.params.settings());
        self.processor = Some(p);
        context.set_latency_samples(dsp::LATENCY_SAMPLES);
        true
    }

    fn reset(&mut self) {
        if let Some(p) = self.processor.as_mut() {
            p.reset();
        }
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let Some(p) = self.processor.as_mut() else {
            return ProcessStatus::Normal;
        };
        for (_, mut block) in buffer.iter_blocks(BLOCK) {
            let n = block.samples() as u32;
            let s = self.params.smoothed_settings(n);
            p.set(&s);
            // dynamics are stereo-linked, so both channels go in together
            let mut chans = block.iter_mut();
            match (chans.next(), chans.next()) {
                (Some(l), Some(r)) => p.process_block(&mut [l, r]),
                (Some(m), None) => p.process_block(&mut [m]),
                _ => {}
            }
        }
        let (comp, gate) = p.take_meters();
        self.meters.set(comp, gate);
        ProcessStatus::Normal
    }
}

impl ClapPlugin for Ak4kEq {
    const CLAP_ID: &'static str = "com.atmos-kings.ak-4k-eq";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("British console channel strip: black-knob EQ, compressor and gate");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] =
        &[ClapFeature::AudioEffect, ClapFeature::Equalizer, ClapFeature::Compressor, ClapFeature::Gate, ClapFeature::Stereo, ClapFeature::Mono];
}

impl Vst3Plugin for Ak4kEq {
    const VST3_CLASS_ID: [u8; 16] = *b"AtmosKingsAK4kEQ";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Fx, Vst3SubCategory::Eq, Vst3SubCategory::Dynamics];
}

nih_export_clap!(Ak4kEq);
nih_export_vst3!(Ak4kEq);
