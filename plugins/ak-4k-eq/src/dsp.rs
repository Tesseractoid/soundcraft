//! AK 4K EQ sound engine.
//!
//! A model of a black-knob (242-style) British console channel EQ:
//!   * HPF 18 dB/oct (3rd-order), 16 Hz - 350 Hz
//!   * LPF 12 dB/oct (2nd-order), 3 kHz - 22 kHz
//!   * LF  shelf (or bell), 30 Hz - 450 Hz, +/-15 dB
//!   * LMF bell, 200 Hz - 2.5 kHz, Q 0.5 - 2.5, +/-15 dB, proportional Q
//!   * HMF bell, 600 Hz - 7 kHz,  Q 0.5 - 2.5, +/-15 dB, proportional Q
//!   * HF  shelf (or bell), 1.5 kHz - 16 kHz, +/-15 dB
//!   * Optional "analog" stage: op-amp style headroom and soft saturation
//!   * E-series channel dynamics (see `Dynamics`): compressor/limiter and
//!     expander/gate sharing one gain stage, before or after the EQ
//!
//! Everything runs at 2x the host sample rate so the top-end curves don't
//! "cramp" near Nyquist and the saturation doesn't alias.
//!
//! This file has no plugin-framework dependencies so it can be unit tested.

use std::f64::consts::PI;

pub const HPF_OFF_HZ: f32 = 16.0;
pub const LPF_OFF_HZ: f32 = 22_000.0;

/// All the knob and switch positions of the channel EQ.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub hpf_hz: f32,
    pub lpf_hz: f32,
    pub lf_gain_db: f32,
    pub lf_hz: f32,
    pub lf_bell: bool,
    pub lmf_gain_db: f32,
    pub lmf_hz: f32,
    pub lmf_q: f32,
    pub hmf_gain_db: f32,
    pub hmf_hz: f32,
    pub hmf_q: f32,
    pub hf_gain_db: f32,
    pub hf_hz: f32,
    pub hf_bell: bool,
    pub eq_in: bool,
    pub analog: bool,
    pub output_db: f32,

    // dynamics
    pub dyn_in: bool,
    pub dyn_pre_eq: bool,
    pub comp_ratio: f32,
    pub comp_thresh_db: f32,
    pub comp_release_s: f32,
    pub comp_fast: bool,
    pub comp_lin_rel: bool,
    pub gate_range_db: f32,
    pub gate_thresh_db: f32,
    pub gate_hold_s: f32,
    pub gate_release_s: f32,
    pub gate_fast: bool,
    pub gate_exp: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hpf_hz: HPF_OFF_HZ,
            lpf_hz: LPF_OFF_HZ,
            lf_gain_db: 0.0,
            lf_hz: 100.0,
            lf_bell: false,
            lmf_gain_db: 0.0,
            lmf_hz: 600.0,
            lmf_q: 1.0,
            hmf_gain_db: 0.0,
            hmf_hz: 2_500.0,
            hmf_q: 1.0,
            hf_gain_db: 0.0,
            hf_hz: 8_000.0,
            hf_bell: false,
            eq_in: true,
            analog: true,
            output_db: 0.0,

            dyn_in: true,
            dyn_pre_eq: false,
            comp_ratio: 1.0,
            comp_thresh_db: 0.0,
            comp_release_s: 0.3,
            comp_fast: false,
            comp_lin_rel: false,
            gate_range_db: 0.0,
            gate_thresh_db: -10.0,
            gate_hold_s: 0.0,
            gate_release_s: 0.3,
            gate_fast: false,
            gate_exp: false,
        }
    }
}

impl Settings {
    pub fn hpf_on(&self) -> bool {
        self.hpf_hz > HPF_OFF_HZ + 0.5
    }
    pub fn lpf_on(&self) -> bool {
        self.lpf_hz < LPF_OFF_HZ - 50.0
    }
}

/// Black-knob "proportional Q": the curve is broad for gentle moves and
/// tightens as you boost or cut harder, reaching the dialled Q at full gain.
pub fn proportional_q(q: f32, gain_db: f32) -> f64 {
    let g = (gain_db.abs() / 15.0).clamp(0.0, 1.0) as f64;
    q as f64 * (0.4 + 0.6 * g)
}

// ---------------------------------------------------------------------------
// Biquad
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct Coefs {
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64,
}

impl Coefs {
    pub const IDENTITY: Coefs = Coefs { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0 };

