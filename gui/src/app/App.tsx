import { useEffect, useRef, useState } from "react";

import { listen } from "@tauri-apps/api/event";
import { Banner } from "../adw/Banner";
import { ToastOverlay } from "../adw/Toast";
import { getDaemonSocket, reloadDaemonConfig } from "../api/daemonCommands";
import { useDaemonStatus } from "../api/useDaemonStatus";
import { CurveEditorDialog } from "../features/curves/CurveEditorDialog";
import { NewCurveDialog } from "../features/curves/NewCurveDialog";
import { curveCommands } from "../features/curves/commands";
import { usedByOf } from "../features/curves/model";
import { ChannelPropsDialog } from "../features/fans/ChannelPropsDialog";
import {
  clearOverride,
  setChannelCurve,
  setMinPwm,
  setOffsetPwm,
  setSmoothingSeconds,
} from "../features/fans/commands";
import { AboutDialog } from "./AboutDialog";
import { Dashboard } from "./Dashboard";
import { HeaderBar } from "./HeaderBar";
import { CHANNEL_LABELS } from "./hardwareLabels";
import { PreferencesDialog } from "./preferences/PreferencesDialog";
import { applyAccent, loadAccent, type Accent } from "./preferences/accent";
import { chartWindowMs, loadChartMinutes, saveChartMinutes } from "./preferences/storage";



function App() {
  const [chartMinutes, setChartMinutes] = useState(loadChartMinutes);
  // The backend pushes status AND config; this component never fetches,
  // caches or reconciles daemon state — it renders the last event.
  const { connected, latest, config, history } = useDaemonStatus(chartWindowMs(chartMinutes));

  const [socketPath, setSocketPath] = useState<string | null>(null);
  useEffect(() => {
    void getDaemonSocket().then(setSocketPath, () => setSocketPath(null));
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

  const handleSetChannelCurve = (channel: string, curve: string) => {
    void setChannelCurve(channel, curve).then(({ error, warning }) => {
      flash(error ?? warning ?? `${channel} now follows ${curve}`);
    });
  };

  const openChannelProperties = (name: string) => {
    // A missing config would make the dialog silently not open; say so instead.
    if (config?.channel_settings[name]) setPropsFor(name);
    else flash("Channel settings not loaded yet — retrying in the background");
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
      <div className="min-h-0 flex-1 overflow-auto">
        <Dashboard
          connected={connected}
          latest={latest}
          config={config}
          history={history}
          onSetChannelCurve={handleSetChannelCurve}
          onOpenChannelProperties={openChannelProperties}
          onEditCurve={setEditing}
          onCreateCurve={() => setCreating(true)}
        />
      </div>

      <ToastOverlay toast={toast} />

      {editing && curves[editing] && (
        <CurveEditorDialog
          name={editing}
          info={curves[editing]}
          temps={temps}
          sensors={config?.sensors ?? sensors}
          curveNames={curveNames}
          usedBy={usedByOf(curves, channelCurves, editing)}
          writes={curveCommands}
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
          setChannelCurve={setChannelCurve}
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
          writes={curveCommands}
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
              await reloadDaemonConfig();
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
