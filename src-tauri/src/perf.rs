//! Menu « Performance PC » : CPU (par cœur, fréquence), RAM, disque, réseau, applications gourmandes.
//! Mêmes sources que le Gestionnaire des tâches : compteurs PDH + liste des processus tenue par Windows.
//! Aucun handle ouvert sur un processus (donc jamais sur un jeu), aucune lecture mémoire.
use crate::hw::Pdh;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use windows::core::HSTRING;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ};
use windows::Win32::System::SystemInformation::{GetTickCount64, GlobalMemoryStatusEx, MEMORYSTATUSEX};

const COUNTERS: [&str; 8] = [
    r"\Processor Information(*)\% Processor Utility",
    r"\Processor Information(_Total)\% Processor Performance",
    r"\Processor Information(_Total)\Processor Frequency",
    r"\PhysicalDisk(_Total)\Disk Read Bytes/sec",
    r"\PhysicalDisk(_Total)\Disk Write Bytes/sec",
    r"\PhysicalDisk(_Total)\% Idle Time",
    r"\Network Interface(*)\Bytes Received/sec",
    r"\Network Interface(*)\Bytes Sent/sec",
];

#[link(name = "ntdll")]
extern "system" {
    fn NtQuerySystemInformation(class: u32, buf: *mut core::ffi::c_void, len: u32, ret: *mut u32) -> i32;
}

const CPU_KEY: &str = r"HARDWARE\DESCRIPTION\System\CentralProcessor\0";

fn cpu_name() -> &'static str {
    static N: OnceLock<String> = OnceLock::new();
    N.get_or_init(|| unsafe {
        let mut buf = [0u16; 128];
        let mut len = 256u32;
        let r = RegGetValueW(
            HKEY_LOCAL_MACHINE,
            &HSTRING::from(CPU_KEY),
            &HSTRING::from("ProcessorNameString"),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as _),
            Some(&mut len),
        );
        if r != ERROR_SUCCESS {
            return String::new();
        }
        let n = buf.iter().position(|&c| c == 0).unwrap_or(128);
        let s = String::from_utf16_lossy(&buf[..n]);
        s.split_whitespace().collect::<Vec<_>>().join(" ")
    })
}

fn base_mhz() -> f64 {
    static M: OnceLock<f64> = OnceLock::new();
    *M.get_or_init(|| unsafe {
        let mut d = 0u32;
        let mut len = 4u32;
        let r = RegGetValueW(
            HKEY_LOCAL_MACHINE,
            &HSTRING::from(CPU_KEY),
            &HSTRING::from("~MHz"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut d as *mut u32 as _),
            Some(&mut len),
        );
        if r == ERROR_SUCCESS { d as f64 } else { 0.0 }
    })
}

struct Proc {
    pid: u32,
    name: String,
    time: u64,
    ram: u64,
    threads: u32,
}

/// Liste des processus (nom, temps CPU, mémoire privée) fournie par Windows en un seul appel.
fn processes() -> Vec<Proc> {
    let mut size = 1u32 << 20;
    for _ in 0..6 {
        let mut buf = vec![0u64; size as usize / 8];
        let mut ret = 0u32;
        let st = unsafe { NtQuerySystemInformation(5, buf.as_mut_ptr() as _, size, &mut ret) };
        if st == 0xC000_0004u32 as i32 {
            size = ret.max(size) + (256 << 10);
            continue;
        }
        if st != 0 {
            return vec![];
        }
        let mut out = Vec::new();
        let base = buf.as_ptr() as *const u8;
        let end = size as usize;
        let mut off = 0usize;
        unsafe {
            while off + 88 <= end {
                let p = base.add(off);
                let rd32 = |o: usize| std::ptr::read_unaligned(p.add(o) as *const u32);
                let rd64 = |o: usize| std::ptr::read_unaligned(p.add(o) as *const u64);
                let next = rd32(0);
                let threads = rd32(4);
                let ram = rd64(8);
                let time = rd64(40).wrapping_add(rd64(48));
                let name_len = std::ptr::read_unaligned(p.add(56) as *const u16) as usize;
                let name_ptr = std::ptr::read_unaligned(p.add(64) as *const *const u16);
                let pid = rd64(80) as u32;
                let name = if name_ptr.is_null() || name_len == 0 {
                    if pid == 0 { "Idle".to_string() } else { "System".to_string() }
                } else {
                    String::from_utf16_lossy(std::slice::from_raw_parts(name_ptr, name_len / 2))
                };
                out.push(Proc { pid, name, time, ram, threads });
                if next == 0 {
                    break;
                }
                off += next as usize;
            }
        }
        return out;
    }
    vec![]
}

const HIDDEN: [&str; 4] = ["idle", "memory compression", "secure system", "registry"];

