// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 TikTok Imperija
//! The sound effects a short is cut with, made rather than shipped.
//!
//! A whoosh over a cut, a pop as the caption lands, a low hit under the
//! hook: eight sounds, and every one of them arithmetic. Nothing is
//! recorded, downloaded or bundled.
//!
//! **Made and not licensed, on purpose.** A free pack of effects is a
//! licence to read, a download to host and a folder to ship, and half of
//! what is offered as "free" turns out to be free for a video nobody earns
//! from. These are noise and sine waves shaped by envelopes, which is what
//! the recorded ones largely are too - a whoosh *is* filtered noise swept
//! past the ear. They cost a kilobyte of code, they are ours outright, and
//! a person who wants one longer or brighter can be given a dial rather
//! than a different file.
//!
//! Everything here is deterministic: the same sound renders the same bytes
//! every time, which is what lets the tests below check them at all.

/// Samples a second. The project's own rate; a sound written at it needs
/// no resampling on the way into the mix.
pub const RATE: u32 = 48_000;

/// How loud a finished sound peaks, before the clip's own level.
///
/// Short of full scale, so that a sound laid under speech does not clip the
/// mix the moment the two land together.
pub const PEAK: f32 = 0.7;

/// One sound the library offers.
///
/// Grouped by what they are for rather than alphabetically, because that
/// is how the library shows them and how somebody cutting looks for one:
/// the movements, the small marks, the weight, the builds, and the bright
/// ones.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sound {
    // ── movement ─────────────────────────────────────────────────────
    /// Air past the ear: the cut, the swipe, the transition.
    Whoosh,
    /// The same, slower, for a transition that takes its time.
    WhooshLong,
    /// The same, fast, for a cut that does not.
    Swipe,
    /// A run backwards, for arriving at something rather than leaving it.
    Reverse,

    // ── small marks ──────────────────────────────────────────────────
    /// The caption lands.
    Pop,
    /// A word appears: short, rising, and out of the way.
    Blip,
    /// A jump cut, marked.
    Click,
    /// Smaller again: a beat, counted.
    Tick,
    /// One key struck, for words typed on screen.
    Typewriter,
    /// Two clicks a moment apart: a photograph taken.
    Shutter,

    // ── weight ───────────────────────────────────────────────────────
    /// Under the hook: low, long, and felt more than heard.
    Boom,
    /// A smaller Boom, for a word rather than a moment.
    Thud,
    /// Boom with the crack of something breaking on the front of it.
    Impact,
    /// Nearer a punch than a drum.
    Punch,
    /// The floor going out: as low as a phone will carry.
    SubDrop,
    /// Two beats of a heart, for the pause before the answer.
    Heartbeat,

    // ── builds ───────────────────────────────────────────────────────
    /// The build before a reveal.
    Riser,
    /// The same, falling, for landing rather than lifting.
    Faller,
    /// The low horn a trailer leans on.
    Braam,

    // ── bright ───────────────────────────────────────────────────────
    /// The point landed.
    Ding,
    /// Sweeter and longer than a Ding.
    Chime,
    /// Small bells falling over each other: something appearing.
    Sparkle,
    /// Money.
    Cash,
    /// Two tones: look here.
    Alert,

    // ── rough ────────────────────────────────────────────────────────
    /// Wrong answer.
    Buzz,
    /// The picture breaking up.
    Glitch,
    /// A record dragged back.
    Scratch,
}

impl Sound {
    /// Every sound, in the order the library shows them.
    pub const ALL: [Sound; 27] = [
        Sound::Whoosh,
        Sound::WhooshLong,
        Sound::Swipe,
        Sound::Reverse,
        Sound::Pop,
        Sound::Blip,
        Sound::Click,
        Sound::Tick,
        Sound::Typewriter,
        Sound::Shutter,
        Sound::Boom,
        Sound::Thud,
        Sound::Impact,
        Sound::Punch,
        Sound::SubDrop,
        Sound::Heartbeat,
        Sound::Riser,
        Sound::Faller,
        Sound::Braam,
        Sound::Ding,
        Sound::Chime,
        Sound::Sparkle,
        Sound::Cash,
        Sound::Alert,
        Sound::Buzz,
        Sound::Glitch,
        Sound::Scratch,
    ];

