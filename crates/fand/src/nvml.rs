//! GPU temperature via NVML. The GPU is a temperature *input* only — its
//! own fans stay self-managed and are never controlled.

use anyhow::{Context, Result};
use nvml_wrapper::enum_wrappers::device::TemperatureSensor;
use nvml_wrapper::Nvml;

pub struct Gpu {
    nvml: Nvml,
}

impl Gpu {
    pub fn init() -> Result<Self> {
        let nvml = Nvml::init().context("initializing NVML (is the NVIDIA driver loaded?)")?;
        Ok(Self { nvml })
    }

    pub fn read_temp_c(&self, device_index: u32) -> Result<f64> {
        let device = self
            .nvml
            .device_by_index(device_index)
            .with_context(|| format!("opening NVML device {device_index}"))?;
        let temp = device
            .temperature(TemperatureSensor::Gpu)
            .with_context(|| format!("reading NVML device {device_index} temperature"))?;
        Ok(f64::from(temp))
    }
}