/// Applications les plus gourmandes (regroupées par nom, comme le Gestionnaire des tâches).
fn top(logical: f64) -> (Value, Value, u32, u32) {
    static LAST: Mutex<Option<(Instant, HashMap<u32, u64>)>> = Mutex::new(None);
    let procs = processes();
    let now = Instant::now();
    let mut last = LAST.lock().unwrap();
    let (dt, prev) = match last.as_ref() {
        Some((t, m)) => (now.duration_since(*t).as_secs_f64(), Some(m)),
        None => (0.0, None),
    };
    let mut groups: HashMap<String, (String, f64, u64, u32)> = HashMap::new();
    let threads: u32 = procs.iter().map(|p| p.threads).sum();
    for p in &procs {
        let key = p.name.to_lowercase();
        if p.pid == 0 || HIDDEN.contains(&key.as_str()) {
            continue;
        }
        let cpu = match prev.and_then(|m| m.get(&p.pid)) {
            Some(&t0) if dt > 0.05 && p.time >= t0 => (p.time - t0) as f64 / (dt * 1e7 * logical) * 100.0,
            _ => 0.0,
        };
        let disp = p.name.trim_end_matches(".exe").trim_end_matches(".EXE").to_string();
        let g = groups.entry(key).or_insert((disp, 0.0, 0, 0));
        g.1 += cpu;
        g.2 += p.ram;
        g.3 += 1;
    }
    *last = Some((now, procs.iter().map(|p| (p.pid, p.time)).collect()));
    let mut v: Vec<_> = groups.into_values().collect();
    let row = |g: &(String, f64, u64, u32)| json!({ "name": g.0, "cpu": (g.1.min(100.0) * 10.0).round() / 10.0, "ram": g.2, "n": g.3 });
    v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then(b.2.cmp(&a.2)));
    let by_cpu: Vec<Value> = v.iter().take(8).map(row).collect();
    v.sort_by(|a, b| b.2.cmp(&a.2));
    let by_ram: Vec<Value> = v.iter().take(8).map(row).collect();
    (json!(by_cpu), json!(by_ram), procs.len() as u32, threads)
}

/// Mesure complète (appelée chaque seconde uniquement quand le menu Performance est ouvert).
pub fn info() -> Value {
    static Q: Mutex<Option<Pdh>> = Mutex::new(None);
    let logical = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) as f64;
    let mut q = Q.lock().unwrap();
    Pdh::collect(&mut q, &COUNTERS, 120);
    let (mut cpu, mut cores, mut freq) = (None, Vec::new(), None);
    let (mut dr, mut dw, mut dact, mut down, mut up) = (0.0, 0.0, 0.0, 0.0, 0.0);
    if let Some(p) = q.as_ref() {
        let mut per: Vec<(u32, u32, f64)> = Vec::new();
        for (n, v) in p.values(0) {
            if n == "_total" {
                cpu = Some(v.clamp(0.0, 100.0));
            } else if let Some((a, b)) = n.split_once(',') {
                if let (Ok(a), Ok(b)) = (a.parse::<u32>(), b.parse::<u32>()) {
                    per.push((a, b, v.clamp(0.0, 100.0)));
                }
            }
        }
        per.sort_by_key(|x| (x.0, x.1));
        cores = per.into_iter().map(|x| (x.2 * 10.0).round() / 10.0).collect();
        let perf = p.values(1).first().map(|x| x.1);
        let nominal = p.values(2).first().map(|x| x.1).filter(|v| *v > 0.0).unwrap_or_else(base_mhz);
        if let Some(pf) = perf {
            if nominal > 0.0 {
                freq = Some(nominal * pf / 100.0);
            }
        }
        let first = |i: usize| p.values(i).first().map(|x| x.1).unwrap_or(0.0);
        dr = first(3);
        dw = first(4);
        dact = p.values(5).first().map(|x| (100.0 - x.1).clamp(0.0, 100.0)).unwrap_or(0.0);
        down = p.values(6).iter().map(|x| x.1).sum();
        up = p.values(7).iter().map(|x| x.1).sum();
    }
    drop(q);
    let cpu = cpu.unwrap_or_else(|| crate::sys::info().0);

    let mut m = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    let _ = unsafe { GlobalMemoryStatusEx(&mut m) };
    let gb = |b: f64| b / 1073741824.0;
    let mut pi = PERFORMANCE_INFORMATION { cb: std::mem::size_of::<PERFORMANCE_INFORMATION>() as u32, ..Default::default() };
    let has_pi = unsafe { GetPerformanceInfo(&mut pi, pi.cb) }.is_ok();
    let page = pi.PageSize as f64;
    let (top_cpu, top_ram, nproc, threads) = top(logical);

    json!({
        "cpu": cpu,
        "cores": cores,
        "cpu_name": cpu_name(),
        "logical": logical as u32,
        "freq": freq.map(|f| f / 1000.0),
        "base": if base_mhz() > 0.0 { Some(base_mhz() / 1000.0) } else { None },
        "uptime": unsafe { GetTickCount64() } / 1000,
        "procs": nproc,
        "threads": threads,
        "handles": if has_pi { Some(pi.HandleCount) } else { None },
        "ram_total": gb(m.ullTotalPhys as f64),
        "ram_used": gb((m.ullTotalPhys - m.ullAvailPhys) as f64),
        "ram_cache": if has_pi { Some(gb(pi.SystemCache as f64 * page)) } else { None },
        "commit": if has_pi { Some(gb(pi.CommitTotal as f64 * page)) } else { None },
        "commit_limit": if has_pi { Some(gb(pi.CommitLimit as f64 * page)) } else { None },
        "disk_r": dr,
        "disk_w": dw,
        "disk_act": dact,
        "net_down": down,
        "net_up": up,
        "top_cpu": top_cpu,
        "top_ram": top_ram,
    })
}
