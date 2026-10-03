//! Bağlı sunucunun kaynak durumu (CPU, bellek, disk, yük, ağ): MobaXterm'in alt çubuğu gibi.
//!
//! Mevcut SSH bağlantısında ayrı bir kanalda `sh -s` çalıştırılır ve aşağıdaki betik
//! standart girdiden verilir; kullanıcının kabuğu (fish vb.) ne olursa olsun çalışır.
//! CPU ve ağ hızı için iki ölçüm arasındaki fark kullanılır.

use std::{collections::HashMap, time::Instant};

use serde::Serialize;

pub const SCRIPT: &str = r#"
echo "@os"; uname -s
case "$(uname -s)" in
Linux)
  echo "@cpu"; head -n 1 /proc/stat
  echo "@mem"; grep -E '^(MemTotal|MemAvailable|MemFree|Buffers|Cached|SwapTotal|SwapFree):' /proc/meminfo
  echo "@load"; cat /proc/loadavg
  echo "@uptime"; cut -d' ' -f1 /proc/uptime
  echo "@ncpu"; nproc 2>/dev/null || grep -c '^processor' /proc/cpuinfo
  echo "@net"; tail -n +3 /proc/net/dev
  echo "@df"; df -kP 2>/dev/null
  ;;
Darwin)
  echo "@cpupct"; top -l 2 -n 0 -s 1 2>/dev/null | grep 'CPU usage' | tail -n 1
  echo "@darwinmem"; sysctl -n hw.memsize; vm_stat
  echo "@load"; sysctl -n vm.loadavg
  echo "@boot"; sysctl -n kern.boottime; date +%s
  echo "@ncpu"; sysctl -n hw.ncpu
  echo "@df"; df -kP 2>/dev/null
  ;;
*)
  echo "@df"; df -kP 2>/dev/null
  ;;
esac
"#;

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Disk {
    pub mount: String,
    pub total: u64,
    pub used: u64,
}

/// Tek bir ölçümün ham değerleri.
#[derive(Clone, Debug, Default)]
pub struct Sample {
    pub at: Option<Instant>,
    pub os: String,
    /// Linux /proc/stat: (toplam, boşta) jiffy.
    pub cpu_counters: Option<(u64, u64)>,
    /// macOS top: doğrudan yüzde.
    pub cpu_percent: Option<f64>,
    pub mem_total: u64,
    pub mem_used: u64,
    pub swap_total: u64,
    pub swap_used: u64,
    pub load: Option<[f64; 3]>,
    pub uptime: Option<u64>,
    pub ncpu: Option<u32>,
    /// Geri döngü hariç toplam alınan/gönderilen bayt.
    pub net: Option<(u64, u64)>,
    pub disks: Vec<Disk>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub os: String,
    /// İlk ölçümde (Linux) fark olmadığı için boş.
    pub cpu: Option<f64>,
    pub ncpu: Option<u32>,
    pub mem_total: u64,
    pub mem_used: u64,
    pub swap_total: u64,
    pub swap_used: u64,
    pub load: Option<[f64; 3]>,
    pub uptime: Option<u64>,
    /// Bayt/sn.
    pub net_rx: Option<f64>,
    pub net_tx: Option<f64>,
    /// Ana disk (Linux'ta /, macOS'ta veri birimi).
    pub disk: Option<Disk>,
    pub disks: Vec<Disk>,
}

fn sections(out: &str) -> HashMap<&str, Vec<&str>> {
    let mut map: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut cur = "";
    for line in out.lines() {
        if let Some(name) = line.strip_prefix('@') {
            cur = name.trim();
            map.entry(cur).or_default();
        } else if !cur.is_empty() {
            map.entry(cur).or_default().push(line);
        }
    }
    map
}

fn num<T: std::str::FromStr>(s: Option<&str>) -> Option<T> {
    s?.trim().parse().ok()
}

