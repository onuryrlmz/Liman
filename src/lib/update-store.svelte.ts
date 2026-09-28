import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { errText } from "./api";

type State =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "latest" }
  | { kind: "available"; version: string; notes: string }
  | { kind: "downloading"; done: number; total: number }
  | { kind: "ready" }
  | { kind: "error"; message: string };

class UpdaterStore {
  state = $state<State>({ kind: "idle" });
  /** Kullanıcı bu oturumda bildirimi kapattıysa tekrar gösterme. */
  dismissed = $state(false);
  private update: Update | null = null;

  async check(silent = false) {
    if (this.state.kind === "checking" || this.state.kind === "downloading") return;
    this.state = { kind: "checking" };
    try {
      this.update = await check();
      this.state = this.update
        ? { kind: "available", version: this.update.version, notes: this.update.body ?? "" }
        : { kind: "latest" };
    } catch (e) {
      // Sessiz denetimde (açılışta, çevrimdışı vb.) hata gösterme.
      this.state = silent ? { kind: "idle" } : { kind: "error", message: errText(e) };
    }
  }

  async install() {
    if (!this.update) return;
    let done = 0;
    let total = 0;
    this.state = { kind: "downloading", done, total };
    try {
      await this.update.downloadAndInstall((ev) => {
        if (ev.event === "Started") total = ev.data.contentLength ?? 0;
        else if (ev.event === "Progress") done += ev.data.chunkLength;
        this.state = { kind: "downloading", done, total };
      });
      this.state = { kind: "ready" };
    } catch (e) {
      this.state = { kind: "error", message: errText(e) };
    }
  }

  restart() {
    return relaunch();
  }
}

export const updater = new UpdaterStore();
