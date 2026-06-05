import { invoke } from "@tauri-apps/api/core";
import type {
  AppConfig,
  DingTalkDetectArgs,
  DingTalkTestArgs,
  DingTalkWaitArgs,
  HookStatus,
  PopupInit,
  PopupSubmission,
  TelegramTestArgs,
  ThemeMode,
  WindowEffect,
} from "./types";

export const popupInit = () => invoke<PopupInit>("popup_init");

export const submitPopup = (submission: PopupSubmission) =>
  invoke<void>("submit_popup", { submission });

export const cancelPopup = () => invoke<void>("cancel_popup");

export const openPath = (path: string) => invoke<void>("open_path", { path });

export const previewAttachments = (paths: string[], index: number) =>
  invoke<void>("preview_attachments", { paths, index });

export const refocusPreview = () => invoke<void>("refocus_preview");

export const closePreview = () => invoke<void>("close_preview");

export const readImageDataUrl = (path: string) =>
  invoke<string>("read_image_data_url", { path });

export const fileIconDataUrl = (path: string) =>
  invoke<string>("file_icon_data_url", { path });

export const showAttachmentMenu = (path: string) =>
  invoke<void>("show_attachment_menu", { path });

export const getSettings = () => invoke<AppConfig>("get_settings");

export const saveSettings = (config: AppConfig) =>
  invoke<void>("save_settings", { config });

export const getPrompt = () => invoke<string>("get_prompt");

export const openTestPopup = () => invoke<void>("open_test_popup");

export const setTheme = (theme: ThemeMode) =>
  invoke<void>("set_theme", { theme });

export const updateTheme = (theme: ThemeMode) =>
  invoke<void>("update_theme", { theme });

export const openSettings = () => invoke<void>("open_settings");

export const applyWindowEffect = (effect: WindowEffect) =>
  invoke<void>("apply_window_effect", { effect });

export const startSpeech = (locale: string) =>
  invoke<void>("start_speech", { locale });

export const stopSpeech = () => invoke<void>("stop_speech");

export const flushSpeech = () => invoke<void>("flush_speech");

export const speechAvailable = () => invoke<boolean>("speech_available");

export const cursorHookStatus = () => invoke<HookStatus>("cursor_hook_status");

export const cursorHookInstall = () => invoke<string>("cursor_hook_install");

export const cursorHookUninstall = () => invoke<string>("cursor_hook_uninstall");

export const cursorHookReveal = () => invoke<void>("cursor_hook_reveal");

export const telegramTest = (args: TelegramTestArgs) =>
  invoke<string>("telegram_test", { args });

export const dingtalkTest = (args: DingTalkTestArgs) =>
  invoke<string>("dingtalk_test", { args });

export const dingtalkDetectPrepare = (args: DingTalkDetectArgs) =>
  invoke<string>("dingtalk_detect_prepare", { args });

export const dingtalkDetectWait = (args: DingTalkWaitArgs) =>
  invoke<string>("dingtalk_detect_wait", { args });
