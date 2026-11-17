/**
 * Typed bridge to the Rust core.
 *
 * PLAN.md §6 risk 5: never send a frame over IPC. Only state changes cross this
 * boundary — playhead moves, style edits, project lists, progress numbers.
 */
import { invoke } from "@tauri-apps/api/core";

/** Round-trips a string through Rust to prove the IPC bridge is wired. */
export async function ping(message: string): Promise<string> {
  return invoke<string>("ping", { message });
}

export interface CoreInfo {
  /** Crate version reported by the Rust side. */
  version: string;
  /** "windows" on a real build. */
  platform: string;
  /** Port stage currently implemented (PLAN.md §5). */
  stage: number;
}

export async function coreInfo(): Promise<CoreInfo> {
  return invoke<CoreInfo>("core_info");
}
