import { useState } from "react";
import { Button } from "../adw/Button";
import { Dialog, DialogHeader } from "../adw/Dialog";
import { ActionRow, BoxedList, ComboRow, SpinRow, Switch } from "../adw/rows";
import type { CurveInfo, WriteResult } from "../daemon/types";

const KINDS = [
  { value: "graph", label: "graph — sensor → duty" },
  { value: "mix", label: "mix — combine curves" },
  { value: "flat", label: "flat — constant duty" },
  { value: "trigger", label: "trigger — two-state latch" },
];

/** A fresh graph starts from this ramp; the editor opens next to shape it. */
const DEFAULT_RAMP: [number, number][] = [
  [40, 80],
  [60, 140],
  [80, 220],
  [90, 255],
];

const groupTitle = "px-1 text-[0.82rem] font-bold tracking-[0.02em] text-dim";
const hint = "px-1 text-[0.82rem] leading-[1.4] text-dim";

export function NewCurveDialog({
  curves,
  sensors,
  writes,
  onDone,
  onClose,
}: {
  curves: Record<string, CurveInfo>;
  sensors: string[];
  writes: {
    createGraphCurve: (
      name: string,
      sensor: string,
      points: [number, number][],
    ) => Promise<WriteResult>;
    createFlatCurve: (name: string, pwm: number) => Promise<WriteResult>;
    createMixCurve: (name: string, fn: string, members: string[]) => Promise<WriteResult>;
    createTriggerCurve: (
      name: string,
      sensor: string,
      idleTemp: number,
      idlePwm: number,
      loadTemp: number,
      loadPwm: number,
      responseSeconds: number,
    ) => Promise<WriteResult>;
  };
  /** message + the created curve's name (to open the editor for graphs). */
  onDone: (message: string, name: string, openEditor: boolean) => void;
  onClose: () => void;
}) {
  const [name, setName] = useState("");
  const [kind, setKind] = useState("graph");
  const [sensor, setSensor] = useState(sensors[0] ?? "");
  const [fn, setFn] = useState("max");
  const [members, setMembers] = useState<string[]>([]);
  const [flatDuty, setFlatDuty] = useState(30);
  const [idleTemp, setIdleTemp] = useState(45);
  const [idleDuty, setIdleDuty] = useState(35);
  const [loadTemp, setLoadTemp] = useState(65);
  const [loadDuty, setLoadDuty] = useState(78);
  const [response, setResponse] = useState(0);
  const [error, setError] = useState<string | null>(null);

  const existing = Object.keys(curves);
  const clean = name.trim();
  const taken = existing.includes(clean);
  const valid = clean !== "" && !taken && (kind !== "mix" || members.length >= 2);
  const toggleMember = (m: string) =>
    setMembers((ms) => (ms.includes(m) ? ms.filter((x) => x !== m) : [...ms, m]));

  const create = () => {
    if (!valid) return;
    const duty = (d: number) => Math.round((d / 100) * 255);
    const run =
      kind === "graph"
        ? writes.createGraphCurve(clean, sensor, DEFAULT_RAMP)
        : kind === "flat"
          ? writes.createFlatCurve(clean, duty(flatDuty))
          : kind === "mix"
            ? writes.createMixCurve(clean, fn, members)
            : writes.createTriggerCurve(
                clean,
                sensor,
                idleTemp,
                duty(idleDuty),
                loadTemp,
                duty(loadDuty),
                response,
              );
    void run.then(({ error, warning }) => {
      if (error) setError(error);
      else onDone(warning ?? `Curve ${clean} created`, clean, kind === "graph");
    });
  };

  return (
    <Dialog width={468} label="New curve" onClose={onClose}>
      <DialogHeader
        left={<Button variant="flat" onClick={onClose}>Cancel</Button>}
        title="New curve"
        right={
          <Button variant="suggested" disabled={!valid} onClick={create}>
            Create
          </Button>
        }
      />
      <div className="flex flex-col gap-4.5 overflow-auto p-4">
        <div className="flex flex-col gap-1.5">
          <BoxedList>
            <ActionRow
              title="Name"
              subtitle={taken ? undefined : "lowercase · letters, numbers, _"}
              trailing={
                <input
                  aria-label="Curve name"
                  value={name}
                  onChange={(e) =>
                    setName(e.target.value.replace(/[^a-z0-9_]/gi, "").toLowerCase())
                  }
                  placeholder="e.g. cpu_case"
                  spellCheck={false}
                  className="w-37.5 border-none bg-transparent text-right font-mono text-ink outline-none"
                  style={{ caretColor: "var(--color-accent)" }}
                />
              }
            />
            <ComboRow title="Kind" value={kind} options={KINDS} onChange={setKind} />
          </BoxedList>
          {taken && (
            <span className="px-1 text-[0.82rem] text-error">
              a curve named "{clean}" already exists
            </span>
          )}
        </div>

        {kind === "graph" && (
          <div className="flex flex-col gap-1.5">
            <div className={groupTitle}>Graph</div>
            <BoxedList>
              <ComboRow
                title="Temperature source"
                value={sensor}
                options={sensors}
                onChange={setSensor}
              />
            </BoxedList>
            <span className={hint}>starts from a default ramp — shape it in the editor next.</span>
          </div>
        )}

        {kind === "flat" && (
          <div className="flex flex-col gap-1.5">
            <div className={groupTitle}>Flat</div>
            <BoxedList>
              <SpinRow
                title="Constant duty"
                value={flatDuty}
                min={0}
                max={100}
                unit="%"
                onChange={setFlatDuty}
              />
            </BoxedList>
          </div>
        )}

        {kind === "mix" && (
          <div className="flex flex-col gap-1.5">
            <div className={groupTitle}>Mix</div>
            <BoxedList>
              <ComboRow
                title="Function"
                value={fn}
                options={["max", "min", "average"]}
                onChange={setFn}
              />
              {existing.length === 0 ? (
                <ActionRow
                  title="No curves to combine"
                  subtitle="create a graph or flat curve first"
                />
              ) : (
                existing.map((m) => (
                  <ActionRow
                    key={m}
                    title={<span className="font-mono">{m}</span>}
                    subtitle={curves[m].kind}
                    trailing={
                      <Switch
                        checked={members.includes(m)}
                        ariaLabel={`Include ${m} in this mix`}
                        onChange={() => toggleMember(m)}
                      />
                    }
                  />
                ))
              )}
            </BoxedList>
            <span className={hint}>
              {members.length < 2
                ? "pick at least two curves to mix"
                : `${fn} of ${members.length} curves`}
            </span>
          </div>
        )}

        {kind === "trigger" && (
          <div className="flex flex-col gap-1.5">
            <div className={groupTitle}>Trigger</div>
            <BoxedList>
              <ComboRow
                title="Temperature source"
                value={sensor}
                options={sensors}
                onChange={setSensor}
              />
              <SpinRow title="Idle below" value={idleTemp} min={0} max={114} unit="°C" onChange={setIdleTemp} />
              <SpinRow title="Idle duty" value={idleDuty} min={0} max={100} unit="%" onChange={setIdleDuty} />
              <SpinRow title="Load above" value={loadTemp} min={0} max={114} unit="°C" onChange={setLoadTemp} />
              <SpinRow title="Load duty" value={loadDuty} min={0} max={100} unit="%" onChange={setLoadDuty} />
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
            <span className={hint}>not allowed on pwm1 (the pump header).</span>
          </div>
        )}

        {error && <div className="px-1 text-[0.82rem] leading-[1.4] text-error">{error}</div>}
      </div>
    </Dialog>
  );
}