    /// The name its file takes, and the key its label is looked up by.
    pub fn id(self) -> &'static str {
        match self {
            Sound::Whoosh => "whoosh",
            Sound::WhooshLong => "whoosh-long",
            Sound::Swipe => "swipe",
            Sound::Reverse => "reverse",
            Sound::Pop => "pop",
            Sound::Blip => "blip",
            Sound::Click => "click",
            Sound::Tick => "tick",
            Sound::Typewriter => "typewriter",
            Sound::Shutter => "shutter",
            Sound::Boom => "boom",
            Sound::Thud => "thud",
            Sound::Impact => "impact",
            Sound::Punch => "punch",
            Sound::SubDrop => "sub-drop",
            Sound::Heartbeat => "heartbeat",
            Sound::Riser => "riser",
            Sound::Faller => "faller",
            Sound::Braam => "braam",
            Sound::Ding => "ding",
            Sound::Chime => "chime",
            Sound::Sparkle => "sparkle",
            Sound::Cash => "cash",
            Sound::Alert => "alert",
            Sound::Buzz => "buzz",
            Sound::Glitch => "glitch",
            Sound::Scratch => "scratch",
        }
    }

    /// Its name in English, for the library card and for translation.
    pub fn label(self) -> &'static str {
        match self {
            Sound::Whoosh => "Whoosh",
            Sound::WhooshLong => "Long Whoosh",
            Sound::Swipe => "Swipe",
            Sound::Reverse => "Reverse Whoosh",
            Sound::Pop => "Pop",
            Sound::Blip => "Blip",
            Sound::Click => "Click",
            Sound::Tick => "Tick",
            Sound::Typewriter => "Typewriter",
            Sound::Shutter => "Shutter",
            Sound::Boom => "Boom",
            Sound::Thud => "Thud",
            Sound::Impact => "Impact",
            Sound::Punch => "Punch",
            Sound::SubDrop => "Sub Drop",
            Sound::Heartbeat => "Heartbeat",
            Sound::Riser => "Riser",
            Sound::Faller => "Faller",
            Sound::Braam => "Braam",
            Sound::Ding => "Ding",
            Sound::Chime => "Chime",
            Sound::Sparkle => "Sparkle",
            Sound::Cash => "Cash",
            Sound::Alert => "Alert",
            Sound::Buzz => "Buzz",
            Sound::Glitch => "Glitch",
            Sound::Scratch => "Scratch",
        }
    }

    /// How long it runs, in seconds.
    pub fn seconds(self) -> f64 {
        match self {
            Sound::Whoosh | Sound::Reverse => 0.55,
            Sound::WhooshLong => 1.2,
            Sound::Swipe => 0.22,
            Sound::Pop => 0.14,
            Sound::Blip => 0.11,
            Sound::Click => 0.04,
            Sound::Tick => 0.02,
            Sound::Typewriter => 0.09,
            Sound::Shutter => 0.16,
            Sound::Boom => 1.2,
            Sound::Thud => 0.35,
            Sound::Impact => 0.9,
            Sound::Punch => 0.28,
            Sound::SubDrop => 1.8,
            Sound::Heartbeat => 0.95,
            Sound::Riser => 1.6,
            Sound::Faller => 1.2,
            Sound::Braam => 1.8,
            Sound::Ding => 1.0,
            Sound::Chime => 1.4,
            Sound::Sparkle => 1.0,
            Sound::Cash => 0.9,
            Sound::Alert => 0.6,
            Sound::Buzz => 0.4,
            Sound::Glitch => 0.4,
            Sound::Scratch => 0.5,
        }
    }

    /// The one it is looked up by, from the name in [`Sound::id`].
    pub fn from_id(id: &str) -> Option<Sound> {
        Sound::ALL.into_iter().find(|sound| sound.id() == id)
    }
}

/// Noise that is the same noise every time.
///
/// A sound the tests can check has to render the same bytes twice, and a
/// whoosh built on a random number generator does not. This is the plain
/// multiply-and-add kind, which is more than random enough for something
/// the ear hears as hiss.
struct Hiss(u64);

impl Hiss {
    fn new() -> Hiss {
        Hiss(0x2545_F491_4F6C_DD1D)
    }

    fn next(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        f32::from((self.0 >> 40) as u16) / f32::from(u16::MAX) * 2.0 - 1.0
    }
}

