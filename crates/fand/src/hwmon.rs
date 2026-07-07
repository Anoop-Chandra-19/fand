//! sysfs hwmon access. Devices are resolved by their `name` file — hwmon
//! indices are not stable across boots, so index-based lookup is not offered.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

#[derive(Debug)]
pub struct HwmonDevice {
    name: String,
    path: PathBuf,
}

impl HwmonDevice {
    /// Scan `root` (normally /sys/class/hwmon) for the device whose `name`
    /// file matches. Taking the root as a parameter keeps this testable
    /// against a fake directory tree.
    pub fn find_by_name(root: &Path, name: &str) -> Result<Self> {
        let entries =
            fs::read_dir(root).with_context(|| format!("reading {}", root.display()))?;
        for entry in entries {
            let path = entry?.path();
            let Ok(dev_name) = fs::read_to_string(path.join("name")) else {
                continue;
            };
            if dev_name.trim() == name {
                return Ok(Self {
                    name: name.to_string(),
                    path,
                });
            }
        }
        bail!("no hwmon device named `{name}` under {}", root.display())
    }

    /// Temperature in °C from a millidegree input file (e.g. "temp1_input").
    pub fn read_temp_c(&self, input: &str) -> Result<f64> {
        Ok(self.read_attr(input)? as f64 / 1000.0)
    }

    pub fn read_fan_rpm(&self, n: u32) -> Result<u32> {
        let v = self.read_attr(&format!("fan{n}_input"))?;
        u32::try_from(v).with_context(|| format!("{}: fan{n}_input reads {v}", self.name))
    }

    pub fn read_pwm(&self, n: u32) -> Result<u8> {
        let v = self.read_attr(&format!("pwm{n}"))?;
        u8::try_from(v).with_context(|| format!("{}: pwm{n} reads {v}, expected 0-255", self.name))
    }

    pub fn read_pwm_enable(&self, n: u32) -> Result<i64> {
        self.read_attr(&format!("pwm{n}_enable"))
    }

    pub fn write_pwm(&self, n: u32, value: u8) -> Result<()> {
        self.write_attr(&format!("pwm{n}"), &value.to_string())
    }

    pub fn write_pwm_enable(&self, n: u32, value: u8) -> Result<()> {
        self.write_attr(&format!("pwm{n}_enable"), &value.to_string())
    }

    pub fn pwm_enable_path(&self, n: u32) -> PathBuf {
        self.path.join(format!("pwm{n}_enable"))
    }

    pub fn pwm_path(&self, n: u32) -> PathBuf {
        self.path.join(format!("pwm{n}"))
    }

    /// Every pwmN_enable file the chip exposes (used by --restore-auto,
    /// which must cover channels beyond the configured ones).
    pub fn all_pwm_enable_paths(&self) -> Result<Vec<PathBuf>> {
        let entries = fs::read_dir(&self.path)
            .with_context(|| format!("reading {}", self.path.display()))?;
        let mut out = Vec::new();
        for entry in entries {
            let path = entry?.path();
            let Some(fname) = path.file_name().and_then(|f| f.to_str()) else {
                continue;
            };
            let Some(mid) = fname
                .strip_prefix("pwm")
                .and_then(|s| s.strip_suffix("_enable"))
            else {
                continue;
            };
            if !mid.is_empty() && mid.bytes().all(|b| b.is_ascii_digit()) {
                out.push(path);
            }
        }
        out.sort();
        Ok(out)
    }

    /// `attr` can come from user config (sensor `input`); refuse anything
    /// that is not a plain attribute filename so it cannot escape the
    /// device directory.
    fn attr_path(&self, attr: &str) -> Result<PathBuf> {
        if attr.is_empty() || !attr.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            bail!("invalid hwmon attribute name `{attr}`");
        }
        Ok(self.path.join(attr))
    }

    fn read_attr(&self, attr: &str) -> Result<i64> {
        let path = self.attr_path(attr)?;
        let s =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        s.trim()
            .parse()
            .with_context(|| format!("parsing {} (`{}`)", path.display(), s.trim()))
    }

    fn write_attr(&self, attr: &str, value: &str) -> Result<()> {
        let path = self.attr_path(attr)?;
        fs::write(&path, value).with_context(|| format!("writing {value} to {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fake_root() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let dev = dir.path().join("hwmon0");
        fs::create_dir(&dev).unwrap();
        fs::write(dev.join("name"), "nct6799\n").unwrap();
        fs::write(dev.join("temp1_input"), "40500\n").unwrap();
        fs::write(dev.join("fan2_input"), "789\n").unwrap();
        fs::write(dev.join("pwm2"), "128\n").unwrap();
        fs::write(dev.join("pwm2_enable"), "5\n").unwrap();
        fs::write(dev.join("pwm7_enable"), "5\n").unwrap();
        dir
    }

    fn dev(root: &tempfile::TempDir) -> HwmonDevice {
        HwmonDevice::find_by_name(root.path(), "nct6799").unwrap()
    }

    #[test]
    fn finds_device_by_name() {
        let root = fake_root();
        // Proves the lookup landed on the right directory.
        assert_eq!(dev(&root).read_pwm(2).unwrap(), 128);
    }

    #[test]
    fn missing_device_is_an_error() {
        let root = fake_root();
        assert!(HwmonDevice::find_by_name(root.path(), "k10temp").is_err());
    }

    #[test]
    fn reads_millidegrees_as_celsius() {
        let root = fake_root();
        assert_eq!(dev(&root).read_temp_c("temp1_input").unwrap(), 40.5);
    }

    #[test]
    fn reads_fan_and_pwm() {
        let root = fake_root();
        let d = dev(&root);
        assert_eq!(d.read_fan_rpm(2).unwrap(), 789);
        assert_eq!(d.read_pwm(2).unwrap(), 128);
        assert_eq!(d.read_pwm_enable(2).unwrap(), 5);
    }

    #[test]
    fn writes_pwm_and_enable() {
        let root = fake_root();
        let d = dev(&root);
        d.write_pwm(2, 200).unwrap();
        d.write_pwm_enable(2, 1).unwrap();
        assert_eq!(d.read_pwm(2).unwrap(), 200);
        assert_eq!(d.read_pwm_enable(2).unwrap(), 1);
    }

    #[test]
    fn rejects_path_escaping_attr_names() {
        let root = fake_root();
        let d = dev(&root);
        assert!(d.read_temp_c("../hwmon0/temp1_input").is_err());
        assert!(d.read_temp_c("temp1/input").is_err());
        assert!(d.read_temp_c("").is_err());
    }

    #[test]
    fn lists_all_pwm_enable_files() {
        let root = fake_root();
        let paths = dev(&root).all_pwm_enable_paths().unwrap();
        let names: Vec<_> = paths
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap().to_string())
            .collect();
        assert_eq!(names, vec!["pwm2_enable", "pwm7_enable"]);
    }
}
