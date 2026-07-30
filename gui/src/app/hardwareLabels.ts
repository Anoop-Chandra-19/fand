// Friendly names for this machine's hardware; unknown names fall back to
// the raw identifier — which always stays visible next to the label.
export const CHANNEL_LABELS: Record<string, string> = {
  pwm1: "CPU radiator · AIO pump",
  pwm2: "Case fans",
};

export const SENSOR_LABELS: Record<string, string> = {
  cpu: "CPU · Core (Tctl) — Ryzen 7 7800X3D",
  gpu: "GPU — NVIDIA GeForce RTX 4090",
};
