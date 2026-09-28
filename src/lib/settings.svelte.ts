import type { ITheme } from "@xterm/xterm";
import { api } from "./api";

export const themes: Record<string, { label: string; theme: ITheme }> = {
  liman: {
    label: "Liman",
    theme: {
      background: "#11151c",
      foreground: "#d6dde6",
      cursor: "#7fd1b9",
      selectionBackground: "#2d4f67",
      black: "#1c2129",
      red: "#e06c75",
      green: "#98c379",
      yellow: "#e5c07b",
      blue: "#61afef",
      magenta: "#c678dd",
      cyan: "#56b6c2",
      white: "#d6dde6",
      brightBlack: "#5c6370",
      brightRed: "#ff7a85",
      brightGreen: "#b5e890",
      brightYellow: "#ffd68a",
      brightBlue: "#7cc4ff",
      brightMagenta: "#de9bf0",
      brightCyan: "#7fd1dc",
      brightWhite: "#ffffff",
    },
  },
  dracula: {
    label: "Dracula",
    theme: {
      background: "#282a36",
      foreground: "#f8f8f2",
      cursor: "#f8f8f2",
      selectionBackground: "#44475a",
      black: "#21222c",
      red: "#ff5555",
      green: "#50fa7b",
      yellow: "#f1fa8c",
      blue: "#bd93f9",
      magenta: "#ff79c6",
      cyan: "#8be9fd",
      white: "#f8f8f2",
      brightBlack: "#6272a4",
      brightRed: "#ff6e6e",
      brightGreen: "#69ff94",
      brightYellow: "#ffffa5",
      brightBlue: "#d6acff",
      brightMagenta: "#ff92df",
      brightCyan: "#a4ffff",
      brightWhite: "#ffffff",
    },
  },
  solarized: {
    label: "Solarized Dark",
    theme: {
      background: "#002b36",
      foreground: "#93a1a1",
      cursor: "#93a1a1",
      selectionBackground: "#073642",
      black: "#073642",
      red: "#dc322f",
      green: "#859900",
      yellow: "#b58900",
      blue: "#268bd2",
      magenta: "#d33682",
      cyan: "#2aa198",
      white: "#eee8d5",
      brightBlack: "#586e75",
      brightRed: "#cb4b16",
      brightGreen: "#859900",
      brightYellow: "#b58900",
      brightBlue: "#268bd2",
      brightMagenta: "#6c71c4",
      brightCyan: "#2aa198",
      brightWhite: "#fdf6e3",
    },
  },
  monokai: {
    label: "Monokai",
    theme: {
      background: "#272822",
      foreground: "#f8f8f2",
      cursor: "#f8f8f0",
      selectionBackground: "#49483e",
      black: "#272822",
      red: "#f92672",
      green: "#a6e22e",
      yellow: "#f4bf75",
      blue: "#66d9ef",
      magenta: "#ae81ff",
      cyan: "#a1efe4",
      white: "#f8f8f2",
      brightBlack: "#75715e",
      brightRed: "#f92672",
      brightGreen: "#a6e22e",
      brightYellow: "#f4bf75",
      brightBlue: "#66d9ef",
      brightMagenta: "#ae81ff",
      brightCyan: "#a1efe4",
      brightWhite: "#f9f8f5",
    },
  },
  light: {
    label: "Açık",
    theme: {
      background: "#fbfbfa",
      foreground: "#383a42",
      cursor: "#526eff",
      selectionBackground: "#d7e3f4",
      black: "#383a42",
      red: "#e45649",
      green: "#50a14f",
      yellow: "#c18401",
      blue: "#4078f2",
      magenta: "#a626a4",
      cyan: "#0184bc",
      white: "#a0a1a7",
      brightBlack: "#696c77",
      brightRed: "#e45649",
      brightGreen: "#50a14f",
      brightYellow: "#c18401",
      brightBlue: "#4078f2",
      brightMagenta: "#a626a4",
      brightCyan: "#0184bc",
      brightWhite: "#fafafa",
    },
  },
};

export const fonts = [
  '"JetBrains Mono", "Cascadia Mono", Menlo, Consolas, "DejaVu Sans Mono", monospace',
  'Menlo, monospace',
  'Consolas, monospace',
  '"Cascadia Mono", "Cascadia Code", monospace',
  '"SF Mono", Menlo, monospace',
  '"Fira Code", monospace',
  '"Ubuntu Mono", monospace',
  '"DejaVu Sans Mono", monospace',
];

export interface Settings {
  fontSize: number;
  fontFamily: string;
  lineHeight: number;
  theme: string;
  cursorStyle: "block" | "bar" | "underline";
  cursorBlink: boolean;
  scrollback: number;
  copyOnSelect: boolean;
  rightClickPaste: boolean;
  autoReconnect: boolean;
  checkUpdates: boolean;
  /** Terminal çıktısını düz metin dosyasına yaz (Rust tarafı okur). */
  sessionLog: boolean;
  logDir: string;
  logTimestamps: boolean;
}

export const defaults: Settings = {
  fontSize: 13,
  fontFamily: fonts[0],
  lineHeight: 1.15,
  theme: "liman",
  cursorStyle: "block",
  cursorBlink: true,
  scrollback: 10000,
  copyOnSelect: true,
  rightClickPaste: true,
  autoReconnect: true,
  checkUpdates: true,
  sessionLog: false,
  logDir: "",
  logTimestamps: true,
};

export const MIN_FONT = 8;
export const MAX_FONT = 32;

class SettingsStore {
  value = $state<Settings>({ ...defaults });
  loaded = $state(false);
  private timer: ReturnType<typeof setTimeout> | undefined;

  async load() {
    try {
      const saved = await api.settingsGet();
      this.value = { ...defaults, ...(saved as Partial<Settings>) };
    } catch {
      /* varsayılanlarla devam */
    }
    this.loaded = true;
  }

  update(patch: Partial<Settings>) {
    Object.assign(this.value, patch);
    clearTimeout(this.timer);
    this.timer = setTimeout(() => api.settingsSet({ ...this.value }).catch(() => {}), 300);
  }

  zoom(delta: number | null) {
    const size = delta === null ? defaults.fontSize : this.value.fontSize + delta;
    this.update({ fontSize: Math.max(MIN_FONT, Math.min(MAX_FONT, size)) });
  }

  get termTheme(): ITheme {
    return (themes[this.value.theme] ?? themes.liman).theme;
  }
}

export const settings = new SettingsStore();
