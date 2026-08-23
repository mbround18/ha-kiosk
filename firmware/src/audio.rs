//! A short click sound on button press, via this board's onboard ES8311 speaker codec
//! (`bsp_audio_codec_speaker_init`/`esp_codec_dev_*`, confirmed present in the vendored BSP -
//! see `bsp_bindings.h`). Synthesizes a brief fading tone burst rather than embedding an audio
//! file/decoder for something this simple.
//!
//! Worth watching for: this codec's control path uses the *legacy* I2C driver, while
//! touch/display on this board use the newer `i2c_master` driver - `CONFIG_I2C_SKIP_LEGACY_
//! CONFLICT_CHECK=y` was previously safe only because nothing actually drove the legacy path at
//! runtime. This is the first code to actually do so; if the two I2C generations turn out to
//! conflict for real (not just at ESP-IDF's build-time guard), this is the first place it'd show
//! up as I2C bus errors or a hang.

use esp_idf_svc::sys::*;

pub struct Audio {
    codec: esp_codec_dev_handle_t,
}

unsafe impl Send for Audio {}

impl Audio {
    pub fn init() -> anyhow::Result<Self> {
        unsafe {
            esp!(bsp_audio_init(std::ptr::null()))?;

            let codec = bsp_audio_codec_speaker_init();
            anyhow::ensure!(!codec.is_null(), "bsp_audio_codec_speaker_init returned null");

            let mut fs = esp_codec_dev_sample_info_t {
                bits_per_sample: 16,
                channel: 1,
                channel_mask: 0,
                sample_rate: 16000,
                mclk_multiple: 0,
            };
            let ret = esp_codec_dev_open(codec, &mut fs);
            anyhow::ensure!(ret == 0, "esp_codec_dev_open failed: {ret}");
            esp_codec_dev_set_out_vol(codec, 70);
            // The board's speaker amplifier (a separate enable GPIO, driven by the ES8311
            // driver's mute/unmute logic - see vendor/esp32_p4_function_ev_board's `pa_pin`
            // wiring) stays disabled until explicitly unmuted here; opening the codec device
            // alone leaves it silent even though I2S transmits without any error.
            esp_codec_dev_set_out_mute(codec, false);

            Ok(Self { codec })
        }
    }

    /// A brief mid-range tone burst with a linear fade-out (avoids an audible pop from an
    /// abrupt stop).
    pub fn click(&mut self) {
        const SAMPLE_RATE: u32 = 16000;
        const DURATION_MS: u32 = 30;
        const FREQ_HZ: f32 = 1200.0;

        let n = (SAMPLE_RATE * DURATION_MS / 1000) as usize;
        let mut samples = vec![0i16; n];
        for (i, s) in samples.iter_mut().enumerate() {
            let t = i as f32 / SAMPLE_RATE as f32;
            let fade = 1.0 - (i as f32 / n as f32);
            let amplitude = 12000.0 * fade;
            *s = (amplitude * (2.0 * std::f32::consts::PI * FREQ_HZ * t).sin()) as i16;
        }

        unsafe {
            esp_codec_dev_write(
                self.codec,
                samples.as_mut_ptr() as *mut core::ffi::c_void,
                (samples.len() * 2) as i32,
            );
        }
    }
}
