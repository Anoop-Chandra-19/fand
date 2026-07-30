import { invoke } from "@tauri-apps/api/core";

export function getDaemonSocket(): Promise<string> {
  return invoke<string>("daemon_socket");
}

export async function reloadDaemonConfig(): Promise<void> {
  await invoke("reload_config");
}