/// A band of noise, swept: the state-variable filter, one sample at a time.
///
/// Two integrators chasing each other, which is the cheapest filter that
/// can be retuned every sample without going unstable - and retuning it
/// every sample is the whole of a whoosh.
struct Band {
    low: f32,
    band: f32,
}

impl Band {
    fn new() -> Band {
        Band {
            low: 0.0,
            band: 0.0,
        }
    }

    /// One sample through, centred at `centre` hertz with resonance `q`.
    fn pass(&mut self, input: f32, centre: f64, q: f32) -> f32 {
        // Held under a quarter of the rate, past which the two-integrator
        // form stops behaving like a filter and starts behaving like a
        // squeal.
        let centre = centre.clamp(20.0, f64::from(RATE) / 4.0);
        let f = 2.0 * (std::f64::consts::PI * centre / f64::from(RATE)).sin();
        let high = input - self.low - q * self.band;
        self.band += f as f32 * high;
        self.low += f as f32 * self.band;
        self.band
    }
}

/// `0` at both ends and `1` between, so no sound begins or ends on a step.
///
/// A waveform cut off mid-swing is a click, and a library of effects whose
/// every effect clicks is worse than no library. The fade is milliseconds
/// long - short enough that nothing audible is lost from even the shortest
/// of these.
fn eased(at: usize, total: usize) -> f32 {
    const EDGE: usize = (RATE as usize) / 500; // two milliseconds
    let edge = (total / 4).clamp(1, EDGE);
    if at < edge {
        at as f32 / edge as f32
    } else if at + edge >= total {
        // Counted from the last sample, not from one past it: ending on
        // `1 / edge` of the waveform rather than on nothing is exactly the
        // click this exists to prevent.
        (total - 1 - at) as f32 / edge as f32
    } else {
        1.0
    }
}

/// `exp(-t / tau)`, the shape almost everything struck decays in.
fn fall(t: f64, tau: f64) -> f32 {
    (-t / tau.max(1e-6)).exp() as f32
}

/// A plain tone.
fn tone(t: f64, hz: f64) -> f32 {
    (std::f64::consts::TAU * hz * t).sin() as f32
}

/// A tone whose pitch falls from `from` to `to`, a `rate` of it a second.
///
/// The phase is the frequency *integrated*, not the frequency times the
/// time. Multiplying instead is the usual mistake and it does not bend the
/// tone down, it bends it down and then back up again.
fn swoop(t: f64, from: f64, to: f64, rate: f64) -> f32 {
    let phase = std::f64::consts::TAU * (to * t + (from - to) / rate * (1.0 - (-t * rate).exp()));
    phase.sin() as f32
}

/// One partial of something struck: a tone that dies away.
fn bell(t: f64, hz: f64, tau: f64, level: f32) -> f32 {
    tone(t, hz) * fall(t, tau) * level
}

/// A saw, the crude way, from its first few harmonics: enough teeth to
/// sound like brass and few enough not to whistle at this pitch.
fn saw(t: f64, hz: f64) -> f32 {
    (1..=8)
        .map(|n| tone(t, hz * f64::from(n)) / n as f32)
        .sum::<f32>()
        * 0.5
}

/// Nothing before `at`, and dying away after it.
fn after(t: f64, at: f64, tau: f64) -> f32 {
    if t < at { 0.0 } else { fall(t - at, tau) }
}

