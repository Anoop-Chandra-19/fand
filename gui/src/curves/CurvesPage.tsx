import { useState } from "react";
import type { CurveEditorPayload, CurveInfo, CurvePoint } from "../daemon/types";
import { dutyPercent } from "../daemon/types";
import { CurveEditor } from "./CurveEditor";

type WriteResult = Promise<string | null>;

interface Props {
  data: CurveEditorPayload | null;
  temps: Record<string, number>;
  setCurvePoints: (name: string, points: CurvePoint[]) => WriteResult;
  createGraphCurve: (name: string, sensor: string, points: CurvePoint[]) => WriteResult;
  setGraphSensor: (name: string, sensor: string) => WriteResult;
  addMixMember: (name: string, member: string) => WriteResult;
  removeMixMember: (name: string, member: string) => WriteResult;
  deleteCurve: (name: string) => WriteResult;
}

const STARTER_POINTS: CurvePoint[] = [
  [40, 80],
  [80, 255],
];

const selectClass =
  "rounded-md bg-white/10 px-2 py-1 text-[13px] outline-none focus:ring-1 focus:ring-accent";

/** Channels bound to this curve, plus mix curves that include it. */
function usedBy(curveName: string, data: CurveEditorPayload): string[] {
  const users: string[] = [];
  for (const [channel, bound] of Object.entries(data.channels)) {
    if (bound === curveName) users.push(channel);
  }
  for (const [name, info] of Object.entries(data.curves)) {
    if (info.kind === "mix" && info.members.includes(curveName)) users.push(name);
  }
  return users;
}

function GraphCurveBody({
  name,
  info,
  temps,
  sensors,
  setCurvePoints,
  setGraphSensor,
  onError,
}: {
  name: string;
  info: Extract<CurveInfo, { kind: "graph" }>;
  temps: Record<string, number>;
  sensors: string[];
  setCurvePoints: Props["setCurvePoints"];
  setGraphSensor: Props["setGraphSensor"];
  onError: (e: string | null) => void;
}) {
  return (
    <>
      <div className="flex items-center gap-2 text-[13px]">
        <span className="shrink-0 text-dim">sensor</span>
        <select
          value={info.sensor}
          onChange={(e) => void setGraphSensor(name, e.target.value).then(onError)}
          className={selectClass}
        >
          {sensors.map((s) => (
            <option key={s} value={s}>
              {s}
            </option>
          ))}
        </select>
        {temps[info.sensor] !== undefined && (
          <span className="text-dim tabular-nums">{temps[info.sensor].toFixed(1)} °C</span>
        )}
      </div>
      <CurveEditor
        points={info.points}
        liveTemp={temps[info.sensor]}
        onCommit={(next) => setCurvePoints(name, next)}
      />
    </>
  );
}

// Read-only until the phase-10 editor gains trigger controls.
function TriggerCurveBody({
  info,
  temps,
}: {
  info: Extract<CurveInfo, { kind: "trigger" }>;
  temps: Record<string, number>;
}) {
  return (
    <div className="flex flex-col gap-1 text-[13px]">
      <div className="flex items-center gap-2">
        <span className="text-dim">sensor</span>
        <span>{info.sensor}</span>
        {temps[info.sensor] !== undefined && (
          <span className="text-dim tabular-nums">{temps[info.sensor].toFixed(1)} °C</span>
        )}
      </div>
      <p className="text-dim">
        idle <span className="tabular-nums">≤ {info.idle_temp} °C</span> →{" "}
        <span className="tabular-nums">{dutyPercent(info.idle_pwm)}%</span> duty (pwm{" "}
        <span className="tabular-nums">{info.idle_pwm}</span>)
      </p>
      <p className="text-dim">
        load <span className="tabular-nums">≥ {info.load_temp} °C</span> →{" "}
        <span className="tabular-nums">{dutyPercent(info.load_pwm)}%</span> duty (pwm{" "}
        <span className="tabular-nums">{info.load_pwm}</span>)
      </p>
      <p className="text-dim">
        {info.response_seconds > 0 ? (
          <>
            switches after <span className="tabular-nums">{info.response_seconds} s</span> past a
            threshold
          </>
        ) : (
          "switches instantly at the thresholds"
        )}
      </p>
    </div>
  );
}

function MixCurveBody({
  name,
  info,
  curveNames,
  addMixMember,
  removeMixMember,
  onError,
}: {
  name: string;
  info: Extract<CurveInfo, { kind: "mix" }>;
  curveNames: string[];
  addMixMember: Props["addMixMember"];
  removeMixMember: Props["removeMixMember"];
  onError: (e: string | null) => void;
}) {
  // A mix can include any curve except itself and current members; deeper
  // cycles are rejected by the daemon and surface via onError.
  const candidates = curveNames.filter((c) => c !== name && !info.members.includes(c));

  return (
    <div className="flex flex-col gap-2">
      <span className="text-[13px] text-dim">
        {info.function} of {info.members.length} curve{info.members.length === 1 ? "" : "s"}
      </span>
      <div className="flex flex-wrap gap-1.5">
        {info.members.map((member) => (
          <span
            key={member}
            className="flex items-center gap-1.5 rounded-full bg-white/10 px-2.5 py-0.5 text-xs whitespace-nowrap"
          >
            {member}
            {info.members.length > 1 && (
              <button
                type="button"
                onClick={() => void removeMixMember(name, member).then(onError)}
                className="text-dim hover:text-error"
                title="remove from mix"
              >
                ×
              </button>
            )}
          </span>
        ))}
      </div>
      {candidates.length > 0 && (
        <select
          value=""
          onChange={(e) => {
            if (e.target.value) void addMixMember(name, e.target.value).then(onError);
          }}
          className={`self-start ${selectClass}`}
        >
          <option value="">+ add curve…</option>
          {candidates.map((c) => (
            <option key={c} value={c}>
              {c}
            </option>
          ))}
        </select>
      )}
    </div>
  );
}

