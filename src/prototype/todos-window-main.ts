// Todos-window preview entry: the real `TodosView` in a plain browser (no Tauri), backed by
// `todosWindowMock.ts`. See that file for the URL parameters.
import { createApp } from "vue";
import "../styles/tokens.css";
import "../styles/base.css";
import "../styles/controls.css";
import { i18n, applyLanguage } from "../i18n";
import { applyTheme } from "../lib/theme";
import { installMock, readParams } from "./todosWindowMock";

// The mock backend must be in place before the view asks for its init payload.
const params = readParams();
installMock(params);
const root = document.documentElement;
if (params.platform === "mac") root.classList.add("macos");
applyTheme(params.theme);
applyLanguage("zh");

void import("../views/TodosView.vue").then(({ default: TodosView }) => {
  createApp(TodosView).use(i18n).mount("#app");
});
