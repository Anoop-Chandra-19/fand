import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Banner } from "./adw/Banner";
import { StatusPage } from "./adw/StatusPage";
import { ToastOverlay } from "./adw/Toast";
import { WarnIcon } from "./adw/icons";
import { useDaemonStatus } from "./daemon/useDaemonStatus";
import type { CurveInfo } from "./daemon/types";
import { curveWrites } from "./curves/writes";
import { clearOverride, setMinPwm, setOffsetPwm, setSmoothingSeconds } from "./settings/writes";
import { ChannelCard } from "./dashboard/ChannelCard";
import { AddCurveCard, CurveCard } from "./dashboard/CurveCard";
import { TempChartCard } from "./dashboard/TempChart";
import { invoke } from "@tauri-apps/api/core";
import { HeaderBar } from "./shell/HeaderBar";
import { applyAccent, loadAccent, type Accent } from "./shell/accent";
import { chartWindowMs, loadChartMinutes, saveChartMinutes } from "./shell/prefs";
import { CurveEditorDialog } from "./dialogs/CurveEditorDialog";
import { ChannelPropsDialog } from "./dialogs/ChannelPropsDialog";
import { NewCurveDialog } from "./dialogs/NewCurveDialog";
import { PreferencesDialog } from "./dialogs/PreferencesDialog";
import { AboutDialog } from "./dialogs/AboutDialog";

// Friendly names for this machine's hardware; unknown names fall back to
// the raw identifier — which always stays visible next to the label.
const CHANNEL_LABELS: Record<string, string> = {
  pwm1: "CPU radiator · AIO pump",
  pwm2: "Case fans",
};
const SENSOR_LABELS: Record<string, string> = {
  cpu: "CPU · Core (Tctl) — Ryzen 7 7800X3D",
  gpu: "GPU — NVIDIA GeForce RTX 4090",
};

function usedByOf(
  curves: Record<string, CurveInfo>,
  channels: Record<string, string>,
  name: string,
): string[] {
  const used: string[] = [];
  for (const [ch, curve] of Object.entries(channels)) if (curve === name) used.push(ch);
  for (const [cn, c] of Object.entries(curves))
    if (c.kind === "mix" && c.members.includes(name)) used.push(cn);
  return used;
}

