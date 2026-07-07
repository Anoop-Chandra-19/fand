import { useState } from "react";
import { useDaemonStatus } from "./daemon/useDaemonStatus";
import { useCurveEditor } from "./curves/useCurveEditor";
import { CurvesPage } from "./curves/CurvesPage";
import { useChannelSettings } from "./settings/useChannelSettings";
import { SettingsPage } from "./settings/SettingsPage";
import { ChannelCard } from "./dashboard/ChannelCard";
import { TempChart } from "./dashboard/TempChart";
import { Sidebar, type Page } from "./nav/Sidebar";

// Friendly names for the channels on this machine; unknown channels fall
// back to their raw pwmN name.
const CHANNEL_LABELS: Record<string, string> = {
  pwm1: "CPU radiator · AIO pump",
  pwm2: "Case fans",
};

function App() {
  const { connected, latest, history } = useDaemonStatus();
  const {
    data: curveData,
    setCurvePoints,
    createGraphCurve,
    setGraphSensor,
    addMixMember,
    removeMixMember,
    setChannelCurve,
    deleteCurve,
  } = useCurveEditor();
  const {
    data: settingsData,
    setMinPwm,
    setSmoothingSeconds,
  } = useChannelSettings();
  const curveNames = curveData ? Object.keys(curveData.curves) : [];
  const [page, setPage] = useState<Page>("overview");

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

      <div className="flex flex-1">
        <Sidebar page={page} onChange={setPage} />

        <main className="mx-auto flex w-full max-w-[1080px] flex-col gap-6 px-5 py-5">
          {page === "overview" &&
            (latest ? (
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
                        boundCurve={curveData?.channels[name]}
                        curveNames={curveNames}
                        setChannelCurve={setChannelCurve}
                      />
                    ))}
                  </div>
                </section>
              </>
            ) : (
              <p className="py-12 text-center text-dim">
                Waiting for the first status frame…
              </p>
            ))}

          {page === "curves" && (
            <CurvesPage
              data={curveData}
              temps={latest?.temps ?? {}}
              setCurvePoints={setCurvePoints}
              createGraphCurve={createGraphCurve}
              setGraphSensor={setGraphSensor}
              addMixMember={addMixMember}
              removeMixMember={removeMixMember}
              deleteCurve={deleteCurve}
            />
          )}

          {page === "settings" && (
            <SettingsPage
              data={settingsData}
              labels={CHANNEL_LABELS}
              setMinPwm={setMinPwm}
              setSmoothingSeconds={setSmoothingSeconds}
            />
          )}
        </main>
      </div>
    </div>
  );
}

export default App;
