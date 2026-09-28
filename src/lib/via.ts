import { api, parseQuick, type ConnectRequest } from "./api";
import { store } from "./tabs.svelte";

/**
 * VNC/RDP için tünel kurulacak SSH bağlantısını açar ve kimliğini döndürür.
 * `jump`: kayıtlı bir SSH oturumunun kimliği ya da "kullanıcı@sunucu:port".
 */
export async function openVia(jump: string, log: (s: string) => void): Promise<string> {
  const saved = store.sessions.find((s) => s.id === jump);
  let req: ConnectRequest;
  if (saved) {
    req = {
      sessionId: saved.id,
      host: saved.host,
      port: saved.port,
      username: saved.username,
      auth: saved.auth,
      keyPath: saved.keyPath,
      jump: saved.jump,
      tunnelOnly: true,
    };
  } else {
    const q = parseQuick(jump);
    if (!q) throw new Error(`Geçersiz SSH sunucusu: ${jump}`);
    req = { host: q.host, port: q.port, username: q.username, auth: "auto", tunnelOnly: true };
  }
  log(`SSH tüneli: ${req.username ? req.username + "@" : ""}${req.host}:${req.port}`);
  const decoder = new TextDecoder();
  // Tünel bağlantısının çıktısı (ör. anahtar kaydedildi mesajı) günlüğe düşer.
  return api.sshConnect(req, 80, 24, (d) => log(decoder.decode(d).replace(/\x1b\[[0-9;]*m/g, "").trim()), `tunel-${req.host}`);
}
