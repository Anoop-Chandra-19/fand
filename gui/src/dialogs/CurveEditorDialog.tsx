import { useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import { Button } from "../adw/Button";
import { CloseButton, Dialog, DialogHeader } from "../adw/Dialog";
import { PlusIcon, TrashIcon } from "../adw/icons";
import { ActionRow, BoxedList, ComboRow, SpinRow, Switch } from "../adw/rows";
import type { CurveInfo, CurvePoint, WriteResult } from "../daemon/types";
import { dutyPercent } from "../daemon/types";

/** Writes shared by every editor variant; each resolves to a WriteResult. */
export interface CurveWrites {
  applyGraphCurve: (
    name: string,
    sensor: string,
    points: CurvePoint[],
    hysteresisUp: number,
    hysteresisDown: number,
    responseSeconds: number,
  ) => Promise<WriteResult>;
  setFlatPwm: (name: string, pwm: number) => Promise<WriteResult>;
  setMixFunction: (name: string, fn: string) => Promise<WriteResult>;
  addMixMember: (name: string, member: string) => Promise<WriteResult>;
  removeMixMember: (name: string, member: string) => Promise<WriteResult>;
  applyTriggerCurve: (
    name: string,
    sensor: string,
    idleTemp: number,
    idlePwm: number,
    loadTemp: number,
    loadPwm: number,
    responseSeconds: number,
  ) => Promise<WriteResult>;
  deleteCurve: (name: string) => Promise<WriteResult>;
}

const groupTitle = "px-1 text-[0.82rem] font-bold tracking-[0.02em] text-dim";
const hint = "px-1 text-[0.82rem] leading-[1.4] text-dim";

function ErrorLine({ error }: { error: string | null }) {
  if (!error) return null;
  return <div className="px-1 text-[0.82rem] leading-[1.4] text-error">{error}</div>;
}

function DeleteCurveRow({
  name,
  usedBy,
  onDone,
  writes,
  onError,
}: {
  name: string;
  usedBy: string[];
  onDone: (message: string) => void;
  writes: CurveWrites;
  onError: (error: string) => void;
}) {
  const inUse = usedBy.length > 0;
  return (
    <BoxedList>
      <ActionRow
        title="Delete curve"
        subtitle={
          inUse ? "remove it from the channels/mixes using it first" : "cannot be undone"
        }
        activatable={!inUse}
        onClick={() => {
          void writes.deleteCurve(name).then(({ error, warning }) => {
            if (error) onError(error);
            else onDone(warning ?? `Curve ${name} deleted`);
          });
        }}
        trailing={
          <span
            className={`inline-flex items-center gap-[6px] text-[0.82rem] font-bold ${
              inUse ? "text-dim" : "text-error"
            }`}
          >
            <TrashIcon /> Delete
          </span>
        }
      />
    </BoxedList>
  );
}

/** Dispatches to the kind-specific editor. */
export function CurveEditorDialog({
  name,
  info,
  temps,
  sensors,
  curveNames,
  usedBy,
  writes,
  onDone,
  onClose,
}: {
  name: string;
  info: CurveInfo;
  temps: Record<string, number>;
  sensors: string[];
  curveNames: string[];
  usedBy: string[];
  writes: CurveWrites;
  /** Called with a toast message after a successful apply/delete. */
  onDone: (message: string) => void;
  onClose: () => void;
}) {
  const shared = { name, usedBy, writes, onDone, onClose };
  if (info.kind === "graph") return <GraphEditor {...shared} info={info} temps={temps} sensors={sensors} />;
  if (info.kind === "mix") return <MixEditor {...shared} info={info} curveNames={curveNames} />;
  if (info.kind === "flat") return <FlatEditor {...shared} info={info} />;
  return <TriggerEditor {...shared} info={info} temps={temps} sensors={sensors} />;
}

interface SharedProps {
  name: string;
  usedBy: string[];
  writes: CurveWrites;
  onDone: (message: string) => void;
  onClose: () => void;
}

// ---------------------------------------------------------------- graph

function GraphEditor({
  name,
  info,
  temps,
  sensors,
  usedBy,
  writes,
  onDone,
  onClose,
}: SharedProps & {
  info: Extract<CurveInfo, { kind: "graph" }>;
  temps: Record<string, number>;
  sensors: string[];
}) {
  const [pts, setPts] = useState<CurvePoint[]>(info.points.map((p) => [...p] as CurvePoint));
  const [sensor, setSensor] = useState(info.sensor);
  const [hystUp, setHystUp] = useState(info.hysteresis_up);
  const [hystDown, setHystDown] = useState(info.hysteresis_down);
  const [response, setResponse] = useState(info.response_seconds);
  const [sel, setSel] = useState(Math.min(1, info.points.length - 1));
  // Dragging is relative: the point moves by the pointer's delta from the
  // grab position, so picking a point up never snaps it to the cursor.
  const [drag, setDrag] = useState<{
    index: number;
    grabT: number;
    grabP: number;
    startX: number;
    startY: number;
  } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const svgRef = useRef<SVGSVGElement>(null);

  const W = 600;
  const H = 384;
  const PADL = 46;
  const PADR = 18;
  const PADT = 18;
  const PADB = 34;
  const iw = W - PADL - PADR;
  const ih = H - PADT - PADB;
  // Absolute bounds a point's temperature may take: the failsafe fires at
  // ≥ 115 °C, so 114 is the last degree a curve can ever act on.
  const T_LO = 0;
  const T_HI = 114;
  // The 20–100 °C base domain widens (in 10° steps) to fit off-range
  // points — they are legal config and must stay visible and editable.
  // Dragging is clamped to the visible frame, so only the spin row can
  // push a point past the current domain (no feedback loop mid-drag).
  let tMin = 20;
  let tMax = 100;
  for (const [t] of pts) {
    tMin = Math.min(tMin, Math.floor(t / 10) * 10);
    tMax = Math.max(tMax, Math.ceil(t / 10) * 10);
  }
  const tTicks: number[] = [];
  for (let t = tMin; t <= tMax; t += 20) tTicks.push(t);
  const X = (t: number) => PADL + ((t - tMin) / (tMax - tMin)) * iw;
  const Y = (p: number) => PADT + (1 - p / 255) * ih;
  const invX = (px: number) => Math.round(tMin + ((px - PADL) / iw) * (tMax - tMin));
  const invY = (py: number) => Math.round((1 - (py - PADT) / ih) * 255);
  // Map client coordinates into viewBox space through the SVG's real
  // transform — a bounding-rect ratio ignores the preserveAspectRatio
  // letterboxing and lands points offset from the cursor.
  const toLocal = (e: ReactPointerEvent): [number, number] => {
    const ctm = svgRef.current?.getScreenCTM();
    if (!ctm) return [0, 0];
    const p = new DOMPoint(e.clientX, e.clientY).matrixTransform(ctm.inverse());
    return [p.x, p.y];
  };

  const onMove = (e: ReactPointerEvent) => {
    if (drag === null) return;
    const [lx, ly] = toLocal(e);
    const { index, grabT, grabP, startX, startY } = drag;
    let t = Math.round(grabT + ((lx - startX) / iw) * (tMax - tMin));
    const p = Math.min(255, Math.max(0, Math.round(grabP - ((ly - startY) / ih) * 255)));
    const lo = index === 0 ? tMin : pts[index - 1][0] + 1;
    const hi = index === pts.length - 1 ? tMax : pts[index + 1][0] - 1;
    t = Math.min(Math.max(t, lo), Math.max(lo, hi));
    setPts((prev) => prev.map((pt, i) => (i === index ? [t, p] : pt)));
  };

  const setPoint = (key: "t" | "d", val: number) =>
    setPts((prev) =>
      prev.map((pt, i) => {
        if (i !== sel) return pt;
        if (key === "t") {
          const lo = i === 0 ? T_LO : prev[i - 1][0] + 1;
          const hi = i === prev.length - 1 ? T_HI : prev[i + 1][0] - 1;
          return [Math.min(Math.max(val, lo), hi), pt[1]];
        }
        return [pt[0], Math.round((val / 100) * 255)];
      }),
    );

  const removePoint = () => {
    if (pts.length <= 2) return;
    setPts((prev) => prev.filter((_, i) => i !== sel));
    setSel((s) => Math.max(0, s - 1));
  };

  const addPoint = () =>
    setPts((prev) => {
      let gap = 0;
      let at = 1;
      for (let i = 1; i < prev.length; i++) {
        if (prev[i][0] - prev[i - 1][0] > gap) {
          gap = prev[i][0] - prev[i - 1][0];
          at = i;
        }
      }
      const np: CurvePoint = [
        Math.round((prev[at - 1][0] + prev[at][0]) / 2),
        Math.round((prev[at - 1][1] + prev[at][1]) / 2),
      ];
      setSel(at);
      return [...prev.slice(0, at), np, ...prev.slice(at)];
    });

  // Click an empty spot on the graph to drop a point there, then keep dragging.
  const addAtPointer = (e: ReactPointerEvent) => {
    const [lx, ly] = toLocal(e);
    const t = Math.min(tMax, Math.max(tMin, invX(lx)));
    const p = Math.min(255, Math.max(0, invY(ly)));
    const idx = pts.filter((pt) => pt[0] < t).length;
    setPts((prev) => {
      const arr = [...prev];
      arr.splice(idx, 0, [t, p]);
      return arr;
    });
    setSel(idx);
    setDrag({ index: idx, grabT: t, grabP: p, startX: lx, startY: ly });
    try {
      (e.currentTarget as Element).setPointerCapture(e.pointerId);
    } catch {
      // pointer capture is best-effort
    }
  };

  const apply = () => {
    void writes
      .applyGraphCurve(name, sensor, pts, hystUp, hystDown, response)
      .then(({ error, warning }) => {
        if (error) setError(error);
        else onDone(warning ?? `Curve ${name} applied`);
      });
  };

  const path =
    `M${X(tMin).toFixed(1)},${Y(pts[0][1]).toFixed(1)} ` +
    pts.map(([t, p]) => `L${X(t).toFixed(1)},${Y(p).toFixed(1)}`).join(" ") +
    ` L${X(tMax).toFixed(1)},${Y(pts[pts.length - 1][1]).toFixed(1)}`;
  const area = `${path} L${X(tMax).toFixed(1)},${(H - PADB).toFixed(1)} L${X(tMin).toFixed(1)},${(H - PADB).toFixed(1)} Z`;
  const live = temps[sensor];
  const showLive = live !== undefined;
  // Off-scale readings clamp to the frame edge but keep the true value on
  // the label — hiding a valid temperature would be worse than a marker
  // parked at the border.
  const liveX = showLive ? X(Math.min(tMax, Math.max(tMin, live))) : 0;
  const selPt = pts[sel] ?? pts[0];

  return (
    <Dialog width={884} label={`Edit graph curve ${name}`} onClose={onClose}>
      <DialogHeader
        left={<Button variant="flat" onClick={onClose}>Cancel</Button>}
        title={name}
        subtitle="graph curve"
        mono
        right={<Button variant="suggested" onClick={apply}>Apply</Button>}
      />
      <div className="flex min-h-0 flex-1">
        <div className="flex min-w-0 flex-1 flex-col gap-[10px] py-4 pl-4 pr-2">
          <div className="min-h-[300px] flex-1 rounded-[10px] bg-view p-1">
            <svg
              ref={svgRef}
              viewBox={`0 0 ${W} ${H}`}
              width="100%"
              height="100%"
              preserveAspectRatio="xMidYMid meet"
              onPointerDown={addAtPointer}
              onPointerMove={onMove}
              onPointerUp={() => setDrag(null)}
              onPointerLeave={() => setDrag(null)}
              className="block cursor-crosshair touch-none select-none"
            >
              {[0, 20, 40, 60, 80, 100].map((p) => (
                <g key={`h${p}`}>
                  <line
                    x1={PADL}
                    y1={Y((p / 100) * 255)}
                    x2={W - PADR}
                    y2={Y((p / 100) * 255)}
                    stroke="var(--color-separator)"
                  />
                  <text
                    x={PADL - 8}
                    y={Y((p / 100) * 255) + 3.5}
                    fontSize="10.5"
                    fill="var(--color-dim)"
                    textAnchor="end"
                    style={{ fontVariantNumeric: "tabular-nums", fontFamily: "var(--font-mono)" }}
                  >
                    {p}%
                  </text>
                </g>
              ))}
              {tTicks.map((t) => (
                <g key={`v${t}`}>
                  <line x1={X(t)} y1={PADT} x2={X(t)} y2={H - PADB} stroke="var(--color-separator)" />
                  <text
                    x={X(t)}
                    y={H - PADB + 17}
                    fontSize="10.5"
                    fill="var(--color-dim)"
                    textAnchor="middle"
                    style={{ fontVariantNumeric: "tabular-nums", fontFamily: "var(--font-mono)" }}
                  >
                    {t}°
                  </text>
                </g>
              ))}
              {showLive && (
                <g>
                  <line
                    x1={liveX}
                    y1={PADT}
                    x2={liveX}
                    y2={H - PADB}
                    stroke="var(--color-warning)"
                    strokeDasharray="3,3"
                    opacity="0.75"
                  />
                  <rect
                    x={liveX - 26}
                    y={PADT - 2}
                    width={52}
                    height={15}
                    rx={4}
                    fill="var(--color-warning-bg)"
                    opacity="0.9"
                  />
                  <text
                    x={liveX}
                    y={PADT + 8.5}
                    fontSize="9.5"
                    fill="#fff"
                    textAnchor="middle"
                    style={{ fontVariantNumeric: "tabular-nums", fontFamily: "var(--font-mono)" }}
                  >
                    {live.toFixed(1)}°C
                  </text>
                </g>
              )}
              <path d={area} fill="var(--color-accent)" opacity="0.09" />
              <path
                d={path}
                fill="none"
                stroke="var(--color-accent)"
                strokeWidth={2.2}
                strokeLinejoin="round"
              />
              {pts.map(([t, p], i) => (
                <g key={i}>
                  {i === sel && (
                    <circle cx={X(t)} cy={Y(p)} r={11} fill="var(--color-accent)" opacity="0.18" />
                  )}
                  <circle
                    cx={X(t)}
                    cy={Y(p)}
                    r={i === sel ? 6.5 : 5}
                    fill={i === sel ? "var(--color-accent)" : "var(--color-view)"}
                    stroke="var(--color-accent)"
                    strokeWidth={2}
                    className="cursor-grab"
                    onPointerDown={(e) => {
                      e.stopPropagation();
                      (e.currentTarget as Element).setPointerCapture(e.pointerId);
                      setSel(i);
                      const [lx, ly] = toLocal(e);
                      setDrag({ index: i, grabT: t, grabP: p, startX: lx, startY: ly });
                    }}
                  />
                </g>
              ))}
            </svg>
          </div>
          <div className="flex items-center gap-[10px]">
            <span className="text-[0.82rem] text-dim">
              {pts.length} points · click to add, drag to reshape
            </span>
            <div className="ml-auto">
              <Button variant="flat" onClick={addPoint}>
                <PlusIcon /> Add point
              </Button>
            </div>
          </div>
        </div>

        <div className="flex w-[296px] shrink-0 flex-col gap-[18px] overflow-auto border-l border-separator bg-white/2 p-4">
          <div className="flex flex-col gap-2">
            <div className={groupTitle}>Source</div>
            <BoxedList>
              <ComboRow title="Temperature source" value={sensor} options={sensors} onChange={setSensor} />
              <ActionRow
                title="Live reading"
                trailing={
                  <span className="numeric">{showLive ? live.toFixed(1) : "—"} °C</span>
                }
              />
            </BoxedList>
          </div>

          <div className="flex flex-col gap-2">
            <div className={groupTitle}>Selected point</div>
            <BoxedList>
              <SpinRow
                title="Temperature"
                value={selPt[0]}
                min={T_LO}
                max={T_HI}
                unit="°C"
                onChange={(v) => setPoint("t", v)}
              />
              <SpinRow
                title="Fan duty"
                value={dutyPercent(selPt[1])}
                min={0}
                max={100}
                unit="%"
                onChange={(v) => setPoint("d", v)}
              />
              <ActionRow
                title="Remove point"
                subtitle={pts.length <= 2 ? "keep at least two" : "delete from this curve"}
                activatable={pts.length > 2}
                onClick={removePoint}
                trailing={
                  <span
                    className={`inline-flex items-center gap-[6px] text-[0.82rem] font-bold ${
                      pts.length <= 2 ? "text-dim" : "text-error"
                    }`}
                  >
                    <TrashIcon /> Remove
                  </span>
                }
              />
            </BoxedList>
            <div className={hint}>
              duty {dutyPercent(selPt[1])} % = pwm {selPt[1]} at {selPt[0]} °C
            </div>
          </div>

          <div className="flex flex-col gap-2">
            <div className={groupTitle}>Response</div>
            <BoxedList>
              <SpinRow
                title="Hysteresis up"
                subtitle="rise needed before re-evaluating"
                value={hystUp}
                min={0}
                max={20}
                step={0.5}
                unit="°C"
                onChange={(v) => setHystUp(Math.round(v * 10) / 10)}
              />
              <SpinRow
                title="Hysteresis down"
                subtitle="drop needed before re-evaluating"
                value={hystDown}
                min={0}
                max={20}
                step={0.5}
                unit="°C"
                onChange={(v) => setHystDown(Math.round(v * 10) / 10)}
              />
              <SpinRow
                title="Response"
                subtitle="dwell past a threshold · 0 = instant"
                value={response}
                min={0}
                max={600}
                unit="s"
                onChange={setResponse}
              />
            </BoxedList>
          </div>

          <DeleteCurveRow name={name} usedBy={usedBy} writes={writes} onDone={onDone} onError={setError} />
          <ErrorLine error={error} />
          <div className={`${hint} mt-auto pt-1`}>
            Changes apply as a batch — no half-edited curve reaches the hardware.
          </div>
        </div>
      </div>
    </Dialog>
  );
}

// ------------------------------------------------------------------ mix

const MIX_SUBTITLES: Record<string, string> = {
  max: "the safety default — follows the hottest member",
  min: "opt-in: can under-cool the hotter component",
  average: "opt-in: can run below the hottest member's need",
};

function MixEditor({
  name,
  info,
  curveNames,
  usedBy,
  writes,
  onDone,
  onClose,
}: SharedProps & { info: Extract<CurveInfo, { kind: "mix" }>; curveNames: string[] }) {
  // Instant-apply rows: errors in red; applied-with-caveat warnings in
  // the dim style — a warning is a success and must not read as failure.
  const [note, setNote] = useState<{ error: boolean; text: string } | null>(null);
  const report = ({ error, warning }: WriteResult) =>
    setNote(
      error
        ? { error: true, text: error }
        : warning
          ? { error: false, text: warning }
          : null,
    );
  // Every other curve is a candidate; the daemon rejects cycles and
  // dropping the last member, and those errors surface inline.
  const candidates = curveNames.filter((c) => c !== name);
  return (
    <Dialog width={468} label={`Edit mix curve ${name}`} onClose={onClose}>
      <DialogHeader
        left={<div className="min-w-[34px]" />}
        title={name}
        subtitle="mix curve · changes apply instantly"
        mono
        right={<CloseButton onClose={onClose} />}
      />
      <div className="flex flex-col gap-[18px] overflow-auto p-4">
        <div className="flex flex-col gap-2">
          <div className={groupTitle}>Function</div>
          <BoxedList>
            <ComboRow
              title="Combine outputs with"
              subtitle={MIX_SUBTITLES[info.function]}
              value={info.function}
              options={["max", "min", "average"]}
              onChange={(fn) => void writes.setMixFunction(name, fn).then(report)}
            />
          </BoxedList>
          <div className={hint}>
            Members are evaluated at their own sensors; only their outputs combine.
          </div>
        </div>
        <div className="flex flex-col gap-2">
          <div className={groupTitle}>Members</div>
          <BoxedList>
            {candidates.length === 0 ? (
              <ActionRow title="No curves to combine" subtitle="create another curve first" />
            ) : (
              candidates.map((m) => (
                <ActionRow
                  key={m}
                  title={<span className="font-mono">{m}</span>}
                  trailing={
                    <Switch
                      checked={info.members.includes(m)}
                      ariaLabel={`Include ${m} in this mix`}
                      onChange={(on) =>
                        void (on
                          ? writes.addMixMember(name, m)
                          : writes.removeMixMember(name, m)
                        ).then(report)
                      }
                    />
                  }
                />
              ))
            )}
          </BoxedList>
        </div>
        <DeleteCurveRow
          name={name}
          usedBy={usedBy}
          writes={writes}
          onDone={onDone}
          onError={(e) => setNote({ error: true, text: e })}
        />
        <ErrorLine error={note?.error ? note.text : null} />
        {note && !note.error && <div className={hint}>Applied — {note.text}</div>}
      </div>
    </Dialog>
  );
}

// ----------------------------------------------------------------- flat

function FlatEditor({
  name,
  info,
  usedBy,
  writes,
  onDone,
  onClose,
}: SharedProps & { info: Extract<CurveInfo, { kind: "flat" }> }) {
  // The raw pwm is the state; percent is only its display. Converting at
  // dialog-open and back at apply would re-quantize an untouched value
  // (pwm 80 → 31 % → 79) — Apply must never change what the user didn't.
  const [pwm, setPwm] = useState(info.pwm);
  const [error, setError] = useState<string | null>(null);
  const duty = dutyPercent(pwm);
  const apply = () => {
    void writes.setFlatPwm(name, pwm).then(({ error, warning }) => {
      if (error) setError(error);
      else onDone(warning ?? `Curve ${name} applied`);
    });
  };
  return (
    <Dialog width={468} label={`Edit flat curve ${name}`} onClose={onClose}>
      <DialogHeader
        left={<Button variant="flat" onClick={onClose}>Cancel</Button>}
        title={name}
        subtitle="flat curve"
        mono
        right={<Button variant="suggested" onClick={apply}>Apply</Button>}
      />
      <div className="flex flex-col gap-[18px] overflow-auto p-4">
        <div className="flex flex-col items-center gap-3 rounded-card bg-card px-4 py-6 shadow-card">
          <span className="numeric text-[46px] font-light leading-none">
            {duty}
            <span className="text-[1.18rem] text-dim"> %</span>
          </span>
          <input
            type="range"
            min={0}
            max={100}
            aria-label="Constant duty"
            value={duty}
            onChange={(e) => setPwm(Math.round((Number(e.target.value) / 100) * 255))}
            className="w-full"
            style={{ accentColor: "var(--color-accent-bg)" }}
          />
          <span className="text-[0.82rem] text-dim">constant {duty} % duty (pwm {pwm})</span>
        </div>
        <div className={hint}>
          The channel's min-duty floor still applies — a flat curve below the floor runs at the
          floor.
        </div>
        <DeleteCurveRow name={name} usedBy={usedBy} writes={writes} onDone={onDone} onError={setError} />
        <ErrorLine error={error} />
      </div>
    </Dialog>
  );
}

// -------------------------------------------------------------- trigger

function TriggerEditor({
  name,
  info,
  temps,
  sensors,
  usedBy,
  writes,
  onDone,
  onClose,
}: SharedProps & {
  info: Extract<CurveInfo, { kind: "trigger" }>;
  temps: Record<string, number>;
  sensors: string[];
}) {
  const [sensor, setSensor] = useState(info.sensor);
  const [idleTemp, setIdleTemp] = useState(info.idle_temp);
  // Raw pwm state, percent display — see the FlatEditor note: opening and
  // applying untouched must round-trip the stored values exactly.
  const [idlePwm, setIdlePwm] = useState(info.idle_pwm);
  const [loadTemp, setLoadTemp] = useState(info.load_temp);
  const [loadPwm, setLoadPwm] = useState(info.load_pwm);
  const [response, setResponse] = useState(info.response_seconds);
  const [error, setError] = useState<string | null>(null);
  const live = temps[sensor];
  const fromDuty = (duty: number) => Math.round((duty / 100) * 255);

  const apply = () => {
    void writes
      .applyTriggerCurve(name, sensor, idleTemp, idlePwm, loadTemp, loadPwm, response)
      .then(({ error, warning }) => {
        if (error) setError(error);
        else onDone(warning ?? `Curve ${name} applied`);
      });
  };

  return (
    <Dialog width={468} label={`Edit trigger curve ${name}`} onClose={onClose}>
      <DialogHeader
        left={<Button variant="flat" onClick={onClose}>Cancel</Button>}
        title={name}
        subtitle="trigger curve"
        mono
        right={<Button variant="suggested" onClick={apply}>Apply</Button>}
      />
      <div className="flex flex-col gap-[18px] overflow-auto p-4">
        <div className="flex flex-col gap-2">
          <div className={groupTitle}>Source</div>
          <BoxedList>
            <ComboRow title="Temperature source" value={sensor} options={sensors} onChange={setSensor} />
            <ActionRow
              title="Live reading"
              trailing={
                <span className="numeric">{live !== undefined ? live.toFixed(1) : "—"} °C</span>
              }
            />
          </BoxedList>
        </div>
        <div className="flex flex-col gap-2">
          <div className={groupTitle}>Thresholds</div>
          <BoxedList>
            <SpinRow title="Idle below" value={idleTemp} min={0} max={114} unit="°C" onChange={setIdleTemp} />
            <SpinRow
              title="Idle duty"
              value={dutyPercent(idlePwm)}
              min={0}
              max={100}
              unit="%"
              onChange={(v) => setIdlePwm(fromDuty(v))}
            />
            <SpinRow title="Load above" value={loadTemp} min={0} max={114} unit="°C" onChange={setLoadTemp} />
            <SpinRow
              title="Load duty"
              value={dutyPercent(loadPwm)}
              min={0}
              max={100}
              unit="%"
              onChange={(v) => setLoadPwm(fromDuty(v))}
            />
            <SpinRow
              title="Response"
              subtitle="dwell past a threshold · 0 = instant"
              value={response}
              min={0}
              max={600}
              unit="s"
              onChange={setResponse}
            />
          </BoxedList>
          <div className={hint}>
            Latches between the two duties — inside the deadband it holds its last state, so it
            never oscillates. Not allowed on pwm1 (the pump header).
          </div>
        </div>
        <DeleteCurveRow name={name} usedBy={usedBy} writes={writes} onDone={onDone} onError={setError} />
        <ErrorLine error={error} />
      </div>
    </Dialog>
  );
}