/// The samples of one sound: mono, `-1` to `1`, at [`RATE`].
///
/// One pass, sample by sample, because every one of these is some mixture
/// of noise through a moving filter and tones that fall - and both want to
/// be told where they were a sample ago.
pub fn render(sound: Sound) -> Vec<f32> {
    let seconds = sound.seconds();
    let total = (seconds * f64::from(RATE)).round() as usize;
    let mut hiss = Hiss::new();
    let mut band = Band::new();
    let mut out = Vec::with_capacity(total);

    for i in 0..total {
        let t = i as f64 / f64::from(RATE);
        let through = t / seconds;
        let value = match sound {
            // Noise swept up and back down, loudest as it passes.
            Sound::Whoosh | Sound::WhooshLong | Sound::Swipe | Sound::Reverse => {
                let arc = (through * std::f64::consts::PI).sin();
                band.pass(hiss.next(), 300.0 + 2700.0 * arc, 0.6) * (arc as f32).powf(1.5) * 2.5
            }
            // A short tone with a knock on the front of it.
            Sound::Pop => {
                let knock = if t < 0.002 { hiss.next() * 0.6 } else { 0.0 };
                (tone(t, 900.0) * fall(t, 0.025) + knock) * 0.9
            }
            // The same idea, rising instead of steady. `swoop` starts at
            // its first number and approaches its second, so naming them
            // the other way round is all a rise takes - a negative rate
            // would run the exponential the wrong way and shriek.
            Sound::Blip => swoop(t, 420.0, 1400.0, 25.0) * fall(t, 0.035),
            // Noise, briefly, with the low end taken out of it.
            Sound::Click => band.pass(hiss.next(), 3200.0, 1.2) * fall(t, 0.008) * 2.0,
            Sound::Tick => band.pass(hiss.next(), 5200.0, 1.4) * fall(t, 0.0035) * 2.0,
            // A key struck: the snap of it and the body of the machine.
            Sound::Typewriter => {
                band.pass(hiss.next(), 3600.0, 1.0) * fall(t, 0.006) * 2.0
                    + swoop(t, 320.0, 180.0, 40.0) * fall(t, 0.02) * 0.4
            }
            // Two of those, a breath apart.
            Sound::Shutter => {
                let snap = band.pass(hiss.next(), 4200.0, 1.1) * 2.0;
                snap * (after(t, 0.0, 0.006) + after(t, 0.062, 0.008) * 0.85)
            }
            // A sine falling in pitch as it fades, under a scrap of noise.
            Sound::Boom | Sound::Thud | Sound::Impact | Sound::Punch | Sound::SubDrop => {
                let (from, to, rate, tau, crack) = match sound {
                    Sound::Boom => (95.0, 42.0, 3.0, 0.34, 0.25),
                    Sound::Thud => (80.0, 55.0, 3.0, 0.08, 0.25),
                    Sound::Impact => (110.0, 45.0, 3.5, 0.26, 0.9),
                    Sound::Punch => (185.0, 90.0, 9.0, 0.07, 0.55),
                    _ => (60.0, 25.0, 1.2, 0.7, 0.1),
                };
                let knock = if t < 0.02 {
                    band.pass(hiss.next(), 2600.0, 1.0) * crack * fall(t, 0.006) * 2.0
                } else {
                    0.0
                };
                swoop(t, from, to, rate) * fall(t, tau) + knock
            }
            // Two beats, the second softer, the way one is heard.
            Sound::Heartbeat => {
                swoop(t, 78.0, 48.0, 8.0) * after(t, 0.0, 0.085)
                    + swoop((t - 0.38).max(0.0), 74.0, 46.0, 8.0) * after(t, 0.38, 0.075) * 0.75
            }
            // Noise climbing, and getting louder as it climbs.
            Sound::Riser => {
                let flutter = 1.0 + 0.15 * tone(t, 11.0);
                band.pass(hiss.next(), 400.0 + 5600.0 * through.powi(2), 0.5)
                    * (through as f32).powf(2.2)
                    * flutter
                    * 2.5
            }
            // And the same falling away.
            Sound::Faller => {
                band.pass(hiss.next(), 6000.0 - 5600.0 * through.powf(0.7), 0.5)
                    * (1.0 - through as f32).powf(1.2)
                    * 2.5
            }
            // Brass, slowly leaned on: two saws a fifth apart, opening.
            Sound::Braam => {
                let swell = (through as f32 * 3.0).min(1.0) * (1.0 - through as f32).powf(0.4);
                let voices = saw(t, 55.0) + saw(t, 82.5) * 0.6 + saw(t, 55.5) * 0.5;
                band.pass(voices, 180.0 + 900.0 * through, 0.9) * swell * 1.5
            }
            // Partials of a struck bar, each dying at its own rate.
            Sound::Ding => {
                bell(t, 1400.0, 0.45, 0.6) + bell(t, 3860.0, 0.22, 0.25) + bell(t, 7560.0, 0.1, 0.1)
            }
            Sound::Chime => {
                bell(t, 1050.0, 0.8, 0.55) + bell(t, 2100.0, 0.5, 0.3) + bell(t, 3150.0, 0.3, 0.15)
            }
            // Small bells falling over each other.
            Sound::Sparkle => (0..6)
                .map(|n| {
                    let at = f64::from(n) * 0.085;
                    let hz = 1800.0 * 1.26_f64.powi(n);
                    tone(t - at, hz) * after(t, at, 0.12) * 0.4
                })
                .sum(),
            // A bell, and then the coins.
            Sound::Cash => {
                let bell = tone(t, 1600.0) * after(t, 0.0, 0.18) * 0.5;
                let coins: f32 = (0..4)
                    .map(|n| {
                        let at = 0.12 + f64::from(n) * 0.1;
                        tone(t - at, 2600.0 + f64::from(n) * 380.0) * after(t, at, 0.05) * 0.35
                    })
                    .sum();
                bell + coins
            }
            // Two tones: look here.
            Sound::Alert => {
                tone(t, 880.0) * after(t, 0.0, 0.09) * 0.6
                    + tone(t - 0.22, 1175.0) * after(t, 0.22, 0.12) * 0.6
            }
            // A square is a saw's harsher cousin; detuned, it grates.
            Sound::Buzz => {
                let square = |hz: f64| if tone(t, hz) >= 0.0 { 1.0 } else { -1.0 };
                band.pass(square(110.0) + square(111.7) * 0.7, 900.0, 0.9)
                    * (1.0 - through as f32).powf(0.3)
                    * 0.5
            }
            // Noise chopped at a rate the ear hears as stuttering.
            Sound::Glitch => {
                let gate = if tone(t, 27.0) > -0.2 { 1.0 } else { 0.0 };
                let cut = if tone(t, 6.5) > 0.0 { 1.0 } else { 0.35 };
                band.pass(hiss.next(), 1200.0 + 2600.0 * cut, 0.8) * gate * 2.2
            }
            // A record dragged back and forth under the needle.
            Sound::Scratch => {
                let zigzag = (tone(t, 4.5).abs() as f64).mul_add(2600.0, 400.0);
                band.pass(hiss.next(), zigzag, 0.4)
                    * (1.0 - (through as f32 - 0.5).abs() * 1.6)
                    * 2.2
            }
        };
        out.push(value * eased(i, total));
    }

    if sound == Sound::Reverse {
        out.reverse();
    }
    normalise(&mut out);
    out
}

