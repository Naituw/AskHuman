import { createApp } from "vue";
import "../styles/tokens.css";
import "../styles/base.css";
import "../styles/controls.css";
import "./macAway.css";
import { applyTheme } from "../lib/theme";
import MacAwayFrame from "./MacAwayFrame.vue";
import { readParams } from "./macAwayMock";

const params = readParams();
document.documentElement.classList.add("ma-frame", "macos");
applyTheme(params.theme);
createApp(MacAwayFrame, { view: params.view, w: params.w, h: params.h }).mount("#app");
