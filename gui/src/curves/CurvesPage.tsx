import { useState } from "react";
import type { CurveEditorPayload, CurvePoint } from "../daemon/types";
import { CurveEditor } from "./CurveEditor";

interface Props {
  data: CurveEditorPayload | null;
  temps: Record<string, number>;
  setCurvePoints: (name: string, points: CurvePoint[]) => Promise<string | null>;
  deleteCurve: (name: string) => Promise<string | null>;
}

interface Usage {
  channel: string;
  sensor: string;
}

const STARTER_POINTS: CurvePoint[] = [
  [40, 80],
  [80, 255],
];

function usedBy(curveName: string, channels: CurveEditorPayload["channels"]): Usage[] {
  const usages: Usage[] = [];
  for (const [channel, { refs }] of Object.entries(channels)) {
    for (const ref of refs) {
      if (ref.curve === curveName) usages.push({ channel, sensor: ref.sensor });
    }
  }
  return usages;
}

export function CurvesPage({ data, temps, setCurvePoints, deleteCurve }: Props) {
  const [creating, setCreating] = useState(false);
  const [newName, setNewName] = useState("");
  const [createError, setCreateError] = useState<string | null>(null);
  const [deleteErrors, setDeleteErrors] = useState<Record<string, string>>({});

  if (!data) {
    return <p className="py-12 text-center text-dim">Loading curves…</p>;
  }

  const startCreate = async () => {
    const name = newName.trim();
    if (!name) return;
    if (data.curves[name]) {
      setCreateError(`a curve named "${name}" already exists`);
      return;
    }
    const err = await setCurvePoints(name, STARTER_POINTS);
    if (err) {
      setCreateError(err);
    } else {
      setCreating(false);
      setNewName("");
      setCreateError(null);
    }
  };

  const handleDelete = async (name: string) => {
    const err = await deleteCurve(name);
    setDeleteErrors((prev) => ({ ...prev, [name]: err ?? "" }));
  };

  return (
    <section className="grid grid-cols-[repeat(auto-fit,minmax(340px,1fr))] gap-5">
      {Object.entries(data.curves).map(([name, points]) => {
        const usages = usedBy(name, data.channels);
        const liveTemp = usages.length > 0 ? temps[usages[0].sensor] : undefined;
        const inUse = usages.length > 0;
        return (
          <article key={name} className="flex flex-col gap-3 rounded-xl bg-card px-5 py-4">
            <header className="flex items-start justify-between gap-2">
              <div className="flex items-center gap-1.5">
                <h3 className="font-bold">{name}</h3>
                <button
                  type="button"
                  disabled={inUse}
                  title={inUse ? "reassign the channels using this curve first" : "delete curve"}
                  onClick={() => void handleDelete(name)}
                  className="text-dim hover:text-error disabled:cursor-not-allowed disabled:opacity-30 disabled:hover:text-dim"
                >
                  ×
                </button>
              </div>
              <div className="flex flex-wrap justify-end gap-1">
                {usages.length > 0 ? (
                  usages.map((u) => (
                    <span
                      key={`${u.channel}-${u.sensor}`}
                      className="rounded-full bg-white/10 px-2.5 py-0.5 text-xs whitespace-nowrap text-dim"
                    >
                      {u.channel} · {u.sensor}
                    </span>
                  ))
                ) : (
                  <span className="rounded-full bg-white/10 px-2.5 py-0.5 text-xs whitespace-nowrap text-dim">
                    unused
                  </span>
                )}
              </div>
            </header>
            <CurveEditor
              points={points}
              liveTemp={liveTemp}
              onCommit={(next) => setCurvePoints(name, next)}
            />
            {deleteErrors[name] && <p className="text-xs text-error">{deleteErrors[name]}</p>}
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
                if (e.key === "Escape") {
                  setCreating(false);
                  setNewName("");
                  setCreateError(null);
                }
              }}
              placeholder="curve name"
              className="rounded-md bg-white/10 px-2.5 py-1.5 text-sm outline-none focus:ring-1 focus:ring-accent"
            />
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
                onClick={() => {
                  setCreating(false);
                  setNewName("");
                  setCreateError(null);
                }}
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
