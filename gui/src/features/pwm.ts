export function dutyPercent(pwm: number): number {
  return Math.round((pwm * 100) / 255);
}