export function CurvesPage({
  data,
  temps,
  setCurvePoints,
  createGraphCurve,
  setGraphSensor,
  addMixMember,
  removeMixMember,
  deleteCurve,
}: Props) {
  const [creating, setCreating] = useState(false);
  const [newName, setNewName] = useState("");
  const [newSensor, setNewSensor] = useState("");
  const [createError, setCreateError] = useState<string | null>(null);
  const [cardErrors, setCardErrors] = useState<Record<string, string>>({});

  if (!data) {
    return <p className="py-12 text-center text-dim">Loading curves…</p>;
  }

  const curveNames = Object.keys(data.curves);
  const setCardError = (name: string) => (err: string | null) =>
    setCardErrors((prev) => ({ ...prev, [name]: err ?? "" }));

  const resetCreate = () => {
    setCreating(false);
    setNewName("");
    setNewSensor("");
    setCreateError(null);
  };

  const startCreate = async () => {
    const name = newName.trim();
    const sensor = newSensor || data.sensors[0];
    if (!name || !sensor) return;
    if (data.curves[name]) {
      setCreateError(`a curve named "${name}" already exists`);
      return;
    }
    const err = await createGraphCurve(name, sensor, STARTER_POINTS);
    if (err) {
      setCreateError(err);
    } else {
      resetCreate();
    }
  };

  return (
    <section className="grid grid-cols-[repeat(auto-fit,minmax(340px,1fr))] gap-5">
      {Object.entries(data.curves).map(([name, info]) => {
        const users = usedBy(name, data);
        const inUse = users.length > 0;
        return (
          <article key={name} className="flex flex-col gap-3 rounded-xl bg-card px-5 py-4">
            <header className="flex items-start justify-between gap-2">
              <div className="flex items-center gap-1.5">
                <h3 className="font-bold">{name}</h3>
                <span className="rounded-full bg-white/10 px-2 py-0.5 text-[11px] text-dim">
                  {info.kind}
                </span>
                <button
                  type="button"
                  disabled={inUse}
                  title={inUse ? "remove it from the channels/mixes using it first" : "delete curve"}
                  onClick={() => void deleteCurve(name).then(setCardError(name))}
                  className="text-dim hover:text-error disabled:cursor-not-allowed disabled:opacity-30 disabled:hover:text-dim"
                >
                  ×
                </button>
              </div>
              <div className="flex flex-wrap justify-end gap-1">
                {users.length > 0 ? (
                  users.map((u) => (
                    <span
                      key={u}
                      className="rounded-full bg-white/10 px-2.5 py-0.5 text-xs whitespace-nowrap text-dim"
                    >
                      {u}
                    </span>
                  ))
                ) : (
                  <span className="rounded-full bg-white/10 px-2.5 py-0.5 text-xs whitespace-nowrap text-dim">
                    unused
                  </span>
                )}
              </div>
            </header>

            {info.kind === "graph" && (
              <GraphCurveBody
                name={name}
                info={info}
                temps={temps}
                sensors={data.sensors}
                setCurvePoints={setCurvePoints}
                setGraphSensor={setGraphSensor}
                onError={setCardError(name)}
              />
            )}
            {info.kind === "mix" && (
              <MixCurveBody
                name={name}
                info={info}
                curveNames={curveNames}
                addMixMember={addMixMember}
                removeMixMember={removeMixMember}
                onError={setCardError(name)}
              />
            )}
            {info.kind === "flat" && (
              <p className="text-[13px] text-dim">
                constant <span className="tabular-nums">{dutyPercent(info.pwm)}%</span> duty
                (pwm <span className="tabular-nums">{info.pwm}</span>)
              </p>
            )}
            {info.kind === "trigger" && <TriggerCurveBody info={info} temps={temps} />}

            {cardErrors[name] && <p className="text-xs text-error">{cardErrors[name]}</p>}
          </article>
        );
      })}

      <article className="flex flex-col gap-3 rounded-xl border border-dashed border-separator px-5 py-4">
        {creating ? (
          <>
            <input
              autoFocus
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") void startCreate();
                if (e.key === "Escape") resetCreate();
              }}
              placeholder="curve name"
              className="rounded-md bg-white/10 px-2.5 py-1.5 text-sm outline-none focus:ring-1 focus:ring-accent"
            />
            <div className="flex items-center gap-2 text-[13px]">
              <span className="shrink-0 text-dim">sensor</span>
              <select
                value={newSensor || data.sensors[0] || ""}
                onChange={(e) => setNewSensor(e.target.value)}
                className={`flex-1 ${selectClass}`}
              >
                {data.sensors.map((s) => (
                  <option key={s} value={s}>
                    {s}
                  </option>
                ))}
              </select>
            </div>
            <div className="flex gap-2">
              <button
                type="button"
                onClick={() => void startCreate()}
                className="rounded-full bg-accent/15 px-2.5 py-0.5 text-xs whitespace-nowrap text-accent hover:bg-accent/25"
              >
                Create
              </button>
              <button
                type="button"
                onClick={resetCreate}
                className="rounded-full bg-white/10 px-2.5 py-0.5 text-xs whitespace-nowrap text-dim hover:bg-white/15"
              >
                Cancel
              </button>
            </div>
            {createError && <p className="text-xs text-error">{createError}</p>}
          </>
        ) : (
          <button
            type="button"
            onClick={() => setCreating(true)}
            className="flex h-full min-h-[120px] items-center justify-center text-sm text-dim hover:text-ink"
          >
            + New curve
          </button>
        )}
      </article>
    </section>
  );
}
