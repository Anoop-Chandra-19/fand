import { CloseButton, Dialog } from "../adw/Dialog";
import { ActionRow, BoxedList } from "../adw/rows";

export function AboutDialog({
  connected,
  onClose,
}: {
  connected: boolean | null;
  onClose: () => void;
}) {
  return (
    <Dialog width={400} label="About fand" onClose={onClose}>
      <div className="flex shrink-0 justify-end px-3 pt-2.5">
        <CloseButton onClose={onClose} />
      </div>
      <div className="flex flex-col items-center gap-1.5 overflow-auto px-6 pb-6.5 pt-1 text-center">
        <div className="text-[1.81rem] font-extrabold tracking-[-0.01em]">fand</div>
        <div className="numeric text-[0.82rem] text-dim">version 0.1.0</div>
        <p className="mb-1 mt-1.5 max-w-75 leading-[1.45] text-dim">
          A fan-control daemon, CLI and GUI for Linux — one privileged daemon owns the fan
          hardware and the thermal failsafe.
        </p>
        <BoxedList className="w-full text-left">
          <ActionRow
            title="Daemon"
            trailing={
              <span
                className={`numeric text-[0.82rem] ${connected ? "text-success" : "text-error"}`}
              >
                {connected ? "connected" : "unreachable"}
              </span>
            }
          />
          <ActionRow
            title="License"
            trailing={<span className="text-[0.82rem] text-dim">MIT</span>}
          />
          <ActionRow
            title="Built with"
            trailing={<span className="text-[0.82rem] text-dim">Tauri · React</span>}
          />
        </BoxedList>
      </div>
    </Dialog>
  );
}