/// "df -kP" çıktısı: yalnızca gerçek aygıtlar ve ağ paylaşımları.
fn parse_df(lines: &[&str]) -> Vec<Disk> {
    let mut disks: Vec<Disk> = lines
        .iter()
        .skip_while(|l| l.starts_with("Filesystem"))
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            if f.len() < 6 {
                return None;
            }
            let fs = f[0];
            let real = fs.starts_with("/dev/") || fs.contains(":/") || fs.starts_with("//");
            // Döngü aygıtları (snap paketleri) ve macOS sistem birimlerinin çoğu gürültüdür.
            if !real || fs.starts_with("/dev/loop") {
                return None;
            }
            let mount = f[5..].join(" ");
            if mount.starts_with("/System/Volumes/") && mount != "/System/Volumes/Data" {
                return None;
            }
            let total = f[1].parse::<u64>().ok()? * 1024;
            let used = f[2].parse::<u64>().ok()? * 1024;
            (total > 0).then_some(Disk { mount, total, used })
        })
        .collect();
    disks.dedup_by(|a, b| a.mount == b.mount);
    disks
}

pub fn parse(out: &str) -> Sample {
    let sec = sections(out);
    let first = |k: &str| sec.get(k).and_then(|v| v.first().copied());
    let mut s = Sample {
        os: first("os").unwrap_or("").trim().to_string(),
        disks: sec.get("df").map(|l| parse_df(l)).unwrap_or_default(),
        ncpu: num(first("ncpu")),
        ..Default::default()
    };

    // Linux
    if let Some(line) = first("cpu") {
        let v: Vec<u64> = line.split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
        if v.len() >= 4 {
            // boşta = idle + iowait
            let idle = v[3] + v.get(4).copied().unwrap_or(0);
            // guest/guest_nice user içinde zaten sayılır.
            let total: u64 = v.iter().take(8).sum();
            s.cpu_counters = Some((total, idle));
        }
    }
    if let Some(lines) = sec.get("mem") {
        let kb: HashMap<&str, u64> = lines
            .iter()
            .filter_map(|l| {
                let (k, v) = l.split_once(':')?;
                Some((k, v.split_whitespace().next()?.parse().ok()?))
            })
            .collect();
        let total = kb.get("MemTotal").copied().unwrap_or(0);
        // Eski çekirdeklerde MemAvailable yoktur.
        let avail = kb.get("MemAvailable").copied().unwrap_or_else(|| {
            kb.get("MemFree").copied().unwrap_or(0) + kb.get("Buffers").copied().unwrap_or(0) + kb.get("Cached").copied().unwrap_or(0)
        });
        s.mem_total = total * 1024;
        s.mem_used = total.saturating_sub(avail) * 1024;
        let st = kb.get("SwapTotal").copied().unwrap_or(0);
        s.swap_total = st * 1024;
        s.swap_used = st.saturating_sub(kb.get("SwapFree").copied().unwrap_or(0)) * 1024;
    }
    if let Some(line) = first("load") {
        // Linux: "0.10 0.20 0.30 1/234 5678"; macOS: "{ 1.23 1.45 1.67 }"
        let v: Vec<f64> = line
            .split_whitespace()
            .filter_map(|x| x.trim_matches(|c| c == '{' || c == '}').parse().ok())
            .take(3)
            .collect();
        if v.len() == 3 {
            s.load = Some([v[0], v[1], v[2]]);
        }
    }
    if let Some(line) = first("uptime") {
        s.uptime = line.trim().parse::<f64>().ok().map(|f| f as u64);
    }
    if let Some(lines) = sec.get("net") {
        let mut rx = 0u64;
        let mut tx = 0u64;
        for l in lines {
            let Some((name, rest)) = l.split_once(':') else { continue };
            if name.trim() == "lo" {
                continue;
            }
            let v: Vec<u64> = rest.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            if v.len() >= 9 {
                rx += v[0];
                tx += v[8];
            }
        }
        s.net = Some((rx, tx));
    }

    // macOS
    if let Some(line) = first("cpupct") {
        // "CPU usage: 5.12% user, 3.40% sys, 91.46% idle"
        let idle = line
            .split(',')
            .find(|p| p.contains("idle"))
            .and_then(|p| p.split_whitespace().find_map(|w| w.trim_end_matches('%').parse::<f64>().ok()));
        s.cpu_percent = idle.map(|i| (100.0 - i).clamp(0.0, 100.0));
    }
    if let Some(lines) = sec.get("darwinmem") {
        s.mem_total = num(lines.first().copied()).unwrap_or(0);
        let page: u64 = lines
            .iter()
            .find_map(|l| l.split("page size of ").nth(1)?.split_whitespace().next()?.parse().ok())
            .unwrap_or(4096);
        let pages = |key: &str| -> u64 {
            lines
                .iter()
                .find(|l| l.starts_with(key))
                .and_then(|l| l.rsplit(':').next()?.trim().trim_end_matches('.').parse().ok())
                .unwrap_or(0)
        };
        // Etkinlik Monitörü'ndeki "kullanılan bellek": uygulama + kablolu + sıkıştırılmış.
        let used = pages("Pages active") + pages("Pages wired down") + pages("Pages occupied by compressor");
        s.mem_used = (used * page).min(s.mem_total);
    }
    if let Some(lines) = sec.get("boot") {
        // "{ sec = 1727000000, usec = 0 } ..." ve ardından şimdiki zaman.
        let boot: Option<u64> = lines
            .first()
            .and_then(|l| l.split("sec = ").nth(1)?.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok());
        let now: Option<u64> = num(lines.get(1).copied());
        if let (Some(b), Some(n)) = (boot, now) {
            s.uptime = Some(n.saturating_sub(b));
        }
    }
    s
}

