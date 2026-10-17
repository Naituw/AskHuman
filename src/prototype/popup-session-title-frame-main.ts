import { createApp } from "vue";
import "../styles/tokens.css";
import "../styles/base.css";
import "../styles/controls.css";
import { i18n, applyLanguage } from "../i18n";
import { applyTheme } from "../lib/theme";
import { installMock, readParams } from "./sessionTitleMock";

// The mock backend must be in place before the popup core asks for its init payload.
const params = readParams();
installMock(params);
const root = document.documentElement;
root.classList.add("pt-frame");
if (params.platform === "mac") root.classList.add("macos");
applyTheme(params.theme);
applyLanguage("zh");

void import("./SessionTitleFrame.vue").then(({ default: Frame }) => {
  createApp(Frame).use(i18n).mount("#app");
});
