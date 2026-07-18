import { useEffect, useRef, useState } from "react";
import { Banner } from "./adw/Banner";
import { StatusPage } from "./adw/StatusPage";
import { ToastOverlay } from "./adw/Toast";
import { WarnIcon } from "./adw/icons";
import { useDaemonStatus } from "./daemon/useDaemonStatus";
import type { CurveInfo } from "./daemon/types";
import { useCurveEditor } from "./curves/useCurveEditor";
import { clearOverride, useChannelSettings } from "./settings/useChannelSettings";
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
  const { connected, latest, history } = useDaemonStatus(chartWindowMs(chartMinutes));
  const curveEditor = useCurveEditor();
  const settings = useChannelSettings();
  const { data: curveData, refresh: refreshCurves } = curveEditor;
  const { data: settingsData, refresh: refreshSettings } = settings;

  // Config self-heal. The daemon restates its config generation in every
  // status frame (level-triggered, so a missed change can't strand us);
  // refetch when our copies came from a different generation, when an
  // earlier fetch failed, or after a reconnect — the counter restarts
  // with the daemon, so a matching number across a reconnect proves
  // nothing.
  const syncing = useRef(false);
  const forceSync = useRef(false);
  useEffect(() => {
    if (connected === false) forceSync.current = true;
  }, [connected]);
  useEffect(() => {
    if (!latest || syncing.current) return;
    // Strictly "frame is ahead of our copy": right after one of our own
    // writes the copy is ahead of the last frame until the next tick, and
    // a plain != would refetch in a loop until it arrives. The daemon
    // restart case (counter reset to a lower value) is covered by
    // forceSync — the socket drop always emits daemon-down first.
    // Each payload carries its own generation: if one refetch fails while
    // the other succeeds, the failed one alone stays flagged stale.
    const stale =
      forceSync.current ||
      curveData === null ||
      settingsData === null ||
      curveData.config_generation < latest.config_generation ||
      settingsData.config_generation < latest.config_generation;
    if (!stale) return;
    syncing.current = true;
    void Promise.allSettled([refreshCurves(), refreshSettings()]).then((results) => {
      syncing.current = false;
      if (results.every((r) => r.status === "fulfilled")) forceSync.current = false;
    });
  }, [latest, curveData, settingsData, refreshCurves, refreshSettings]);

  const [socketPath, setSocketPath] = useState<string | null>(null);
  useEffect(() => {
    void invoke<string>("daemon_socket").then(setSocketPath, () => setSocketPath(null));
  }, []);

  const [editing, setEditing] = useState<string | null>(null);
  const [propsFor, setPropsFor] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [prefs, setPrefs] = useState(false);
  const [about, setAbout] = useState(false);
  const [accent, setAccent] = useState<Accent>(loadAccent);
  useEffect(() => applyAccent(accent), [accent]);

  const [toast, setToast] = useState<string | null>(null);
  const toastTimer = useRef<number | undefined>(undefined);
  const flash = (msg: string) => {
    setToast(msg);
    window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => setToast(null), 2600);
  };

  const curves = curveData?.curves ?? {};
  const curveNames = Object.keys(curves);
  const channelCurves = curveData?.channels ?? {};
  const sensors = latest ? Object.keys(latest.temps) : [];
  const temps = latest?.temps ?? {};

  const overriding = latest
    ? Object.entries(latest.channels).filter(([, c]) => c.mode === "override")
    : [];

  const setChannelCurve = (channel: string, curve: string) => {
    void curveEditor.setChannelCurve(channel, curve).then((err) => {
      flash(err ?? `${channel} now follows ${curve}`);
    });
  };

  const cancelOverrides = () => {
    for (const [name] of overriding) {
      void clearOverride(name).then((err) => flash(err ?? "Override cleared"));
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
                  // A missing settings payload would make the dialog
                  // silently not open; say so instead.
                  if (settingsData?.channels[name]) setPropsFor(name);
                  else flash("Channel settings not loaded yet — retrying in the background");
                }}
              />
            ))}
          </div>
        </section>

        <section>
          <SectionHeader
            trailing={
              curveData
                ? curveNames.length
                  ? "reusable behaviors"
                  : "none configured"
                : "loading…"
            }
          >
            Curves
          </SectionHeader>
          {/* An unloaded config is not an empty one: never show "no
              curves" (or offer edits) while the fetch hasn't succeeded. */}
          {!curveData && (
            <div className="rounded-card bg-card px-5 py-[18px] shadow-card">
              <span className="text-[0.82rem] leading-[1.4] text-dim">
                Waiting for the curve configuration — retrying automatically…
              </span>
            </div>
          )}
          {curveData && curveNames.length === 0 && (
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
          {curveData && (
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
          sensors={curveData?.sensors ?? sensors}
          curveNames={curveNames}
          usedBy={usedByOf(curves, channelCurves, editing)}
          writes={curveEditor}
          onDone={(msg) => {
            flash(msg);
            setEditing(null);
          }}
          onClose={() => setEditing(null)}
        />
      )}

      {propsFor && settings.data?.channels[propsFor] && (
        <ChannelPropsDialog
          name={propsFor}
          label={CHANNEL_LABELS[propsFor]}
          settings={settings.data.channels[propsFor]}
          boundCurve={channelCurves[propsFor]}
          curveNames={curveNames}
          setChannelCurve={curveEditor.setChannelCurve}
          setMinPwm={settings.setMinPwm}
          setSmoothingSeconds={settings.setSmoothingSeconds}
          setOffsetPwm={settings.setOffsetPwm}
          onClose={() => setPropsFor(null)}
        />
      )}

      {creating && (
        <NewCurveDialog
          curves={curves}
          sensors={curveData?.sensors ?? sensors}
          writes={curveEditor}
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
              // Refetch right away for snappiness; the generation check
              // against the status stream is the backstop either way.
              void Promise.allSettled([refreshCurves(), refreshSettings()]);
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
