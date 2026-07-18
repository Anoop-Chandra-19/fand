import { Badge } from "../adw/Badge";
import { Card } from "../adw/Card";
import { PlusIcon } from "../adw/icons";
import type { CurveInfo } from "../daemon/types";
import { dutyPercent } from "../daemon/types";
import { evalCurve } from "../daemon/eval";
import { CurveSparkline } from "./CurveSparkline";

function DutyNow({
  output,
  caption,
  estimate = false,
}: {
  output: number | null;
  caption: string;
  /** Marks a client-side instant evaluation — the daemon's hysteresis,
   * dwell, smoothing and floors are not in this number. */
  estimate?: boolean;
}) {
  return (
    <div
      className="flex items-baseline gap-[6px]"
      title={
        estimate
          ? "instant curve output — the daemon applies hysteresis, smoothing and floors on top"
          : undefined
      }
    >
      <span className="numeric text-[28px] font-light leading-none">
        {output === null ? "—" : `${estimate ? "≈" : ""}${dutyPercent(output)}`}
        <span className="text-[1rem] text-dim"> %</span>
      </span>
      <span className="text-[0.82rem] text-dim">{caption}</span>
    </div>
  );
}

function SensorLine({ sensor, temp }: { sensor: string; temp?: number }) {
  return (
    <div className="flex items-center gap-[6px] text-[0.82rem] text-dim">
      <span>sensor</span>
      <span className="font-mono text-ink">{sensor}</span>
      <span className="numeric ml-auto text-warning">
        {temp !== undefined ? `${temp.toFixed(1)} °C` : "—"}
      </span>
    </div>
  );
}

/**
 * A Curves-section card: kind badge, used-by badges, live output, and a
 * kind-specific body. Every card opens its editor on click.
 */
export function CurveCard({
  name,
  info,
  curves,
  temps,
  usedBy,
  onEdit,
}: {
  name: string;
  info: CurveInfo;
  curves: Record<string, CurveInfo>;
  temps: Record<string, number>;
  usedBy: string[];
  onEdit: () => void;
}) {
  const output = evalCurve(curves, name, temps);
  return (
    <Card activatable onClick={onEdit} className="flex flex-col gap-3">
      <header className="flex items-start justify-between gap-2">
        <div className="flex items-center gap-2">
          <h3 className="numeric m-0 text-[1.18rem] font-bold">{name}</h3>
          <Badge tone="accent">{info.kind}</Badge>
        </div>
        <div className="flex flex-wrap justify-end gap-[5px]">
          {usedBy.length ? (
            usedBy.map((u) => <Badge key={u}>{u}</Badge>)
          ) : (
            <Badge>unused</Badge>
          )}
        </div>
      </header>

      {info.kind === "graph" && (
        <>
          <div className="flex items-center justify-between gap-2">
            <DutyNow output={output} caption="duty now · estimate" estimate />
            <span className="text-[0.82rem] text-accent">Edit curve</span>
          </div>
          <div className="rounded-[10px] bg-view px-[6px] pb-[6px] pt-2">
            <CurveSparkline
              points={info.points}
              liveTemp={temps[info.sensor]}
              height={92}
              grid
              showPoints
            />
          </div>
          <SensorLine sensor={info.sensor} temp={temps[info.sensor]} />
        </>
      )}

      {info.kind === "mix" && (
        <div className="flex flex-col gap-[10px]">
          <DutyNow
            output={output}
            caption={`${info.function} of ${info.members.length} curves · estimate`}
            estimate
          />
          <div className="flex flex-wrap gap-[6px]">
            {info.members.map((m) => (
              <Badge key={m}>{m}</Badge>
            ))}
          </div>
          {info.function !== "max" && (
            <div className="text-[0.82rem] leading-[1.4] text-warning">
              {info.function} mix — can run below the hottest member's need
            </div>
          )}
        </div>
      )}

      {info.kind === "flat" && (
        <DutyNow output={info.pwm} caption={`constant duty (pwm ${info.pwm})`} />
      )}

      {info.kind === "trigger" && (
        <div className="flex flex-col gap-[10px]">
          <div className="numeric flex flex-col gap-1 text-[0.82rem]">
            <div>
              <span className="text-dim">idle ≤ </span>
              {info.idle_temp} °C
              <span className="text-dim"> → </span>
              {dutyPercent(info.idle_pwm)} % duty
              <span className="text-dim"> (pwm {info.idle_pwm})</span>
            </div>
            <div>
              <span className="text-dim">load ≥ </span>
              {info.load_temp} °C
              <span className="text-dim"> → </span>
              {dutyPercent(info.load_pwm)} % duty
              <span className="text-dim"> (pwm {info.load_pwm})</span>
            </div>
          </div>
          <div className="text-[0.82rem] text-dim">
            {info.response_seconds > 0
              ? `switches after ${info.response_seconds} s past a threshold`
              : "switches instantly at the thresholds"}
          </div>
          <SensorLine sensor={info.sensor} temp={temps[info.sensor]} />
        </div>
      )}
    </Card>
  );
}

/** Dashed "New curve" affordance closing the Curves grid. */
export function AddCurveCard({ onClick }: { onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex min-h-[200px] cursor-pointer flex-col items-center justify-center gap-2 rounded-card border-2 border-dashed border-separator text-dim transition-colors duration-200 hover:bg-[var(--flat-hover-fill)] hover:text-ink"
    >
      <span className="flex h-[34px] w-[34px] items-center justify-center rounded-full bg-white/9">
        <PlusIcon size={18} />
      </span>
      New curve
    </button>
  );
}
