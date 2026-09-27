// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 TikTok Imperija
//! Hearing a sound before laying it down.
//!
//! One short sound, once, straight to the output device - nothing to do
//! with the timeline's transport, which goes on playing or not playing
//! underneath. A click on a card in the sound shelf lands here; the card's
//! plus is what actually puts a clip on a track.
//!
//! **A thread of its own, and the stream never leaves it.** cpal's `Stream`
//! cannot be sent between threads on every platform, so it is built, played
//! and dropped inside the one thread that owns it - the same shape
//! [`super::playback`] uses, smaller. Everything outside talks to it down a
//! channel.
//!
//! A stream per sound rather than one kept open: a preview is under two
//! seconds and building the stream costs a few tens of milliseconds, which
//! nobody hears, and holding an output device open for the whole run to
//! play the occasional whoosh is rude to whatever else wants it.

use std::sync::mpsc;

/// What is sent to the listening thread: samples, and the rate they were
/// made at.
struct Ask {
    samples: Vec<f32>,
    rate: u32,
}

/// Plays short sounds, one at a time.
pub struct Audition {
    tx: mpsc::Sender<Ask>,
}

impl Audition {
    /// Starts the listening thread. Silent until something is sent, and
    /// holds no device open in the meantime.
    pub fn start() -> Audition {
        let (tx, rx) = mpsc::channel::<Ask>();
        // A failure to spawn leaves the sender with nowhere to go, and
        // `play` below is deliberately quiet about that: not hearing a
        // preview is a disappointment, not an error worth a dialog.
        let _ = std::thread::Builder::new()
            .name("audition".into())
            .spawn(move || listen(&rx));
        Audition { tx }
    }

    /// Plays these samples once. A sound already playing is cut short.
    pub fn play(&self, samples: Vec<f32>, rate: u32) {
        let _ = self.tx.send(Ask { samples, rate });
    }
}

/// The listening thread: one sound at a time, most recent first.
fn listen(rx: &mpsc::Receiver<Ask>) {
    while let Ok(ask) = rx.recv() {
        // Somebody clicking down the shelf gets the last card they touched
        // rather than all of them in turn.
        let ask = rx.try_iter().last().unwrap_or(ask);
        let _ = play_once(&ask.samples, ask.rate);
    }
}

/// Builds a stream, plays the samples through it, and lets it go.
fn play_once(samples: &[f32], rate: u32) -> Result<(), String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    if samples.is_empty() || rate == 0 {
        return Ok(());
    }
    let device = cpal::default_host()
        .default_output_device()
        .ok_or("no audio output device")?;
    let config = device
        .default_output_config()
        .map_err(|error| format!("no output configuration: {error}"))?;
    let format = config.sample_format();
    let channels = config.channels() as usize;
    // How far along our own samples one frame of the device's output moves.
    // The device rarely runs at the rate a sound was made at, and a whoosh
    // played a tenth too fast is a different whoosh.
    let step = f64::from(rate) / f64::from(config.sample_rate().0);
    let stream = match format {
        cpal::SampleFormat::F32 => build::<f32>(&device, &config.into(), samples, step, channels),
        cpal::SampleFormat::I16 => build::<i16>(&device, &config.into(), samples, step, channels),
        cpal::SampleFormat::U16 => build::<u16>(&device, &config.into(), samples, step, channels),
        other => Err(format!(
            "the output device speaks {other}, which this does not"
        )),
    }?;
    stream.play().map_err(|error| error.to_string())?;

    // Held open for exactly as long as there is something to hear, and a
    // breath more for the device's own buffer to drain.
    let seconds = samples.len() as f64 / f64::from(rate);
    std::thread::sleep(std::time::Duration::from_secs_f64(seconds + 0.15));
    Ok(())
}

/// One output stream that reads the samples once and then falls silent.
fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    samples: &[f32],
    step: f64,
    channels: usize,
) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    use cpal::traits::DeviceTrait;

    let data = samples.to_vec();
    let mut at = 0.0f64;
    device
        .build_output_stream(
            config,
            move |out: &mut [T], _: &cpal::OutputCallbackInfo| {
                for frame in out.chunks_mut(channels.max(1)) {
                    // Between two of our samples, because `step` is rarely
                    // a whole number. Straight-line is plenty for a sound
                    // this short.
                    let i = at as usize;
                    let value = match (data.get(i), data.get(i + 1)) {
                        (Some(a), Some(b)) => {
                            let part = (at - i as f64) as f32;
                            a + (b - a) * part
                        }
                        (Some(a), None) => *a,
                        _ => 0.0,
                    };
                    let sample = T::from_sample(value);
                    for slot in frame.iter_mut() {
                        *slot = sample;
                    }
                    at += step;
                }
            },
            |error| eprintln!("audition stream: {error}"),
            None,
        )
        .map_err(|error| format!("could not open the output: {error}"))
}
