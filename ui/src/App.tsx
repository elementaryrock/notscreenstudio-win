import { useEffect, useState } from "react";
import { coreInfo, ping, type CoreInfo } from "./ipc";
import { strings } from "./strings";

type Bridge =
  | { kind: "pending" }
  | { kind: "ok"; info: CoreInfo; echoed: string }
  | { kind: "error"; message: string };

export function App(): React.JSX.Element {
  const [bridge, setBridge] = useState<Bridge>({ kind: "pending" });

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [info, echoed] = await Promise.all([coreInfo(), ping("hello")]);
        if (!cancelled) setBridge({ kind: "ok", info, echoed });
      } catch (e) {
        if (!cancelled) {
          setBridge({ kind: "error", message: e instanceof Error ? e.message : String(e) });
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <main className="bootstrap">
      <h1>{strings.appName}</h1>
      <p className="tagline">{strings.tagline}</p>

      <section className="card">
        <h2>{strings.bootstrapTitle}</h2>
        <p>{strings.bootstrapBody}</p>

        <dl className="facts">
          <dt>{strings.platformLabel}</dt>
          <dd>{bridge.kind === "ok" ? bridge.info.platform : "—"}</dd>
          <dt>{strings.versionLabel}</dt>
          <dd>{bridge.kind === "ok" ? bridge.info.version : "—"}</dd>
          <dt>{strings.stageLabel}</dt>
          <dd>{bridge.kind === "ok" ? bridge.info.stage : "—"}</dd>
        </dl>

        <p className="status" data-state={bridge.kind}>
          {bridge.kind === "pending" && strings.ipcPending}
          {bridge.kind === "ok" && `${strings.ipcOk}: “${bridge.echoed}”`}
          {bridge.kind === "error" && `${strings.ipcFailed}: ${bridge.message}`}
        </p>
      </section>
    </main>
  );
}
