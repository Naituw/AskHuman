import { createApp } from "vue";
import { invoke, isTauri } from "@tauri-apps/api/core";
import "../styles/tokens.css";
import "../styles/base.css";
import "../styles/controls.css";
import "./popup-collapse.css";
import PopupCollapseProto from "./PopupCollapseProto.vue";

// Page errors have no console in the native prototype; forward them to the same jsonl log the
// automatic checks use so a stuck run can be diagnosed.
const surface = new URLSearchParams(location.search).get("surface") ?? "popup";
function log(label: string, detail: unknown) {
  if (!isTauri()) return;
  void invoke("demo_record", { value: { label, surface, detail: String(detail) }, finished: false }).catch(() => {});
}
window.addEventListener("error", event => log("page-error", `${event.message} @ ${event.filename}:${event.lineno}`));
window.addEventListener("unhandledrejection", event => {
  const reason = event.reason as { stack?: string } | undefined;
  log("page-rejection", reason?.stack ?? event.reason);
});
log("page-load", location.href);

const app = createApp(PopupCollapseProto);
app.config.errorHandler = (error, _instance, info) => {
  log("vue-error", `${info}: ${error instanceof Error ? (error.stack ?? error.message) : String(error)}`);
  console.error(error);
};
app.mount("#app");
