import "./App.css";
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
    <main className="app">
      {connected === false && (
        <div className="banner" role="alert">
          ⚠ fand daemon unreachable — fans are under firmware auto control
        </div>
      )}
      <header className="header">
        <h1>fand</h1>
        <span
          className={`dot ${connected ? "up" : connected === false ? "down" : "idle"}`}
        />
        <span className="conn-label">
          {connected === null ? "connecting…" : connected ? "live" : "disconnected"}
        </span>
      </header>

      {latest ? (
        <>
          <section className="panel">
            <h2>temperatures</h2>
            <TempChart history={history} />
          </section>
          <section className="cards">
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
        <p className="empty">waiting for the first status frame…</p>
      )}
    </main>
  );
}

export default App;