    fn norm(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Self {
        Coefs { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 }
    }

    /// Bell. Boost and cut are exact mirror images (reciprocal), as on the console.
    pub fn peak(fs: f64, f: f64, q: f64, gain_db: f64) -> Self {
        if gain_db.abs() < 1e-4 {
            return Self::IDENTITY;
        }
        let a = 10f64.powf(gain_db.abs() / 40.0);
        let w = 2.0 * PI * f / fs;
        let alpha = w.sin() / (2.0 * q);
        let c = w.cos();
        let boost = Self::norm(
            1.0 + alpha * a,
            -2.0 * c,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * c,
            1.0 - alpha / a,
        );
        if gain_db > 0.0 {
            boost
        } else {
            // invert: swap numerator and denominator
            Self::norm(1.0 + alpha / a, -2.0 * c, 1.0 - alpha / a, 1.0 + alpha * a, -2.0 * c, 1.0 - alpha * a)
        }
    }

    pub fn low_shelf(fs: f64, f: f64, q: f64, gain_db: f64) -> Self {
        if gain_db.abs() < 1e-4 {
            return Self::IDENTITY;
        }
        let a = 10f64.powf(gain_db / 40.0);
        let w = 2.0 * PI * f / fs;
        let (s, c) = (w.sin(), w.cos());
        let alpha = s / (2.0 * q);
        let k = 2.0 * a.sqrt() * alpha;
        Self::norm(
            a * ((a + 1.0) - (a - 1.0) * c + k),
            2.0 * a * ((a - 1.0) - (a + 1.0) * c),
            a * ((a + 1.0) - (a - 1.0) * c - k),
            (a + 1.0) + (a - 1.0) * c + k,
            -2.0 * ((a - 1.0) + (a + 1.0) * c),
            (a + 1.0) + (a - 1.0) * c - k,
        )
    }

    pub fn high_shelf(fs: f64, f: f64, q: f64, gain_db: f64) -> Self {
        if gain_db.abs() < 1e-4 {
            return Self::IDENTITY;
        }
        let a = 10f64.powf(gain_db / 40.0);
        let w = 2.0 * PI * f / fs;
        let (s, c) = (w.sin(), w.cos());
        let alpha = s / (2.0 * q);
        let k = 2.0 * a.sqrt() * alpha;
        Self::norm(
            a * ((a + 1.0) + (a - 1.0) * c + k),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
            a * ((a + 1.0) + (a - 1.0) * c - k),
            (a + 1.0) - (a - 1.0) * c + k,
            2.0 * ((a - 1.0) - (a + 1.0) * c),
            (a + 1.0) - (a - 1.0) * c - k,
        )
    }

    pub fn high_pass(fs: f64, f: f64, q: f64) -> Self {
        let w = 2.0 * PI * f / fs;
        let (s, c) = (w.sin(), w.cos());
        let alpha = s / (2.0 * q);
        Self::norm((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0, 1.0 + alpha, -2.0 * c, 1.0 - alpha)
    }

    pub fn low_pass(fs: f64, f: f64, q: f64) -> Self {
        let w = 2.0 * PI * f / fs;
        let (s, c) = (w.sin(), w.cos());
        let alpha = s / (2.0 * q);
        Self::norm((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0, 1.0 + alpha, -2.0 * c, 1.0 - alpha)
    }

    /// First-order high pass (bilinear, prewarped).
    pub fn high_pass_1(fs: f64, f: f64) -> Self {
        let k = (PI * f / fs).tan();
        let a0 = k + 1.0;
        Coefs { b0: 1.0 / a0, b1: -1.0 / a0, b2: 0.0, a1: (k - 1.0) / a0, a2: 0.0 }
    }

    /// Magnitude in dB at frequency f.
    pub fn mag_db(&self, fs: f64, f: f64) -> f64 {
        let w = 2.0 * PI * f / fs;
        // z^-1 = e^{-jw}
        let (c1, s1) = (w.cos(), -w.sin());
        let (c2, s2) = ((2.0 * w).cos(), -(2.0 * w).sin());
        let nr = self.b0 + self.b1 * c1 + self.b2 * c2;
        let ni = self.b1 * s1 + self.b2 * s2;
        let dr = 1.0 + self.a1 * c1 + self.a2 * c2;
        let di = self.a1 * s1 + self.a2 * s2;
        let num = nr * nr + ni * ni;
        let den = dr * dr + di * di;
        10.0 * (num / den).max(1e-30).log10()
    }
}

/// Transposed direct form II biquad state.
#[derive(Clone, Copy, Default, Debug)]
struct Biquad {
    z1: f64,
    z2: f64,
}

impl Biquad {
    #[inline]
    fn tick(&mut self, c: &Coefs, x: f64) -> f64 {
        let y = c.b0 * x + self.z1;
        self.z1 = c.b1 * x - c.a1 * y + self.z2;
        self.z2 = c.b2 * x - c.a2 * y;
        y
    }
}

/// The full set of filter sections for one setting, designed at rate `fs`.
#[derive(Clone, Copy, Debug)]
pub struct Design {
    sections: [Coefs; 8],
    active: [bool; 8],
}

const SHELF_Q: f64 = 0.65; // gentle, console-like shelf slope
const BELL_LF_HF_Q: f32 = 0.9;

impl Design {
    pub fn new(s: &Settings, fs: f64) -> Self {
        let mut sections = [Coefs::IDENTITY; 8];
        let mut active = [false; 8];
        let nyq_guard = fs * 0.45;

        // 0,1: HPF 18 dB/oct = 1st order + 2nd order (Q=1) -> 3rd-order Butterworth
        if s.hpf_on() {
            let f = s.hpf_hz as f64;
            sections[0] = Coefs::high_pass_1(fs, f);
            sections[1] = Coefs::high_pass(fs, f, 1.0);
            active[0] = true;
            active[1] = true;
        }
        // 2: LPF 12 dB/oct
        if s.lpf_on() {
            sections[2] = Coefs::low_pass(fs, (s.lpf_hz as f64).min(nyq_guard), std::f64::consts::FRAC_1_SQRT_2);
            active[2] = true;
        }
        if s.eq_in {
            // 3: LF
            if s.lf_gain_db.abs() > 1e-3 {
                let f = s.lf_hz as f64;
                sections[3] = if s.lf_bell {
                    Coefs::peak(fs, f, proportional_q(BELL_LF_HF_Q, s.lf_gain_db), s.lf_gain_db as f64)
                } else {
                    Coefs::low_shelf(fs, f, SHELF_Q, s.lf_gain_db as f64)
                };
                active[3] = true;
            }
            // 4: LMF
            if s.lmf_gain_db.abs() > 1e-3 {
                sections[4] = Coefs::peak(fs, s.lmf_hz as f64, proportional_q(s.lmf_q, s.lmf_gain_db), s.lmf_gain_db as f64);
                active[4] = true;
            }
            // 5: HMF
            if s.hmf_gain_db.abs() > 1e-3 {
                sections[5] = Coefs::peak(fs, s.hmf_hz as f64, proportional_q(s.hmf_q, s.hmf_gain_db), s.hmf_gain_db as f64);
                active[5] = true;
            }
            // 6: HF
            if s.hf_gain_db.abs() > 1e-3 {
                let f = (s.hf_hz as f64).min(nyq_guard);
                sections[6] = if s.hf_bell {
                    Coefs::peak(fs, f, proportional_q(BELL_LF_HF_Q, s.hf_gain_db), s.hf_gain_db as f64)
                } else {
                    Coefs::high_shelf(fs, f, SHELF_Q, s.hf_gain_db as f64)
                };
                active[6] = true;
            }
        }
        Self { sections, active }
    }

    /// Combined magnitude response in dB (EQ + filters, excluding output gain).
    pub fn mag_db(&self, fs: f64, f: f64) -> f64 {
        self.sections
            .iter()
            .zip(self.active.iter())
            .filter(|(_, a)| **a)
            .map(|(c, _)| c.mag_db(fs, f))
            .sum()
    }
}

/// Response of a setting for drawing the curve (includes output gain).
pub fn response_db(s: &Settings, f: f64) -> f64 {
    const FS: f64 = 96_000.0;
    Design::new(s, FS).mag_db(FS, f) + s.output_db as f64
}

// ---------------------------------------------------------------------------
// Oversampling (2x, linear-phase FIR half-band)
// ---------------------------------------------------------------------------

const FIR_TAPS: usize = 63;

fn design_halfband() -> [f64; FIR_TAPS] {
    // Kaiser-windowed sinc, cutoff at a quarter of the oversampled rate.
    fn bessel_i0(x: f64) -> f64 {
        let mut sum = 1.0;
        let mut term = 1.0;
        let y = x * x / 4.0;
        for k in 1..50 {
            term *= y / (k as f64 * k as f64);
            sum += term;
            if term < 1e-12 * sum {
                break;
            }
        }
        sum
    }
    let beta = 8.0;
    let m = (FIR_TAPS - 1) as f64;
    let mut h = [0.0; FIR_TAPS];
    let fc = 0.25 * 0.94; // just below base-rate Nyquist for a bit of transition band
    for (n, hn) in h.iter_mut().enumerate() {
        let t = n as f64 - m / 2.0;
        let sinc = if t == 0.0 { 2.0 * fc } else { (2.0 * PI * fc * t).sin() / (PI * t) };
        let r = 2.0 * n as f64 / m - 1.0;
        let w = bessel_i0(beta * (1.0 - r * r).max(0.0).sqrt()) / bessel_i0(beta);
        *hn = sinc * w;
    }
    let sum: f64 = h.iter().sum();
    for hn in h.iter_mut() {
        *hn /= sum;
    }
    h
}

#[derive(Clone)]
struct Fir {
    buf: [f64; FIR_TAPS * 2],
    pos: usize,
}

impl Fir {
    fn new() -> Self {
        Self { buf: [0.0; FIR_TAPS * 2], pos: 0 }
    }
    #[inline]
    fn tick(&mut self, h: &[f64; FIR_TAPS], x: f64) -> f64 {
        // double-buffered ring so the window is always contiguous
        self.pos = if self.pos == 0 { FIR_TAPS - 1 } else { self.pos - 1 };
        self.buf[self.pos] = x;
        self.buf[self.pos + FIR_TAPS] = x;
        let w = &self.buf[self.pos..self.pos + FIR_TAPS];
        let mut acc = 0.0;
        for i in 0..FIR_TAPS {
            acc += h[i] * w[i];
        }
        acc
    }
}

/// Latency the oversampler adds, in host-rate samples.
pub const LATENCY_SAMPLES: u32 = (FIR_TAPS as u32 - 1) / 2;

// ---------------------------------------------------------------------------
// Analog stage
// ---------------------------------------------------------------------------

/// Op-amp style stage. 0 dBFS is treated as +18 dBu and the stage starts to
/// round off around +24 dBu, so normal levels stay clean and only hot peaks
/// pick up colour. A small bias adds a touch of 2nd harmonic.
#[derive(Clone, Copy, Default)]
struct AnalogStage {
    dc_x1: f64,
    dc_y1: f64,
}

const HEADROOM: f64 = 0.35; // curve starts to bend audibly only within a few dB of 0 dBFS
const BIAS: f64 = 0.03;

impl AnalogStage {
    #[inline]
    fn tick(&mut self, x: f64, dc_r: f64) -> f64 {
        let u = x * HEADROOM;
        let y = ((u + BIAS).tanh() - BIAS.tanh()) / HEADROOM;
        // remove the tiny DC offset the bias creates
        let out = y - self.dc_x1 + dc_r * self.dc_y1;
        self.dc_x1 = y;
        self.dc_y1 = out;
        out
    }
}

// ---------------------------------------------------------------------------
// Dynamics (E-series channel compressor/limiter and expander/gate)
// ---------------------------------------------------------------------------
//
// Modelled on SSL's published figures for the E-series channel dynamics:
//   Compressor  ratio 1:1 (off) to infinity (limit), threshold +10 to -20 dB,
//               "over-easy" soft knee, attack 30 ms per 20 dB of gain
//               reduction (3 ms per 20 dB with FAST ATT), release 0.1 - 4 s,
//               logarithmic or LIN REL (linear, threshold raised by 6 dB),
//               RMS side-chain, automatic make-up gain from THRESH and RATIO.
//   Gate/Exp    range 0 - 40 dB, threshold -30 to +10 dB, 20:1 gate or 2:1
//               expander (EXP), attack 1.5 ms per 40 dB (100 us with FAST
//               ATT), hold 0 - 4 s, release 0.1 - 4 s.
// Both sections drive one gain stage, like the console's single VCA. Stereo
// is linked: both channels get the same gain, from the louder side.
//
// Levels are in console dB (dBu). 0 dBFS is treated as +18 dBu, the same
// alignment as the analog stage above.

/// dBu = dBFS + this.
pub const DBU_AT_0DBFS: f64 = 18.0;
/// Ratio knob value at (and above) which the compressor becomes a limiter.
pub const COMP_RATIO_LIMIT: f32 = 20.0;

const COMP_KNEE_DB: f64 = 10.0; // "over-easy" curve
const COMP_MAKEUP_REF_DBU: f64 = 0.0; // auto make-up is worked out at the console's working level
const COMP_MAKEUP_AMOUNT: f64 = 0.5;
const RMS_TIME_S: f64 = 0.005;
const GATE_PEAK_DECAY_S: f64 = 0.01;
const GATE_HYSTERESIS_DB: f64 = 2.0;
const GATE_RATIO: f64 = 20.0;
const EXP_RATIO: f64 = 2.0;
const MAX_BLOCK: usize = 64;

impl Settings {
    pub fn comp_active(&self) -> bool {
        self.dyn_in && self.comp_ratio > 1.01
    }
    pub fn gate_active(&self) -> bool {
        self.dyn_in && self.gate_range_db > 0.1
    }
}

/// Compressor curve: gain change in dB (zero or negative) for a level in dBu.
pub fn comp_curve_db(level_dbu: f64, thresh_dbu: f64, ratio: f32) -> f64 {
    let slope = if ratio >= COMP_RATIO_LIMIT { 1.0 } else { 1.0 - 1.0 / ratio.max(1.0) as f64 };
    let over = level_dbu - thresh_dbu;
    let half = COMP_KNEE_DB / 2.0;
    if over <= -half {
        0.0
    } else if over >= half {
        -slope * over
    } else {
        -slope * (over + half).powi(2) / (2.0 * COMP_KNEE_DB)
    }
}

/// Effective compressor threshold (LIN REL raises it by 6 dB, as on the console).
fn comp_thresh(s: &Settings) -> f64 {
    s.comp_thresh_db as f64 + if s.comp_lin_rel { 6.0 } else { 0.0 }
}

/// Automatic make-up gain in dB for the current THRESH and RATIO.
pub fn comp_makeup_db(s: &Settings) -> f64 {
    -comp_curve_db(COMP_MAKEUP_REF_DBU, comp_thresh(s), s.comp_ratio) * COMP_MAKEUP_AMOUNT
}

pub struct Dynamics {
    fs: f64,
    rms_coef: f64,
    peak_decay: f64,
    ms: Vec<f64>,     // per-channel mean square (compressor side-chain)
    comp_gr: f64,     // current compressor gain change, dB (<= 0)
    gate_env: f64,    // peak envelope (gate side-chain), linear
    gate_open: bool,
    gate_gain: f64,   // current gate gain change, dB (<= 0)
    hold_left: u64,
    meter_comp: f32,
    meter_gate: f32,
}

impl Dynamics {
    pub fn new(fs: f64, channels: usize) -> Self {
        Self {
            fs,
            rms_coef: 1.0 - (-1.0 / (RMS_TIME_S * fs)).exp(),
            peak_decay: (-1.0 / (GATE_PEAK_DECAY_S * fs)).exp(),
            ms: vec![0.0; channels.max(1)],
            comp_gr: 0.0,
            gate_env: 0.0,
            gate_open: false,
            gate_gain: 0.0,
            hold_left: 0,
            meter_comp: 0.0,
            meter_gate: 0.0,
        }
    }

    pub fn reset(&mut self) {
        let (fs, ch) = (self.fs, self.ms.len());
        *self = Self::new(fs, ch);
    }

    /// One side-chain step. `levels` iterates the channels' samples at this
    /// instant; returns the linear gain to apply to every channel.
    #[inline]
    fn step(&mut self, s: &Settings, frame: impl Iterator<Item = f64>) -> f64 {
        let fs = self.fs;
        let mut peak = 0.0f64;
        let mut ms_max = 0.0f64;
        for (ms, x) in self.ms.iter_mut().zip(frame) {
            *ms += self.rms_coef * (x * x - *ms);
            ms_max = ms_max.max(*ms);
            peak = peak.max(x.abs());
        }
        let mut total_db = 0.0;

        // compressor / limiter
        if s.comp_active() {
            let level = 10.0 * ms_max.max(1e-20).log10() + DBU_AT_0DBFS;
            let target = comp_curve_db(level, comp_thresh(s), s.comp_ratio);
            if target < self.comp_gr {
                // attack: rate-limited, so it is "program dependent" -
                // small amounts of reduction arrive quickly, big ones take longer
                let per_20db = if s.comp_fast { 0.003 } else { 0.030 };
                let rate = 20.0 / (per_20db * fs);
                self.comp_gr -= (self.comp_gr - target).min(rate);
            } else {
                let rel = (s.comp_release_s as f64).max(0.01);
                if s.comp_lin_rel {
                    // linear: constant dB per second
                    let rate = 10.0 / (rel * fs);
                    self.comp_gr += (target - self.comp_gr).min(rate);
                } else {
                    // logarithmic: exponential return, time constant = RELEASE
                    self.comp_gr += (target - self.comp_gr) * (1.0 - (-1.0 / (rel * fs)).exp());
                }
            }
            total_db += self.comp_gr + comp_makeup_db(s);
        } else {
            self.comp_gr = 0.0;
        }

        // expander / gate
        if s.gate_active() {
            self.gate_env = peak.max(self.gate_env * self.peak_decay);
            let level = 20.0 * self.gate_env.max(1e-10).log10() + DBU_AT_0DBFS;
            let thr = s.gate_thresh_db as f64;
            let range = s.gate_range_db as f64;
            let above = if self.gate_open { level > thr - GATE_HYSTERESIS_DB } else { level > thr };
            self.gate_open = above;
            let target = if above {
                self.hold_left = (s.gate_hold_s as f64 * fs) as u64;
                0.0
            } else if self.hold_left > 0 {
                self.hold_left -= 1;
                self.gate_gain.max(0.0)
            } else {
                let ratio = if s.gate_exp { EXP_RATIO } else { GATE_RATIO };
                ((level - thr) * (ratio - 1.0)).max(-range).min(0.0)
            };
            if target > self.gate_gain {
                let per_40db = if s.gate_fast { 0.0001 } else { 0.0015 };
                self.gate_gain += (target - self.gate_gain).min(40.0 / (per_40db * fs));
            } else {
                let rel = (s.gate_release_s as f64).max(0.01);
                self.gate_gain -= (self.gate_gain - target).min(40.0 / (rel * fs));
            }
            total_db += self.gate_gain;
        } else {
            self.gate_gain = 0.0;
            self.gate_open = false;
            self.hold_left = 0;
        }

        self.meter_comp = self.meter_comp.max((-self.comp_gr) as f32);
        self.meter_gate = self.meter_gate.max((-self.gate_gain) as f32);
        10f64.powf(total_db / 20.0)
    }

    /// Run the side-chain over `chans[..][start..end]` and apply the gain in place.
    fn process(&mut self, s: &Settings, chans: &mut [&mut [f32]], start: usize, end: usize) {
        for i in start..end {
            let g = self.step(s, chans.iter().map(|c| c[i] as f64));
            for c in chans.iter_mut() {
                c[i] = (c[i] as f64 * g) as f32;
            }
        }
    }

    /// Peak compressor and gate gain reduction (dB, positive) since the last call.
    pub fn take_meters(&mut self) -> (f32, f32) {
        let m = (self.meter_comp, self.meter_gate);
        self.meter_comp = 0.0;
        self.meter_gate = 0.0;
        m
    }
}

// ---------------------------------------------------------------------------
// One audio channel (EQ path)
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct Channel {
    up: Fir,
    down: Fir,
    filters: [Biquad; 8],
    analog_in: AnalogStage,
    analog_out: AnalogStage,
}

impl Channel {
    pub fn new() -> Self {
        Self {
            up: Fir::new(),
            down: Fir::new(),
            filters: [Biquad::default(); 8],
            analog_in: AnalogStage::default(),
            analog_out: AnalogStage::default(),
        }
    }
}

impl Default for Channel {
    fn default() -> Self {
        Self::new()
    }
}

/// The whole channel strip (any number of channels).
pub struct Processor {
    fs_os: f64,
    h: [f64; FIR_TAPS],
    channels: Vec<Channel>,
    design: Design,
    settings: Settings,
    out_gain: f64,
    dc_r: f64,
    dynamics: Dynamics,
}

impl Processor {
    pub fn new(sample_rate: f32, num_channels: usize) -> Self {
        let fs_os = sample_rate as f64 * 2.0;
        let settings = Settings::default();
        Self {
            fs_os,
            h: design_halfband(),
            channels: vec![Channel::new(); num_channels],
            design: Design::new(&settings, fs_os),
            settings,
            out_gain: 1.0,
            dc_r: 1.0 - 2.0 * PI * 5.0 / fs_os,
            dynamics: Dynamics::new(sample_rate as f64, num_channels),
        }
    }

    pub fn reset(&mut self) {
        for c in self.channels.iter_mut() {
            *c = Channel::new();
        }
        self.dynamics.reset();
    }

    pub fn set(&mut self, s: &Settings) {
        self.design = Design::new(s, self.fs_os);
        self.settings = *s;
        self.out_gain = 10f64.powf(s.output_db as f64 / 20.0);
    }

    /// Process one block of all channels in place (one slice per channel).
    pub fn process_block(&mut self, chans: &mut [&mut [f32]]) {
        let n = chans.iter().map(|c| c.len()).min().unwrap_or(0);
        let mut start = 0;
        while start < n {
            let end = (start + MAX_BLOCK).min(n);
            let s = self.settings;
            let dynamics = s.comp_active() || s.gate_active();
            if dynamics && s.dyn_pre_eq {
                self.dynamics.process(&s, chans, start, end);
            }
            for (ch, buf) in chans.iter_mut().enumerate().take(self.channels.len()) {
                self.eq(ch, &mut buf[start..end]);
            }
            if dynamics && !s.dyn_pre_eq {
                self.dynamics.process(&s, chans, start, end);
            }
            if !dynamics {
                // keep the side-chain quiet so it doesn't jump when switched back in
                self.dynamics.reset();
            }
            for buf in chans.iter_mut() {
                for x in buf[start..end].iter_mut() {
                    *x = (*x as f64 * self.out_gain) as f32;
                }
            }
            start = end;
        }
    }

    /// Gain-reduction meters (compressor, gate) in dB since the last call.
    pub fn take_meters(&mut self) -> (f32, f32) {
        self.dynamics.take_meters()
    }

    /// The oversampled EQ path for one channel (filters, bands, analog stage).
    fn eq(&mut self, ch: usize, samples: &mut [f32]) {
        let h = &self.h;
        let d = &self.design;
        let analog = self.settings.analog;
        let dc_r = self.dc_r;
        let c = &mut self.channels[ch];
        for x in samples.iter_mut() {
            let input = *x as f64;
            let mut out = 0.0;
            // two oversampled steps per input sample (zero-stuffed, gain 2)
            for phase in 0..2 {
                let up = c.up.tick(h, if phase == 0 { input * 2.0 } else { 0.0 });
                let mut y = if analog { c.analog_in.tick(up, dc_r) } else { up };
                for i in 0..8 {
                    if d.active[i] {
                        y = c.filters[i].tick(&d.sections[i], y);
                    }
                }
                if analog {
                    y = c.analog_out.tick(y, dc_r);
                }
                let dn = c.down.tick(h, y);
                if phase == 0 {
                    out = dn;
                }
            }
            *x = out as f32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s() -> Settings {
        Settings::default()
    }

    fn sine(n: usize, f: f32, amp: f32) -> Vec<f32> {
        (0..n).map(|i| (2.0 * PI as f32 * f * i as f32 / 48_000.0).sin() * amp).collect()
    }

    fn run(st: &Settings, buf: &mut [f32]) {
        let mut p = Processor::new(48_000.0, 1);
        p.set(st);
        for chunk in buf.chunks_mut(32) {
            p.process_block(&mut [chunk]);
        }
    }

    fn rms_dbu(x: &[f32]) -> f64 {
        let ms: f64 = x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len() as f64;
        10.0 * ms.log10() + DBU_AT_0DBFS
    }

    /// Sine amplitude (linear) for a given RMS level in dBu.
    fn amp_for_dbu(dbu: f64) -> f32 {
        (10f64.powf((dbu - DBU_AT_0DBFS) / 20.0) * 2f64.sqrt()) as f32
    }

    #[test]
    fn flat_by_default() {
        let st = s();
        for f in [20.0, 100.0, 1000.0, 10_000.0, 20_000.0] {
            assert!(response_db(&st, f).abs() < 0.01, "not flat at {f}");
        }
    }

    #[test]
    fn bell_hits_target_gain() {
        let mut st = s();
        st.lmf_gain_db = 12.0;
        st.lmf_hz = 1000.0;
        assert!((response_db(&st, 1000.0) - 12.0).abs() < 0.05);
        st.lmf_gain_db = -12.0;
        assert!((response_db(&st, 1000.0) + 12.0).abs() < 0.05);
    }

    #[test]
    fn proportional_q_narrows_with_gain() {
        // width at half-gain points should be narrower at 15 dB than at 3 dB
        let width = |g: f32| {
            let mut st = s();
            st.hmf_gain_db = g;
            st.hmf_hz = 2000.0;
            st.hmf_q = 1.0;
            let half = g as f64 / 2.0;
            let mut lo = 2000.0;
            while response_db(&st, lo) > half {
                lo /= 1.01;
            }
            let mut hi = 2000.0;
            while response_db(&st, hi) > half {
                hi *= 1.01;
            }
            (hi / lo).log2()
        };
        assert!(width(15.0) < width(3.0) * 0.7, "{} vs {}", width(15.0), width(3.0));
    }

    #[test]
    fn hpf_is_18db_per_octave() {
        let mut st = s();
        st.hpf_hz = 200.0;
        let a = response_db(&st, 25.0);
        let b = response_db(&st, 50.0);
        assert!(((b - a) - 18.0).abs() < 1.0, "slope {}", b - a);
        assert!((response_db(&st, 200.0) + 3.0).abs() < 0.3);
    }

    #[test]
    fn lpf_is_12db_per_octave() {
        let mut st = s();
        st.lpf_hz = 3000.0;
        let a = response_db(&st, 6_000.0);
        let b = response_db(&st, 12_000.0);
        assert!(((a - b) - 12.0).abs() < 1.5, "slope {}", a - b);
    }

    #[test]
    fn shelves_reach_full_gain() {
        let mut st = s();
        st.lf_gain_db = 10.0;
        st.lf_hz = 100.0;
        st.hf_gain_db = -10.0;
        st.hf_hz = 5000.0;
        assert!((response_db(&st, 20.0) - 10.0).abs() < 0.5);
        assert!((response_db(&st, 20_000.0) + 10.0).abs() < 0.7);
        assert!(response_db(&st, 1000.0).abs() < 1.0);
    }

    #[test]
    fn processor_is_transparent_when_flat_and_quiet() {
        let mut st = s();
        st.analog = false;
        let n = 4800;
        let mut buf = sine(n, 1000.0, 0.25);
        let orig = buf.clone();
        run(&st, &mut buf);
        let lat = LATENCY_SAMPLES as usize;
        let mut err = 0.0f32;
        for i in 1000..n {
            err = err.max((buf[i] - orig[i - lat]).abs());
        }
        assert!(err < 2e-3, "max err {err}");
    }

    #[test]
    fn analog_stage_is_clean_at_normal_level() {
        // -18 dBFS sine: harmonic distortion should be tiny
        let st = s();
        let mut buf = sine(48_000, 1000.0, 0.125);
        run(&st, &mut buf);
        let h2 = goertzel(&buf[24_000..], 2000.0, 48_000.0);
        let h1 = goertzel(&buf[24_000..], 1000.0, 48_000.0);
        let thd_db = 20.0 * (h2 / h1).log10();
        assert!(thd_db < -55.0, "2nd harmonic at {thd_db} dB");
    }

    // ---- dynamics ----

    fn dyn_settings() -> Settings {
        let mut st = s();
        st.analog = false;
        st
    }

    #[test]
    fn comp_curve_is_soft_then_ratio() {
        // well below threshold: nothing; well above: exactly the ratio
        assert_eq!(comp_curve_db(-20.0, 0.0, 4.0), 0.0);
        assert!((comp_curve_db(20.0, 0.0, 4.0) + 15.0).abs() < 1e-9);
        // at threshold the over-easy knee is already acting a little
        let at = comp_curve_db(0.0, 0.0, 4.0);
        assert!(at < -0.5 && at > -2.5, "{at}");
        // limit: everything above threshold comes back to it
        assert!((comp_curve_db(20.0, 0.0, COMP_RATIO_LIMIT) + 20.0).abs() < 1e-9);
    }

    #[test]
    fn compressor_steady_state_matches_ratio_and_makeup() {
        let mut st = dyn_settings();
        st.comp_ratio = 4.0;
        st.comp_thresh_db = -10.0;
        let in_dbu = 10.0;
        let mut buf = sine(48_000, 1000.0, amp_for_dbu(in_dbu));
        run(&st, &mut buf);
        let out = rms_dbu(&buf[24_000..]);
        // 20 dB over threshold at 4:1 -> 15 dB of reduction, plus automatic make-up
        let expect = in_dbu - 15.0 + comp_makeup_db(&st);
        assert!((out - expect).abs() < 0.6, "out {out:.2} dBu, expected {expect:.2}");
    }

    #[test]
    fn compressor_does_nothing_at_1_to_1() {
        let mut st = dyn_settings();
        st.comp_thresh_db = -20.0; // ratio left at 1:1
        let mut buf = sine(24_000, 1000.0, amp_for_dbu(10.0));
        run(&st, &mut buf);
        assert!((rms_dbu(&buf[12_000..]) - 10.0).abs() < 0.05);
    }

    #[test]
    fn fast_attack_is_about_ten_times_quicker() {
        // time for the gain reduction to reach 10 dB on a sudden loud signal
        let time_to = |fast: bool| {
            let mut st = dyn_settings();
            st.comp_ratio = COMP_RATIO_LIMIT;
            st.comp_thresh_db = -20.0;
            st.comp_fast = fast;
            let mut d = Dynamics::new(48_000.0, 1);
            let x = amp_for_dbu(10.0) as f64 / 2f64.sqrt(); // DC at the same RMS
            for i in 0..48_000 {
                d.step(&st, std::iter::once(x));
                if -d.comp_gr >= 10.0 {
                    return i as f64 / 48.0; // ms
                }
            }
            panic!("never reached 10 dB");
        };
        let (slow, fast) = (time_to(false), time_to(true));
        assert!(slow > 12.0 && slow < 25.0, "normal attack {slow:.1} ms");
        assert!(fast < 3.0, "fast attack {fast:.1} ms");
    }

    #[test]
    fn release_follows_the_knob() {
        let recover = |rel: f32| {
            let mut st = dyn_settings();
            st.comp_ratio = COMP_RATIO_LIMIT;
            st.comp_thresh_db = -20.0;
            st.comp_release_s = rel;
            let mut d = Dynamics::new(48_000.0, 1);
            let loud = amp_for_dbu(10.0) as f64 / 2f64.sqrt();
            for _ in 0..24_000 {
                d.step(&st, std::iter::once(loud));
            }
            let mut n = 0;
            while -d.comp_gr > 3.0 {
                d.step(&st, std::iter::once(0.0));
                n += 1;
            }
            n as f64 / 48_000.0
        };
        let (short, long) = (recover(0.1), recover(1.0));
        assert!(long > short * 6.0, "0.1 s -> {short:.3}, 1 s -> {long:.3}");
    }

    #[test]
    fn gate_closes_by_the_range_below_threshold() {
        let mut st = dyn_settings();
        st.gate_range_db = 40.0;
        st.gate_thresh_db = -10.0;
        let mut quiet = sine(48_000, 1000.0, amp_for_dbu(-30.0));
        run(&st, &mut quiet);
        let att = -30.0 - rms_dbu(&quiet[24_000..]);
        assert!((att - 40.0).abs() < 1.0, "quiet signal attenuated by {att:.1} dB");

        let mut loud = sine(48_000, 1000.0, amp_for_dbu(0.0));
        run(&st, &mut loud);
        assert!(rms_dbu(&loud[24_000..]).abs() < 0.1, "loud signal should pass");
    }

    #[test]
    fn expander_is_2_to_1() {
        let mut st = dyn_settings();
        st.gate_range_db = 40.0;
        st.gate_thresh_db = -10.0;
        st.gate_exp = true;
        let mut buf = sine(48_000, 1000.0, amp_for_dbu(-20.0));
        run(&st, &mut buf);
        // the side-chain reads peaks (3 dB above RMS), so this sine sits ~7 dB under
        let att = -20.0 - rms_dbu(&buf[24_000..]);
        assert!(att > 5.0 && att < 9.0, "expanded by {att:.1} dB");
    }

    #[test]
    fn gate_holds_before_releasing() {
        let mut st = dyn_settings();
        st.gate_range_db = 40.0;
        st.gate_thresh_db = -10.0;
        st.gate_hold_s = 0.2;
        st.gate_release_s = 0.1;
        let mut d = Dynamics::new(48_000.0, 1);
        let loud = amp_for_dbu(0.0) as f64;
        for _ in 0..4_800 {
            d.step(&st, std::iter::once(loud));
        }
        // 150 ms of silence: still inside the hold, so still open
        for _ in 0..7_200 {
            d.step(&st, std::iter::once(0.0));
        }
        assert!(d.gate_gain > -1.0, "closed during hold: {}", d.gate_gain);
        // another 300 ms: hold over and released
        for _ in 0..14_400 {
            d.step(&st, std::iter::once(0.0));
        }
        assert!(d.gate_gain < -39.0, "didn't close: {}", d.gate_gain);
    }

    #[test]
    fn stereo_is_linked() {
        let mut st = dyn_settings();
        st.comp_ratio = COMP_RATIO_LIMIT;
        st.comp_thresh_db = -20.0;
        let mut p = Processor::new(48_000.0, 2);
        p.set(&st);
        let mut l = sine(48_000, 1000.0, amp_for_dbu(10.0));
        let mut r = sine(48_000, 1000.0, amp_for_dbu(-30.0)); // below threshold on its own
        for (a, b) in l.chunks_mut(32).zip(r.chunks_mut(32)) {
            p.process_block(&mut [a, b]);
        }
        let dl = 10.0 - rms_dbu(&l[24_000..]);
        let dr = -30.0 - rms_dbu(&r[24_000..]);
        assert!((dl - dr).abs() < 0.1, "left reduced {dl:.2}, right {dr:.2}");
        assert!(dr > 5.0);
    }

    #[test]
    fn dynamics_can_go_before_the_eq() {
        // a big LF boost after the compressor isn't compressed; before it, it is
        let level = |pre: bool| {
            let mut st = dyn_settings();
            st.comp_ratio = 4.0;
            st.comp_thresh_db = 0.0;
            st.lf_gain_db = 12.0;
            st.lf_hz = 200.0;
            st.dyn_pre_eq = pre;
            let mut buf = sine(48_000, 60.0, amp_for_dbu(-6.0));
            run(&st, &mut buf);
            rms_dbu(&buf[24_000..])
        };
        assert!(level(true) > level(false) + 3.0, "pre {:.1} post {:.1}", level(true), level(false));
    }

    #[test]
    fn meters_report_reduction() {
        let mut st = dyn_settings();
        st.comp_ratio = COMP_RATIO_LIMIT;
        st.comp_thresh_db = -10.0;
        let mut p = Processor::new(48_000.0, 1);
        p.set(&st);
        let mut buf = sine(24_000, 1000.0, amp_for_dbu(10.0));
        for c in buf.chunks_mut(32) {
            p.process_block(&mut [c]);
        }
        let (comp, gate) = p.take_meters();
        assert!(comp > 15.0, "comp meter {comp}");
        assert_eq!(gate, 0.0);
    }

    pub fn goertzel(x: &[f32], f: f64, fs: f64) -> f64 {
        let w = 2.0 * PI * f / fs;
        let c = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0, 0.0);
        for &v in x {
            let s0 = v as f64 + c * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        (s1 * s1 + s2 * s2 - c * s1 * s2).sqrt()
    }
}
