import { createApp } from "vue";
import "../styles/tokens.css";
import "../styles/base.css";
import "../styles/controls.css";
import "./macAway.css";
import { applyTheme } from "../lib/theme";
import MacAwayBoard from "./MacAwayBoard.vue";

const q = new URLSearchParams(location.search);
document.documentElement.classList.add("ma-frame", "macos");
applyTheme(q.get("theme") === "dark" ? "dark" : "light");
document.body.style.background = q.get("theme") === "dark" ? "#0f0f11" : "#f4f4f6";
const board = q.get("board");
createApp(MacAwayBoard, { kind: board === "tray" ? "tray" : board === "place" ? "place" : "hud" }).mount("#app");
