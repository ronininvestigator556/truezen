//! Output device enumeration and stream configuration.

use cpal::traits::{DeviceTrait, HostTrait};
use cpal::{Device, SampleFormat, StreamConfig, SupportedBufferSize};

use crate::HostError;

/// Latency is irrelevant for this app -- nothing is played in response to
/// input -- while a dropout in the middle of a forty-minute session is
/// ruinous. A large buffer is therefore free insurance.
pub const PREFERRED_FRAMES: u32 = 1024;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    /// Stable across reboots and reconnections, so this is what gets saved in
    /// settings. Display names are not unique -- two identical USB interfaces
    /// report the same string.
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

/// Every usable output device on the default host.
pub fn output_devices() -> Result<Vec<DeviceInfo>, HostError> {
    let host = cpal::default_host();
    let default_id = host
        .default_output_device()
        .and_then(|d| d.id().ok())
        .map(|i| i.to_string());

    let mut out = Vec::new();
    for device in host.output_devices().map_err(HostError::from)? {
        // A device that cannot report an output config is not usable; listing
        // it would only produce a confusing failure at selection time.
        if device.default_output_config().is_err() {
            continue;
        }
        let Ok(id) = device.id() else { continue };
        let id = id.to_string();
        out.push(DeviceInfo {
            is_default: Some(&id) == default_id.as_ref(),
            name: device.to_string(),
            id,
        });
    }
    Ok(out)
}

/// Resolve a saved device id, falling back to the system default.
///
/// The fallback is deliberate: a device that no longer exists -- a USB
/// interface left at the office -- should start the app on the built-in
/// output rather than refuse to produce sound.
pub fn resolve(id: Option<&str>) -> Result<Device, HostError> {
    let host = cpal::default_host();
    if let Some(want) = id {
        if let Ok(parsed) = want.parse::<cpal::DeviceId>() {
            if let Some(d) = host.device_by_id(&parsed) {
                if d.default_output_config().is_ok() {
                    return Ok(d);
                }
            }
        }
    }
    host.default_output_device()
        .ok_or(HostError::NoOutputDevice)
}

/// The chosen stream configuration and the sample format to render in.
pub struct ChosenConfig {
    pub config: StreamConfig,
    pub format: SampleFormat,
}

/// Build a stream config from the device's default, widened to our preferred
/// buffer size where the backend allows it.
pub fn choose_config(device: &Device) -> Result<ChosenConfig, HostError> {
    let supported = device
        .default_output_config()
        .map_err(|e| HostError::Config(e.to_string()))?;
    let format = supported.sample_format();
    let buffer_size = match supported.buffer_size() {
        SupportedBufferSize::Range { min, max } => {
            cpal::BufferSize::Fixed(PREFERRED_FRAMES.clamp(*min, *max))
        }
        // The backend picks. CoreAudio in particular often reports Unknown and
        // honours its own sensible default.
        _ => cpal::BufferSize::Default,
    };

    let mut config: StreamConfig = supported.into();
    config.buffer_size = buffer_size;
    Ok(ChosenConfig { config, format })
}
