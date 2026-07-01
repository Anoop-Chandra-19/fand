//! Mix mode: list of (sensor, curve) pairs; evaluate each curve at its own
//! sensor's temperature and take the **max of the resulting PWMs**.
//!
//! Deliberately max-of-outputs, not one curve fed max-temp — 70 °C means
//! different things per component.
