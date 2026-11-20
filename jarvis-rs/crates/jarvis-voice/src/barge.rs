//! Detects the user talking over Jarvis. Jarvis's own voice comes back through the speakers into the mic,
//! so a plain loudness gate would cut him off constantly. Instead this learns (a) the room's noise floor
//! while he is silent and (b) how loud his voice looks in the mic (mic level / speech level over the first
//! moments of each reply), and only reports sustained sound clearly above both. During pauses in his speech
//! only the noise floor matters. Everything is relative, so a quiet microphone still works.

/// Speech frames (~32 ms each) used to learn the echo ratio at the start of each reply.
const CALIBRATION: u32 = 20;
/// Loud frames in a row (~0.3 s) before the user counts as interrupting.
const SUSTAIN: u32 = 9;
/// Speech level below which Jarvis is considered silent (a pause between words/sentences).
const QUIET: f32 = 0.02;
/// Never trigger below this mic level (RMS 0..1), whatever the floor says.
const MINIMUM: f32 = 0.015;

#[derive(Default)]
pub struct BargeIn {
    floor: f32,
    ratio: f32,
    seen: u32,
    loud: u32,
    // Diagnostics for the log.
    frames: u32,
    peak_mic: f32,
    peak_tts: f32,
    threshold: f32,
}

pub struct Stats {
    pub floor: f32,
    pub ratio: f32,
    pub peak_mic: f32,
    pub peak_tts: f32,
    pub threshold: f32,
}

impl BargeIn {
    /// Forget the learned echo; call when a new reply starts speaking. The noise floor is kept.
    pub fn reset(&mut self) {
        self.ratio = 0.0;
        self.seen = 0;
        self.loud = 0;
    }

    /// Feed a mic frame while Jarvis is NOT speaking to learn the room's ambient level.
    pub fn observe_idle(&mut self, mic: f32) {
        if self.floor == 0.0 {
            self.floor = mic;
        } else if mic < self.floor * 2.5 + 0.005 {
            self.floor += 0.01 * (mic - self.floor);
        }
    }

    /// `mic` is the RMS of one mic frame, `tts` the current speech output level (both 0..1).
    pub fn update(&mut self, mic: f32, tts: f32) -> bool {
        let playing = tts >= QUIET;
        let base = (self.floor * 3.0).max(MINIMUM);
        self.frames += 1;
        self.peak_mic = self.peak_mic.max(mic);
        self.peak_tts = self.peak_tts.max(tts);
        if playing && self.seen < CALIBRATION {
            self.seen += 1;
            self.ratio += (mic / tts - self.ratio) / self.seen as f32;
            return false;
        }
        let threshold = if playing {
            (self.ratio * tts * 1.8 + self.floor * 3.0).max(base)
        } else {
            base
        };
        self.threshold = threshold;
        if mic > threshold {
            self.loud += 1;
        } else {
            self.loud = self.loud.saturating_sub(1);
            if playing {
                self.ratio += 0.03 * (mic / tts - self.ratio);
            }
        }
        self.loud >= SUSTAIN
    }

    /// About once a second while speaking: peaks since the last call, for the diagnostic log.
    pub fn take_stats(&mut self) -> Option<Stats> {
        if self.frames < 31 {
            return None;
        }
        let stats = Stats {
            floor: self.floor,
            ratio: self.ratio,
            peak_mic: self.peak_mic,
            peak_tts: self.peak_tts,
            threshold: self.threshold,
        };
        self.frames = 0;
        self.peak_mic = 0.0;
        self.peak_tts = 0.0;
        Some(stats)
    }
}

pub fn rms(frame: &[i16]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    let energy: f32 = frame.iter().map(|s| (f32::from(*s) / 32768.0).powi(2)).sum();
    (energy / frame.len() as f32).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A speech envelope that wobbles between 0.15 and 0.6.
    fn speech(i: u32) -> f32 {
        0.375 + 0.225 * ((i as f32) * 0.37).sin()
    }

    fn with_floor(noise: f32) -> BargeIn {
        let mut barge = BargeIn::default();
        for _ in 0..100 {
            barge.observe_idle(noise);
        }
        barge
    }

    #[test]
    fn his_own_echo_never_interrupts_him() {
        let mut barge = with_floor(0.02);
        for i in 0..2000 {
            let tts = speech(i);
            // Speakers into mic at ~0.4x plus a little noise.
            let mic = tts * 0.4 + 0.02 * ((i as f32) * 1.9).sin().abs();
            assert!(!barge.update(mic, tts), "false interrupt at frame {i}");
        }
    }

    #[test]
    fn talking_over_him_interrupts_within_half_a_second() {
        let mut barge = with_floor(0.02);
        for i in 0..60 {
            let tts = speech(i);
            assert!(!barge.update(tts * 0.4, tts));
        }
        let hit = (60..100).find(|i| {
            let tts = speech(*i);
            barge.update(tts * 0.4 + 0.3, tts)
        });
        assert!(hit.is_some_and(|i| i - 60 <= 12), "{hit:?}");
    }

    #[test]
    fn a_quiet_microphone_still_works() {
        // Room noise 0.004 (very quiet mic); the user's voice adds only 0.06 on top of his echo.
        let mut barge = with_floor(0.004);
        for i in 0..60 {
            let tts = speech(i);
            assert!(!barge.update(tts * 0.05, tts));
        }
        let hit = (60..120).find(|i| {
            let tts = speech(*i);
            barge.update(tts * 0.05 + 0.06, tts)
        });
        assert!(hit.is_some(), "quiet voice never detected");
    }

    #[test]
    fn pauses_use_only_the_noise_floor() {
        let mut noisy = with_floor(0.04);
        assert!((0..200).all(|_| !noisy.update(0.05, 0.0)), "steady room noise must not trigger");
        assert!((0..20).any(|_| noisy.update(0.3, 0.0)), "loud speech in a pause must trigger");
    }

    #[test]
    fn rms_of_silence_and_full_scale() {
        assert_eq!(rms(&[0; 512]), 0.0);
        assert!((rms(&[i16::MAX; 512]) - 1.0).abs() < 0.01);
    }
}
