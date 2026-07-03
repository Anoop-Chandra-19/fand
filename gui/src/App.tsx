import { ChannelCard } from "./ChannelCard";
import { TempChart } from "./TempChart";
import { useDaemonStatus } from "./useDaemonStatus";

// Friendly names for the channels on this machine; unknown channels fall
// back to their raw pwmN name.
const CHANNEL_LABELS: Record<string, string> = {
  pwm1: "CPU radiator · AIO pump",
  pwm2: "case fans",
};

function App() {
  const { connected, latest, history } = useDaemonStatus();

  return (
    <main className="mx-auto flex max-w-[1080px] flex-col gap-4 px-6 pt-4 pb-8">
      {connected === false && (
        <div
          className="rounded-lg bg-critical px-4 py-2.5 font-semibold text-white"
          role="alert"
        >
          ⚠ fand daemon unreachable — fans are under firmware auto control
        </div>
      )}
      <header className="flex items-baseline gap-2.5">
        <h1 className="text-xl font-bold tracking-wide">fand</h1>
        <span
          className={`size-[9px] self-center rounded-full ${
            connected ? "bg-good" : connected === false ? "bg-critical" : "bg-muted"
          }`}
        />
        <span className="text-sm text-muted">
          {connected === null ? "connecting…" : connected ? "live" : "disconnected"}
        </span>
      </header>

      {latest ? (
        <>
          <section className="rounded-[10px] border border-white/10 bg-surface px-4 pt-3.5 pb-1.5">
            <h2 className="mb-2 text-xs font-semibold tracking-[0.08em] text-muted uppercase">
              temperatures
            </h2>
            <TempChart history={history} />
          </section>
          <section className="grid grid-cols-[repeat(auto-fit,minmax(300px,1fr))] gap-4">
            {Object.entries(latest.channels).map(([name, channel]) => (
              <ChannelCard
                key={name}
                name={name}
                label={CHANNEL_LABELS[name]}
                channel={channel}
                history={history}
              />
            ))}
          </section>
        </>
      ) : (
        <p className="py-12 text-center text-muted">
          waiting for the first status frame…
        </p>
      )}
    </main>
  );
}

export default App;
