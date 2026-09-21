//! Lightweight native feedback. Mining uses a synthesized bell; ordinary
//! incoming transfers retain their existing chime. Tests/UI shots stay silent.

/// A soft three-note bell, synthesized once as a 24 kHz mono PCM WAV (~46 kB).
/// Native playback runs on a worker; no audio dependency or downloaded asset.
pub fn play_reward_chime() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static PLAYING: AtomicBool = AtomicBool::new(false);
    if cfg!(test)
        || std::env::var_os("NIGHTFALL_UI_SHOTS").is_some()
        || PLAYING.swap(true, Ordering::SeqCst)
    {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("nightfall-reward-sound".into())
        .spawn(|| {
            struct Finished;
            impl Drop for Finished {
                fn drop(&mut self) {
                    PLAYING.store(false, Ordering::SeqCst);
                }
            }
            let _finished = Finished;
            static WAV: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
            play_wav(WAV.get_or_init(reward_wav));
        });
    if spawned.is_err() {
        PLAYING.store(false, Ordering::SeqCst);
    }
}

fn reward_wav() -> Vec<u8> {
    const RATE: u32 = 24_000;
    const SAMPLES: u32 = 22_800;
    let mut wav = Vec::with_capacity(44 + SAMPLES as usize * 2);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + SAMPLES * 2).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(SAMPLES * 2).to_le_bytes());
    for sample in 0..SAMPLES {
        let time = sample as f64 / RATE as f64;
        let mut value = 0.0;
        for (frequency, onset) in [(783.99, 0.0), (987.77, 0.10), (1174.66, 0.20)] {
            let t = time - onset;
            if t >= 0.0 {
                let envelope = (t / 0.008).min(1.0)
                    * (-t / 0.19).exp()
                    * ((0.95 - time) / 0.10).clamp(0.0, 1.0);
                let phase = std::f64::consts::TAU * frequency * t;
                value += envelope * (phase.sin() + 0.16 * (phase * 2.002).sin()) * 0.14;
            }
        }
        let pcm = (value.clamp(-0.8, 0.8) * i16::MAX as f64) as i16;
        wav.extend_from_slice(&pcm.to_le_bytes());
    }
    wav
}

#[cfg(target_os = "windows")]
fn play_wav(wav: &[u8]) {
    #[link(name = "winmm")]
    unsafe extern "system" {
        fn PlaySoundW(sound: *const u16, module: *mut std::ffi::c_void, flags: u32) -> i32;
    }
    // SND_MEMORY | SND_NODEFAULT, synchronous on this audio worker. The byte
    // buffer remains alive until the native call returns.
    unsafe {
        PlaySoundW(wav.as_ptr().cast(), std::ptr::null_mut(), 0x0004 | 0x0002);
    }
}

#[cfg(not(target_os = "windows"))]
fn play_wav(wav: &[u8]) {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "nightfall-reward-{}-{nonce}.wav",
        std::process::id()
    ));
    let Ok(mut file) = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    else {
        return;
    };
    if file.write_all(wav).is_ok() {
        drop(file);
        #[cfg(target_os = "macos")]
        let players = ["afplay"];
        #[cfg(not(target_os = "macos"))]
        let players = ["pw-play", "paplay", "aplay"];
        for player in players {
            if Command::new(player)
                .arg(&path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
            {
                break;
            }
        }
    }
    let _ = std::fs::remove_file(path);
}

/// A short, quiet coin-in sound. macOS uses the system Glass chime at low
/// volume. Other platforms stay silent rather than pull in an audio stack.
///
/// Everything platform-specific lives inside the `cfg` block, including the
/// import and the silence guard. Written the obvious way — a top-level `use`
/// and an early `return` — this file compiled cleanly on macOS and failed CI
/// on Linux with two errors, because there the `cfg` block disappears and
/// leaves an unused import above a function whose whole body is a bare
/// trailing `return`. A guard that only guards code which does not exist on
/// this platform is not a guard; it is the thing being linted.
pub fn play_coin_chime() {
    #[cfg(target_os = "macos")]
    {
        use std::process::{Command, Stdio};

        if cfg!(test) || std::env::var_os("NIGHTFALL_UI_SHOTS").is_some() {
            return;
        }
        let _ = Command::new("afplay")
            .args(["-v", "0.28", "/System/Library/Sounds/Glass.aiff"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn coin_chime_is_silent_in_tests() {
        super::play_coin_chime();
        super::play_reward_chime();
    }

    #[test]
    fn reward_sound_is_small_valid_pcm_without_clipping_or_a_click_at_the_edges() {
        let wav = super::reward_wav();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(wav.len(), 45_644);
        assert_eq!(
            u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize,
            wav.len() - 44
        );
        let samples: Vec<_> = wav[44..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|bytes| i16::from_le_bytes(*bytes))
            .collect();
        assert_eq!(samples[0], 0);
        assert!(samples.last().unwrap().abs() < 10);
        let peak = samples.iter().map(|sample| sample.abs()).max().unwrap();
        assert!((1000..16000).contains(&peak));
    }
}