function SectionHeader({ trailing, children }: { trailing?: string; children: string }) {
  return (
    <div className="mb-[10px] flex items-baseline justify-between px-[2px]">
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

function App() {
  const [chartMinutes, setChartMinutes] = useState(loadChartMinutes);
  // The backend pushes status AND config; this component never fetches,
  // caches or reconciles daemon state — it renders the last event.
  const { connected, latest, config, history } = useDaemonStatus(chartWindowMs(chartMinutes));

  const [socketPath, setSocketPath] = useState<string | null>(null);
  useEffect(() => {
    void invoke<string>("daemon_socket").then(setSocketPath, () => setSocketPath(null));
  }, []);

  const [editing, setEditing] = useState<string | null>(null);
  const [propsFor, setPropsFor] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [prefs, setPrefs] = useState(false);
  const [about, setAbout] = useState(false);
  // A disconnect closes the editing dialogs explicitly: their drafts were
  // against a daemon that is gone, and the config it restarts with may be
  // different. Preferences/About are connection-independent and stay.
  useEffect(() => {
    if (connected === false) {
      setEditing(null);
      setPropsFor(null);
      setCreating(false);
    }
  }, [connected]);
  // Same rule for the fast-restart path: when the backend detects a
  // restart mid-frame it reconnects immediately, so `connected` never
  // goes false — this event is the only signal the drafts are stale.
  useEffect(() => {
    const unlisten = listen("daemon-restarted", () => {
      setEditing(null);
      setPropsFor(null);
      setCreating(false);
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, []);
  const [accent, setAccent] = useState<Accent>(loadAccent);
  useEffect(() => applyAccent(accent), [accent]);

  const [toast, setToast] = useState<string | null>(null);
  const toastTimer = useRef<number | undefined>(undefined);
  // Long messages (write warnings, error text) get proportionally more
  // reading time than short confirmations.
  const flash = (msg: string) => {
    setToast(msg);
    window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => setToast(null), msg.length > 80 ? 8000 : 2600);
  };

  const curves = config?.curves ?? {};
  const curveNames = Object.keys(curves);
  const channelCurves = config?.channels ?? {};
  const sensors = latest ? Object.keys(latest.temps) : [];
  const temps = latest?.temps ?? {};

  const overriding = latest
    ? Object.entries(latest.channels).filter(([, c]) => c.mode === "override")
    : [];

  const setChannelCurve = (channel: string, curve: string) => {
    void curveWrites.setChannelCurve(channel, curve).then(({ error, warning }) => {
      flash(error ?? warning ?? `${channel} now follows ${curve}`);
    });
  };

  const cancelOverrides = () => {
    for (const [name] of overriding) {
      void clearOverride(name).then(({ error, warning }) =>
        flash(error ?? warning ?? "Override cleared"),
      );
    }
  };

  const subtitle =
    connected === false
      ? "disconnected"
      : latest
        ? `daemon connected · ${Object.keys(latest.channels).length} headers`
        : "connecting…";

  const banner =
    connected === false ? (
      <Banner tone="error">
        fand daemon unreachable — fan state unknown from here; if the daemon stopped, firmware
        auto has the fans
      </Banner>
    ) : overriding.length > 0 ? (
      <Banner tone="warning" action="Cancel" onAction={cancelOverrides}>
        Manual override active on {overriding.map(([n]) => n).join(", ")} — curve control paused
        {overriding[0][1].override_remaining_s !== undefined
          ? ` · ${overriding[0][1].override_remaining_s} s left`
          : ""}
      </Banner>
    ) : null;

  let body;
  if (connected === false) {
    body = (
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
  } else if (!latest) {
    body = (
      <div className="flex h-full items-center justify-center p-6">
        <StatusPage title="Waiting for the first status frame…" />
      </div>
    );
  } else {
    body = (
      <main className="mx-auto flex w-full max-w-[1080px] flex-col gap-[22px] px-6 pb-7 pt-5">
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
                  .map((s) => s.status.channels[name]?.current_pwm)
                  .filter((p): p is number => p !== undefined)}
                onSetCurve={setChannelCurve}
                onProps={() => {
                  // A missing config would make the dialog silently not
                  // open; say so instead.
                  if (config?.channel_settings[name]) setPropsFor(name);
                  else flash("Channel settings not loaded yet — retrying in the background");
                }}
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
            <div className="rounded-card bg-card px-5 py-[18px] shadow-card">
              <span className="text-[0.82rem] leading-[1.4] text-dim">
                Waiting for the curve configuration — retrying automatically…
              </span>
            </div>
          )}
          {config && curveNames.length === 0 && (
            <div className="mb-3 flex flex-wrap items-center gap-[14px] rounded-card bg-card px-5 py-[18px] shadow-card">
              <div className="flex min-w-[220px] flex-1 flex-col gap-[2px]">
                <span className="font-bold">No fan curves yet</span>
                <span className="text-[0.82rem] leading-[1.4] text-dim">
                  fand won't take control of a header until a sensor is mapped to a fan duty. Add
                  a curve, then assign it to a channel.
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
                  onEdit={() => setEditing(name)}
                />
              ))}
              <AddCurveCard onClick={() => setCreating(true)} />
            </div>
          )}
        </section>
      </main>
    );
  }

  return (
    <div className="flex h-full flex-col overflow-hidden bg-window text-ink">
      <HeaderBar
        title="fand"
        subtitle={subtitle}
        menuItems={[
          { label: "New curve", onClick: () => setCreating(true) },
          { label: "Preferences", onClick: () => setPrefs(true) },
          { label: "About fand", onClick: () => setAbout(true) },
        ]}
      />
      {banner}
      <div className="min-h-0 flex-1 overflow-auto">{body}</div>

      <ToastOverlay toast={toast} />

      {editing && curves[editing] && (
        <CurveEditorDialog
          name={editing}
          info={curves[editing]}
          temps={temps}
          sensors={config?.sensors ?? sensors}
          curveNames={curveNames}
          usedBy={usedByOf(curves, channelCurves, editing)}
          writes={curveWrites}
          onDone={(msg) => {
            flash(msg);
            setEditing(null);
          }}
          onClose={() => setEditing(null)}
        />
      )}

      {propsFor && config?.channel_settings[propsFor] && (
        <ChannelPropsDialog
          name={propsFor}
          label={CHANNEL_LABELS[propsFor]}
          settings={config.channel_settings[propsFor]}
          boundCurve={channelCurves[propsFor]}
          curveNames={curveNames}
          setChannelCurve={curveWrites.setChannelCurve}
          setMinPwm={setMinPwm}
          setSmoothingSeconds={setSmoothingSeconds}
          setOffsetPwm={setOffsetPwm}
          onClose={() => setPropsFor(null)}
        />
      )}

      {creating && (
        <NewCurveDialog
          curves={curves}
          sensors={config?.sensors ?? sensors}
          writes={curveWrites}
          onDone={(msg, name, openEditor) => {
            flash(msg);
            setCreating(false);
            if (openEditor) setEditing(name);
          }}
          onClose={() => setCreating(false)}
        />
      )}

      {prefs && (
        <PreferencesDialog
          accent={accent}
          onAccent={setAccent}
          chartMinutes={chartMinutes}
          onChartMinutes={(m) => {
            setChartMinutes(m);
            saveChartMinutes(m);
          }}
          socketPath={socketPath}
          connected={connected}
          onReloadConfig={async () => {
            try {
              await invoke("reload_config");
              // The generation bump reaches the backend with the next
              // status frame, which carries the fresh config here.
              flash("Config reloaded from disk");
              return null;
            } catch (e) {
              return String(e);
            }
          }}
          onClose={() => setPrefs(false)}
        />
      )}

      {about && <AboutDialog connected={connected} onClose={() => setAbout(false)} />}
    </div>
  );
}

export default App;
