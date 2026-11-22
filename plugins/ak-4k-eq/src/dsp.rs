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
// One audio channel
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

/// The whole processor (any number of channels).
pub struct Processor {
    fs_os: f64,
    h: [f64; FIR_TAPS],
    channels: Vec<Channel>,
    design: Design,
    analog: bool,
    out_gain: f64,
    dc_r: f64,
}

impl Processor {
    pub fn new(sample_rate: f32, num_channels: usize) -> Self {
        let fs_os = sample_rate as f64 * 2.0;
        Self {
            fs_os,
            h: design_halfband(),
            channels: vec![Channel::new(); num_channels],
            design: Design::new(&Settings::default(), fs_os),
            analog: true,
            out_gain: 1.0,
            dc_r: 1.0 - 2.0 * PI * 5.0 / fs_os,
        }
    }

    pub fn reset(&mut self) {
        for c in self.channels.iter_mut() {
            *c = Channel::new();
        }
    }

    pub fn set(&mut self, s: &Settings) {
        self.design = Design::new(s, self.fs_os);
        self.analog = s.analog;
        self.out_gain = 10f64.powf(s.output_db as f64 / 20.0);
    }

    /// Process one block of one channel in place.
    pub fn process(&mut self, ch: usize, samples: &mut [f32]) {
        let h = &self.h;
        let d = &self.design;
        let analog = self.analog;
        let dc_r = self.dc_r;
        let g = self.out_gain;
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
            *x = (out * g) as f32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s() -> Settings {
        Settings::default()
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
        let mut p = Processor::new(48_000.0, 1);
        p.set(&st);
        let n = 4800;
        let mut buf: Vec<f32> = (0..n).map(|i| (2.0 * PI as f32 * 1000.0 * i as f32 / 48_000.0).sin() * 0.25).collect();
        let orig = buf.clone();
        p.process(0, &mut buf);
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
        let mut p = Processor::new(48_000.0, 1);
        p.set(&st);
        let n = 48_000;
        let amp = 0.125f32;
        let mut buf: Vec<f32> = (0..n).map(|i| (2.0 * PI as f32 * 1000.0 * i as f32 / 48_000.0).sin() * amp).collect();
        p.process(0, &mut buf);
        let h2 = goertzel(&buf[24_000..], 2000.0, 48_000.0);
        let h1 = goertzel(&buf[24_000..], 1000.0, 48_000.0);
        let thd_db = 20.0 * (h2 / h1).log10();
        assert!(thd_db < -55.0, "2nd harmonic at {thd_db} dB");
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
