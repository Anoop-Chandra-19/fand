import { StatusPage } from "../adw/StatusPage";
import { WarnIcon } from "../adw/icons";
import type { ConfigPayload, Sample, Status } from "../api/daemonTypes";
import { AddCurveCard, CurveCard } from "../features/curves/CurveCard";
import { usedByOf } from "../features/curves/model";
import { ChannelCard } from "../features/fans/ChannelCard";
import { TempChartCard } from "../features/monitoring/TempChart";
import { CHANNEL_LABELS, SENSOR_LABELS } from "./hardwareLabels";

function SectionHeader({ trailing, children }: { trailing?: string; children: string }) {
  return (
    <div className="mb-2.5 flex items-baseline justify-between px-0.5">
      <h2 className="m-0 text-[1rem] font-bold tracking-[0.01em]">{children}</h2>
      {trailing && <span className="text-[0.82rem] text-dim">{trailing}</span>}
    </div>
  );
}

const grid = (min: number) => ({
  display: "grid",
  gridTemplateColumns: `repeat(auto-fit, minmax(${min}px, 1fr))`,
  gap: 14,
});

export function Dashboard({
  connected,
  latest,
  config,
  history,
  onSetChannelCurve,
  onOpenChannelProperties,
  onEditCurve,
  onCreateCurve,
}: {
  connected: boolean | null;
  latest: Status | null;
  config: ConfigPayload | null;
  history: Sample[];
  onSetChannelCurve: (channel: string, curve: string) => void;
  onOpenChannelProperties: (channel: string) => void;
  onEditCurve: (name: string) => void;
  onCreateCurve: () => void;
}) {
  if (connected === false) {
    return (
      <div className="flex h-full items-center justify-center p-6">
        <StatusPage
          icon={<WarnIcon size={56} />}
          title="Lost the connection to fand"
          description="This window can't reach the daemon socket, so the fans' state is unknown from here. If fand stopped, the motherboard firmware automatically took back fan control; if it's still running, it keeps following your curves without this window."
        >
          <span className="text-[0.82rem] text-dim">retrying every 2 s…</span>
        </StatusPage>
      </div>
    );
  }

  if (!latest) {
    return (
      <div className="flex h-full items-center justify-center p-6">
        <StatusPage title="Waiting for the first status frame…" />
      </div>
    );
  }

  const curves = config?.curves ?? {};
  const curveNames = Object.keys(curves);
  const channelCurves = config?.channels ?? {};
  const sensors = Object.keys(latest.temps);
  const temps = latest.temps;

  return (
    <main className="mx-auto flex w-full max-w-270 flex-col gap-5.5 px-6 pb-7 pt-5">
      <section>
        <SectionHeader trailing="live">Temperatures</SectionHeader>
        <TempChartCard history={history} sensors={sensors} labels={SENSOR_LABELS} temps={temps} />
      </section>

      <section>
        <SectionHeader trailing="controllable pwm headers">Fans</SectionHeader>
        <div style={grid(320)}>
          {Object.entries(latest.channels).map(([name, channel]) => (
            <ChannelCard
              key={name}
              name={name}
              label={CHANNEL_LABELS[name]}
              channel={channel}
              boundCurve={channelCurves[name]}
              curves={curves}
              temps={temps}
              curveNames={curveNames}
              pwmHistory={history
                .map((sample) => sample.status.channels[name]?.current_pwm)
                .filter((pwm): pwm is number => pwm !== undefined)}
              onSetCurve={onSetChannelCurve}
              onProps={() => onOpenChannelProperties(name)}
            />
          ))}
        </div>
      </section>

      <section>
        <SectionHeader
          trailing={
            config
              ? curveNames.length
                ? "reusable behaviors"
                : "none configured"
              : "loading…"
          }
        >
          Curves
        </SectionHeader>
        {/* An unloaded config is not an empty one: never show "no
            curves" (or offer edits) while no config has arrived. */}
        {!config && (
          <div className="rounded-card bg-card px-5 py-4.5 shadow-card">
            <span className="text-[0.82rem] leading-[1.4] text-dim">
              Waiting for the curve configuration — retrying automatically…
            </span>
          </div>
        )}
        {config && curveNames.length === 0 && (
          <div className="mb-3 flex flex-wrap items-center gap-3.5 rounded-card bg-card px-5 py-4.5 shadow-card">
            <div className="flex min-w-55 flex-1 flex-col gap-0.5">
              <span className="font-bold">No fan curves yet</span>
              <span className="text-[0.82rem] leading-[1.4] text-dim">
                fand won't take control of a header until a sensor is mapped to a fan duty. Add a
                curve, then assign it to a channel.
              </span>
            </div>
          </div>
        )}
        {config && (
          <div style={grid(300)}>
            {Object.entries(curves).map(([name, info]) => (
              <CurveCard
                key={name}
                name={name}
                info={info}
                curves={curves}
                temps={temps}
                usedBy={usedByOf(curves, channelCurves, name)}
                onEdit={() => onEditCurve(name)}
              />
            ))}
            <AddCurveCard onClick={onCreateCurve} />
          </div>
        )}
      </section>
    </main>
  );
}
