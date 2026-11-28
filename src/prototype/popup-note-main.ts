import { createApp } from "vue";
import "../styles/tokens.css";
import "../styles/base.css";
import "../styles/controls.css";
import { i18n, applyLanguage } from "../i18n";
import { applyTheme } from "../lib/theme";
import { installMock, readParams } from "./noteMock";

// The mock backend must be in place before the popup core asks for its init payload.
const params = readParams();
installMock(params);
const root = document.documentElement;
root.classList.add("rp-frame", "macos");
applyTheme(params.theme);
applyLanguage("zh");
// The Sidebar prints a request's kind as a small tag; "note" is the new one.
i18n.global.mergeLocaleMessage("zh", { popup: { inbox: { kind: { note: "留言" } } } });

void import("./NoteFrame.vue").then(({ default: Frame }) => {
  createApp(Frame).use(i18n).mount("#app");
});
