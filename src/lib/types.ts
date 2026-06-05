export interface AskRequest {
  id: string;
  isMarkdown: boolean;
  message: MessagePrompt;
  questions: Question[];
}

export interface MessagePrompt {
  text: string;
  files: FileAttachment[];
}

export interface Question {
  message: string;
  predefinedOptions: string[];
}

export interface FileAttachment {
  path: string;
  name: string;
  size: number;
  isImage: boolean;
}

export interface ImageAttachment {
  data: string;
  mediaType: string;
  filename?: string | null;
}

export type ThemeMode = "system" | "light" | "dark";

export type PopupAnimation = "none" | "document" | "alert";

export type WindowEffect = "glass" | "blur";

export interface PopupInit {
  request: AskRequest;
  theme: ThemeMode;
  alwaysOnTop: boolean;
  sourceName: string;
}

export interface QuestionAnswer {
  selectedOptions: string[];
  userInput: string;
  images: ImageAttachment[];
  files: string[];
}

export interface PopupSubmission {
  answers: QuestionAnswer[];
}

export interface GeneralConfig {
  theme: ThemeMode;
  alwaysOnTop: boolean;
  appearAnimation: PopupAnimation;
  windowEffect: WindowEffect;
}

export interface PopupChannelConfig {
  enabled: boolean;
  width: number;
  height: number;
  rememberSize: boolean;
}

export interface TelegramChannelConfig {
  enabled: boolean;
  botToken: string;
  chatId: string;
  apiBaseUrl: string;
}

export interface DingTalkChannelConfig {
  enabled: boolean;
  clientId: string;
  clientSecret: string;
  userId: string;
}

export interface ChannelsConfig {
  popup: PopupChannelConfig;
  telegram: TelegramChannelConfig;
  dingding: DingTalkChannelConfig;
}

export interface AppConfig {
  general: GeneralConfig;
  channels: ChannelsConfig;
}

export interface HookStatus {
  installed: boolean;
  hooksJsonExists: boolean;
  supported: boolean;
}

export interface TelegramTestArgs {
  botToken: string;
  chatId: string;
  apiBaseUrl: string;
}

export interface DingTalkTestArgs {
  clientId: string;
  clientSecret: string;
  userId: string;
}

export interface DingTalkDetectArgs {
  clientId: string;
  clientSecret: string;
}

export interface DingTalkWaitArgs {
  clientId: string;
  clientSecret: string;
  code: string;
}