/// Brings the loudest sample to [`PEAK`], so no sound in the library is
/// twice the size of its neighbour.
fn normalise(samples: &mut [f32]) {
    let loudest = samples.iter().fold(0.0f32, |top, s| top.max(s.abs()));
    if loudest <= f32::EPSILON {
        return;
    }
    let gain = PEAK / loudest;
    for sample in samples.iter_mut() {
        *sample *= gain;
    }
}

/// Those samples as a mono 16-bit WAV file, header and all.
pub fn wav(samples: &[f32]) -> Vec<u8> {
    let bytes = samples.len() * 2;
    let mut out = Vec::with_capacity(44 + bytes);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + bytes) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // the size of this block
    out.extend_from_slice(&1u16.to_le_bytes()); // uncompressed
    out.extend_from_slice(&1u16.to_le_bytes()); // one channel
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes()); // bytes a second
    out.extend_from_slice(&2u16.to_le_bytes()); // bytes a frame
    out.extend_from_slice(&16u16.to_le_bytes()); // bits a sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(bytes as u32).to_le_bytes());
    for sample in samples {
        let whole = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16;
        out.extend_from_slice(&whole.to_le_bytes());
    }
    out
}

/// One sound, written where it is asked for. Answers with what it wrote.
pub fn write(sound: Sound, into: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let file = into.join(format!("{}.wav", sound.id()));
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not make {}: {error}", parent.display()))?;
    }
    std::fs::write(&file, wav(&render(sound)))
        .map_err(|error| format!("could not write {}: {error}", file.display()))?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How often the waveform crosses zero, per second: high for hiss and a
    /// click, low for anything with weight to it. A cheap stand-in for
    /// "how bright is this", and enough to tell a Boom from a Click
    /// without anybody listening.
    fn brightness(samples: &[f32]) -> f64 {
        let crossings = samples
            .windows(2)
            .filter(|pair| (pair[0] < 0.0) != (pair[1] < 0.0))
            .count();
        crossings as f64 / (samples.len() as f64 / f64::from(RATE))
    }

    fn loudness(samples: &[f32]) -> f32 {
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    #[test]
    fn every_sound_is_the_length_it_says_and_is_actually_audible() {
        for sound in Sound::ALL {
            let samples = render(sound);
            let wanted = (sound.seconds() * f64::from(RATE)).round() as usize;
            assert_eq!(samples.len(), wanted, "{sound:?}");
            assert!(
                samples.iter().all(|s| s.is_finite()),
                "{sound:?} has a sample that is not a number"
            );
            let peak = samples.iter().fold(0.0f32, |top, s| top.max(s.abs()));
            assert!((peak - PEAK).abs() < 1e-4, "{sound:?} peaks at {peak}");
            assert!(loudness(&samples) > 0.02, "{sound:?} is near silent");
        }
    }

    #[test]
    fn no_sound_begins_or_ends_on_a_step() {
        // A waveform cut off mid-swing clicks, and a library of effects
        // that all click is worse than none.
        for sound in Sound::ALL {
            let samples = render(sound);
            assert!(
                samples[0].abs() < 1e-6,
                "{sound:?} starts at {}",
                samples[0]
            );
            let last = *samples.last().unwrap();
            assert!(last.abs() < 1e-6, "{sound:?} ends at {last}");
        }
    }

    #[test]
    fn the_low_sounds_are_low_and_the_sharp_ones_sharp() {
        let boom = brightness(&render(Sound::Boom));
        let thud = brightness(&render(Sound::Thud));
        let click = brightness(&render(Sound::Click));
        let whoosh = brightness(&render(Sound::Whoosh));
        assert!(boom < 400.0, "Boom crosses zero {boom:.0} times a second");
        assert!(thud < 400.0, "Thud crosses zero {thud:.0} times a second");
        assert!(click > 2000.0, "Click is only {click:.0}");
        assert!(whoosh > boom * 3.0, "a whoosh should outshine a boom");
    }

    #[test]
    fn a_riser_rises_and_a_boom_does_not() {
        let riser = render(Sound::Riser);
        let (first, second) = riser.split_at(riser.len() / 2);
        assert!(
            loudness(second) > loudness(first) * 3.0,
            "the riser does not build: {} then {}",
            loudness(first),
            loudness(second)
        );
        assert!(
            brightness(second) > brightness(first),
            "the riser does not climb"
        );
        let boom = render(Sound::Boom);
        let (first, second) = boom.split_at(boom.len() / 2);
        assert!(
            loudness(second) < loudness(first),
            "the boom does not decay"
        );
    }

    #[test]
    fn the_reverse_is_the_whoosh_backwards() {
        let forwards = render(Sound::Whoosh);
        let backwards = render(Sound::Reverse);
        assert_eq!(forwards.len(), backwards.len());
        for (i, (a, b)) in forwards
            .iter()
            .zip(backwards.iter().rev())
            .enumerate()
            .step_by(97)
        {
            assert!((a - b).abs() < 1e-6, "sample {i}: {a} against {b}");
        }
    }

    #[test]
    fn the_same_sound_renders_the_same_bytes_twice() {
        for sound in Sound::ALL {
            assert_eq!(render(sound), render(sound), "{sound:?}");
        }
    }

    #[test]
    fn the_file_says_what_it_holds() {
        let samples = render(Sound::Pop);
        let file = wav(&samples);
        assert_eq!(&file[0..4], b"RIFF");
        assert_eq!(&file[8..12], b"WAVE");
        assert_eq!(&file[36..40], b"data");
        assert_eq!(file.len(), 44 + samples.len() * 2);
        let stated = u32::from_le_bytes(file[40..44].try_into().unwrap()) as usize;
        assert_eq!(stated, samples.len() * 2);
        let rate = u32::from_le_bytes(file[24..28].try_into().unwrap());
        assert_eq!(rate, RATE);
    }

    #[test]
    fn a_sound_is_found_by_its_name_and_no_two_share_one() {
        let mut names: Vec<&str> = Sound::ALL.iter().map(|s| s.id()).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count);
        for sound in Sound::ALL {
            assert_eq!(Sound::from_id(sound.id()), Some(sound));
        }
        assert_eq!(Sound::from_id("nothing like it"), None);
    }
}
