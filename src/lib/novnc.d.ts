// noVNC'nin Liman'da kullanılan kısmı için tür tanımı (paket kendi tanımını içermiyor).
declare module "@novnc/novnc" {
  export default class RFB extends EventTarget {
    constructor(
      target: HTMLElement,
      urlOrChannel: string | WebSocket,
      options?: { credentials?: { username?: string; password?: string; target?: string } },
    );
    scaleViewport: boolean;
    resizeSession: boolean;
    viewOnly: boolean;
    disconnect(): void;
    sendCredentials(credentials: { username?: string; password?: string; target?: string }): void;
    sendCtrlAltDel(): void;
    clipboardPasteFrom(text: string): void;
    focus(): void;
  }
}
