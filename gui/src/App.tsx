import { useDaemonStatus } from "./daemon/useDaemonStatus";
import { ChannelCard } from "./dashboard/ChannelCard";
import { TempChart } from "./dashboard/TempChart";

// Friendly names for the channels on this machine; unknown channels fall
// back to their raw pwmN name.
const CHANNEL_LABELS: Record<string, string> = {
  pwm1: "CPU radiator · AIO pump",
  pwm2: "Case fans",
};

function App() {
  const { connected, latest, history } = useDaemonStatus();

  return (
    <div className="flex min-h-screen flex-col">
      {connected === false && (
        <div
          className="bg-error-bg px-4 py-2 text-center text-sm font-bold"
          role="alert"
        >
          fand daemon unreachable — fans are under firmware auto control
        </div>
      )}

      <main className="mx-auto flex w-full max-w-[1080px] flex-col gap-6 px-5 py-5">
        {latest ? (
          <>
            <section>
              <h2 className="mb-2.5 font-bold">Temperatures</h2>
              <div className="rounded-xl bg-card px-4 pt-4 pb-2">
                <TempChart history={history} />
              </div>
            </section>
            <section>
              <h2 className="mb-2.5 font-bold">Fans</h2>
              <div className="grid grid-cols-[repeat(auto-fit,minmax(300px,1fr))] gap-4">
                {Object.entries(latest.channels).map(([name, channel]) => (
                  <ChannelCard
                    key={name}
                    name={name}
                    label={CHANNEL_LABELS[name]}
                    channel={channel}
                    history={history}
                  />
                ))}
              </div>
            </section>
          </>
        ) : (
          <p className="py-12 text-center text-dim">
            Waiting for the first status frame…
          </p>
        )}
      </main>
    </div>
  );
}

export default App;
