AK 4K EQ  -  Atmos Kings
A British console channel strip modelled on the E-series: black-knob (242-style)
EQ plus the channel compressor/limiter and expander/gate.

INSTALL (handled by "Update SoundCraft Master" at the top of the repo)

CONTROLS
  Drag knobs up/down. Hold Shift for fine moves. Double-click to reset.
  FILTERS  HPF 18 dB/oct (16-350 Hz, fully left = off), LPF 12 dB/oct (3-22 kHz, fully right = off)
  LF       shelf (or BELL), 30-450 Hz, +/-15 dB
  LMF      200 Hz-2.5 kHz, Q 0.5-2.5, +/-15 dB
  HMF      600 Hz-7 kHz,  Q 0.5-2.5, +/-15 dB
  HF       shelf (or BELL), 1.5-16 kHz, +/-15 dB

  COMP     RATIO 1:1 (off) to LIMIT, THRESH +10 to -20 dB, RELEASE 0.1-4 s
           Soft "over-easy" knee, RMS detection, automatic make-up gain.
           Attack 30 ms per 20 dB of reduction (program dependent);
           FAST ATK = 3 ms per 20 dB. LIN REL = linear release (threshold +6 dB).
  GATE/EXP RANGE 0 (off) to 40 dB, THRESH -30 to +10 dB, HOLD 0-4 s, RELEASE 0.1-4 s
           20:1 gate, or 2:1 expander with EXP. Attack 1.5 ms per 40 dB; FAST ATK = 100 us.
           Compressor and gate share one gain stage, like the console's VCA.
           Stereo is linked (both sides get the same gain).

  OUTPUT   level +/-18 dB, EQ IN (bypass the bands), DYN IN (bypass the dynamics),
           DYN PRE EQ (dynamics before the EQ instead of after),
           ANALOG (console headroom & colour)

  Levels are console dB (dBu) with 0 dBFS = +18 dBu, so THRESH 0 = -18 dBFS.
  Black-knob "proportional Q": small boosts/cuts are broad, big ones get tighter.
  Runs at 2x sample rate internally (31 samples latency, reported to the host).