/// Ana disk: Linux'ta "/", macOS'ta veri birimi; yoksa en büyüğü.
fn main_disk(disks: &[Disk]) -> Option<Disk> {
    disks
        .iter()
        .find(|d| d.mount == "/System/Volumes/Data")
        .or_else(|| disks.iter().find(|d| d.mount == "/"))
        .or_else(|| disks.iter().max_by_key(|d| d.total))
        .cloned()
}

/// Yeni ölçümü öncekiyle birleştirip gösterilecek değerleri hesaplar.
pub fn compute(prev: Option<&Sample>, cur: &Sample) -> Stats {
    let secs = match (prev.and_then(|p| p.at), cur.at) {
        (Some(a), Some(b)) => b.duration_since(a).as_secs_f64(),
        _ => 0.0,
    };
    let cpu = cur.cpu_percent.or_else(|| {
        let (pt, pi) = prev?.cpu_counters?;
        let (ct, ci) = cur.cpu_counters?;
        let dt = ct.checked_sub(pt)?;
        if dt == 0 {
            return None;
        }
        let di = ci.saturating_sub(pi);
        Some(((dt - di.min(dt)) as f64 / dt as f64 * 100.0).clamp(0.0, 100.0))
    });
    let rate = |f: fn(&(u64, u64)) -> u64| -> Option<f64> {
        if secs <= 0.0 {
            return None;
        }
        let a = f(prev?.net.as_ref()?);
        let b = f(cur.net.as_ref()?);
        // Sayaç sıfırlandıysa (arayüz yeniden başladı) hız gösterme.
        b.checked_sub(a).map(|d| d as f64 / secs)
    };
    Stats {
        os: cur.os.clone(),
        cpu,
        ncpu: cur.ncpu,
        mem_total: cur.mem_total,
        mem_used: cur.mem_used,
        swap_total: cur.swap_total,
        swap_used: cur.swap_used,
        load: cur.load,
        uptime: cur.uptime,
        net_rx: rate(|n| n.0),
        net_tx: rate(|n| n.1),
        disk: main_disk(&cur.disks),
        disks: cur.disks.clone(),
    }
}
