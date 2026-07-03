import { useState } from "react";
import { Line, LineChart, ResponsiveContainer, YAxis } from "recharts";
import type { ChannelCurveRefs, ChannelStatus, Sample } from "../daemon/types";
import { dutyPercent } from "../daemon/types";

type WriteResult = Promise<string | null>;

interface Props {
  name: string;
  label?: string;
  channel: ChannelStatus;
  history: Sample[];
  curveRefs?: ChannelCurveRefs;
  curveNames?: string[];
  sensors?: string[];
  setChannelCurve?: (channel: string, sensor: string, curve: string) => WriteResult;
  addMixInput?: (channel: string, sensor: string, curve: string) => WriteResult;
  removeMixInput?: (channel: string, sensor: string) => WriteResult;
}

interface AssignmentProps {
  channel: string;
  curveRefs: ChannelCurveRefs;
  curveNames: string[];
  sensors: string[];
  setChannelCurve: (channel: string, sensor: string, curve: string) => WriteResult;
  addMixInput: (channel: string, sensor: string, curve: string) => WriteResult;
  removeMixInput: (channel: string, sensor: string) => WriteResult;
}

const selectClass =
  "rounded-md bg-white/10 px-2 py-1 text-[13px] outline-none focus:ring-1 focus:ring-accent";

function CurveAssignment({
  channel,
  curveRefs,
  curveNames,
  sensors,
  setChannelCurve,
  addMixInput,
  removeMixInput,
}: AssignmentProps) {
  const [error, setError] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [addSensor, setAddSensor] = useState("");
  const [addCurve, setAddCurve] = useState("");

  const usedSensors = new Set(curveRefs.refs.map((r) => r.sensor));
  const availableSensors = sensors.filter((s) => !usedSensors.has(s));

  const handleAdd = async () => {
    if (!addSensor || !addCurve) return;
    const err = await addMixInput(channel, addSensor, addCurve);
    if (err) {
      setError(err);
    } else {
      setAdding(false);
      setAddSensor("");
      setAddCurve("");
      setError(null);
    }
  };

  return (
    <div className="flex flex-col gap-1.5 border-t border-separator pt-2.5">
      {curveRefs.refs.map((ref) => (
        <div key={ref.sensor} className="flex items-center gap-2 text-[13px]">
          <span className="w-10 shrink-0 text-dim">{ref.sensor}</span>
          <select
            value={ref.curve}
            onChange={(e) =>
              void setChannelCurve(channel, ref.sensor, e.target.value).then(setError)
            }
            className={`flex-1 ${selectClass}`}
          >
            {curveNames.map((c) => (
              <option key={c} value={c}>
                {c}
              </option>
            ))}
          </select>
          {curveRefs.is_mix && curveRefs.refs.length > 1 && (
            <button
              type="button"
              onClick={() => void removeMixInput(channel, ref.sensor).then(setError)}
              className="text-dim hover:text-error"
              title="remove input"
            >
              ×
            </button>
          )}
        </div>
      ))}

      {curveRefs.is_mix &&
        (adding ? (
          <div className="flex items-center gap-2 text-[13px]">
            <select
              value={addSensor}
              onChange={(e) => setAddSensor(e.target.value)}
              className={`w-20 ${selectClass}`}
            >
              <option value="">sensor</option>
              {availableSensors.map((s) => (
                <option key={s} value={s}>
                  {s}
                </option>
              ))}
            </select>
            <select
              value={addCurve}
              onChange={(e) => setAddCurve(e.target.value)}
              className={`flex-1 ${selectClass}`}
            >
              <option value="">curve</option>
              {curveNames.map((c) => (
                <option key={c} value={c}>
                  {c}
                </option>
              ))}
            </select>
            <button type="button" onClick={() => void handleAdd()} className="text-dim hover:text-accent">
              add
            </button>
            <button
              type="button"
              onClick={() => {
                setAdding(false);
                setAddSensor("");
                setAddCurve("");
              }}
              className="text-dim hover:text-ink"
            >
              cancel
            </button>
          </div>
        ) : (
          availableSensors.length > 0 && (
            <button
              type="button"
              onClick={() => setAdding(true)}
              className="self-start text-[13px] text-dim hover:text-ink"
            >
              + add input
            </button>
          )
        ))}

      {error && <p className="text-xs text-error">{error}</p>}
    </div>
  );
}

export function ChannelCard({
  name,
  label,
  channel,
  history,
  curveRefs,
  curveNames,
  sensors,
  setChannelCurve,
  addMixInput,
  removeMixInput,
}: Props) {
  const overriding = channel.mode === "override";
  const spark = history.map((s) => ({
    at: s.at,
    pwm: s.status.channels[name]?.current_pwm ?? null,
  }));

  const canAssign = curveRefs && curveNames && sensors && setChannelCurve && addMixInput && removeMixInput;

  return (
    <article className="flex flex-col gap-3 rounded-xl bg-card px-4 py-3.5">
      <header className="flex items-start justify-between gap-2">
        <div>
          <h3 className="font-bold">{name}</h3>
          {label && <span className="text-[13px] text-dim">{label}</span>}
          {!canAssign && curveRefs && curveRefs.refs.length > 0 && (
            <div className="mt-0.5 text-[11px] text-dim">
              {curveRefs.refs.map((r) => r.curve).join(" · ")}
            </div>
          )}
        </div>
        {overriding ? (
          <span
            className="rounded-full bg-warning/15 px-2.5 py-0.5 text-xs font-bold whitespace-nowrap text-warning"
            role="status"
          >
            Override · {channel.override_remaining_s ?? 0}s left
          </span>
        ) : (
          <span className="rounded-full bg-white/10 px-2.5 py-0.5 text-xs whitespace-nowrap text-dim">
            Curve
          </span>
        )}
      </header>
      <div className="flex items-end justify-between gap-3">
        <div>
          <span className="text-4xl leading-none font-light tabular-nums">
            {dutyPercent(channel.current_pwm)}%
          </span>
          <span className="ml-1.5 text-[13px] text-dim">duty</span>
        </div>
        <dl className="flex gap-5">
          <div>
            <dt className="text-xs text-dim">Fan speed</dt>
            <dd className="text-[15px] tabular-nums">
              {channel.rpm.toLocaleString()} RPM
            </dd>
          </div>
          <div>
            <dt className="text-xs text-dim">Curve target</dt>
            <dd className="text-[15px] tabular-nums">
              {dutyPercent(channel.target_pwm)}%
            </dd>
          </div>
        </dl>
      </div>
      <div className="-mx-1.5 -mb-1.5" aria-hidden="true">
        <ResponsiveContainer width="100%" height={36}>
          <LineChart data={spark} margin={{ top: 2, right: 0, bottom: 2, left: 0 }}>
            <YAxis domain={[0, 255]} hide />
            <Line
              dataKey="pwm"
              stroke={overriding ? "var(--color-warning)" : "var(--color-accent)"}
              strokeWidth={2}
              dot={false}
              isAnimationActive={false}
            />
          </LineChart>
        </ResponsiveContainer>
      </div>
      {canAssign && (
        <CurveAssignment
          channel={name}
          curveRefs={curveRefs}
          curveNames={curveNames}
          sensors={sensors}
          setChannelCurve={setChannelCurve}
          addMixInput={addMixInput}
          removeMixInput={removeMixInput}
        />
      )}
    </article>
  );
}
