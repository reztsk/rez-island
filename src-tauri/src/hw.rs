//! GPU, VRAM, températures, débit réseau.
//! Mêmes sources que le Gestionnaire des tâches : compteurs de performance (PDH), DXGI, D3DKMT.
//! Aucun pilote noyau (bloqués par les anti-cheats), aucun accès aux jeux.
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use windows::core::{HSTRING, PCWSTR};
use windows::Wdk::Graphics::Direct3D::{
    D3DKMTCloseAdapter, D3DKMTEnumAdapters2, D3DKMTQueryAdapterInfo, D3DKMT_ADAPTERINFO, D3DKMT_ADAPTER_PERFDATA,
    D3DKMT_CLOSEADAPTER, D3DKMT_ENUMADAPTERS2, D3DKMT_QUERYADAPTERINFO, KMTQAITYPE_ADAPTERPERFDATA,
};
use windows::Win32::Foundation::LUID;
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};
use windows::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhOpenQueryW,
    PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_MORE_DATA,
};

/// Requête PDH persistante (les compteurs « par seconde » ont besoin de deux mesures).
pub struct Pdh {
    q: isize,
    counters: Vec<isize>,
    uses: u32,
}

impl Pdh {
    fn open(paths: &[&str]) -> Option<Pdh> {
        unsafe {
            let mut q = 0isize;
            if PdhOpenQueryW(PCWSTR::null(), 0, &mut q) != 0 {
                return None;
            }
            let counters = paths
                .iter()
                .map(|p| {
                    let mut c = 0isize;
                    if PdhAddEnglishCounterW(q, &HSTRING::from(*p), 0, &mut c) == 0 { c } else { 0 }
                })
                .collect();
            PdhCollectQueryData(q);
            Some(Pdh { q, counters, uses: 0 })
        }
    }

    /// Nouvelle mesure ; la requête est recréée de temps en temps (nouvelles instances).
    pub fn collect(slot: &mut Option<Pdh>, paths: &[&str], renew: u32) {
        if slot.as_ref().map(|p| p.uses >= renew).unwrap_or(false) {
            *slot = None;
        }
        match slot.as_mut() {
            None => *slot = Pdh::open(paths),
            Some(p) => {
                p.uses += 1;
                unsafe { PdhCollectQueryData(p.q) };
            }
        }
    }

    /// (instance en minuscules, valeur) d'un compteur à joker.
    pub fn values(&self, i: usize) -> Vec<(String, f64)> {
        let c = self.counters.get(i).copied().unwrap_or(0);
        if c == 0 {
            return vec![];
        }
        unsafe {
            let (mut size, mut count) = (0u32, 0u32);
            if PdhGetFormattedCounterArrayW(c, PDH_FMT_DOUBLE, &mut size, &mut count, None) != PDH_MORE_DATA || size == 0 {
                return vec![];
            }
            let mut buf = vec![0u64; size as usize / 8 + 2];
            let items = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
            if PdhGetFormattedCounterArrayW(c, PDH_FMT_DOUBLE, &mut size, &mut count, Some(items)) != 0 {
                return vec![];
            }
            (0..count as usize)
                .filter_map(|k| {
                    let it = &*items.add(k);
                    if it.FmtValue.CStatus > 1 {
                        return None;
                    }
                    let name = it.szName.to_string().unwrap_or_default().to_lowercase();
                    Some((name, it.FmtValue.Anonymous.doubleValue))
                })
                .collect()
        }
    }
}

impl Drop for Pdh {
    fn drop(&mut self) {
        unsafe { PdhCloseQuery(self.q) };
    }
}

struct Gpu {
    name: String,
    luid: LUID,
    tag: String,
    vram_total: f64,
}

/// Carte graphique principale (la plus grosse VRAM dédiée, hors adaptateur logiciel).
fn main_gpu() -> Option<&'static Gpu> {
    static G: OnceLock<Option<Gpu>> = OnceLock::new();
    G.get_or_init(|| unsafe {
        let f: IDXGIFactory1 = CreateDXGIFactory1().ok()?;
        let mut best: Option<Gpu> = None;
        let mut i = 0;
        while let Ok(a) = f.EnumAdapters1(i) {
            i += 1;
            let Ok(d) = a.GetDesc1() else { continue };
            if d.Flags & 2 != 0 || d.VendorId == 0x1414 {
                continue; // adaptateur logiciel (Microsoft Basic Render)
            }
            let vram = d.DedicatedVideoMemory as f64 / 1073741824.0;
            if best.as_ref().map(|b| vram > b.vram_total).unwrap_or(true) {
                let len = d.Description.iter().position(|&c| c == 0).unwrap_or(128);
                best = Some(Gpu {
                    name: String::from_utf16_lossy(&d.Description[..len]).trim().to_string(),
                    luid: d.AdapterLuid,
                    tag: format!("luid_0x{:08x}_0x{:08x}", d.AdapterLuid.HighPart as u32, d.AdapterLuid.LowPart),
                    vram_total: vram,
                });
            }
        }
        best
    })
    .as_ref()
}

/// Température GPU (°C) fournie par le pilote (WDDM 2.4+), comme le Gestionnaire des tâches.
fn gpu_temp(luid: LUID) -> Option<f64> {
    unsafe {
        let mut e = D3DKMT_ENUMADAPTERS2::default();
        if D3DKMTEnumAdapters2(&mut e).0 < 0 || e.NumAdapters == 0 {
            return None;
        }
        let mut list = vec![D3DKMT_ADAPTERINFO::default(); e.NumAdapters as usize];
        e.pAdapters = list.as_mut_ptr();
        if D3DKMTEnumAdapters2(&mut e).0 < 0 {
            return None;
        }
        let n = (e.NumAdapters as usize).min(list.len());
        let mut out = None;
        for a in &list[..n] {
            if a.AdapterLuid.LowPart == luid.LowPart && a.AdapterLuid.HighPart == luid.HighPart {
                let mut pd = D3DKMT_ADAPTER_PERFDATA::default();
                let mut q = D3DKMT_QUERYADAPTERINFO {
                    hAdapter: a.hAdapter,
                    Type: KMTQAITYPE_ADAPTERPERFDATA,
                    pPrivateDriverData: &mut pd as *mut _ as *mut core::ffi::c_void,
                    PrivateDriverDataSize: std::mem::size_of::<D3DKMT_ADAPTER_PERFDATA>() as u32,
                };
                if D3DKMTQueryAdapterInfo(&mut q).0 >= 0 && pd.Temperature > 0 {
                    let t = pd.Temperature as f64 / 10.0;
                    if (5.0..130.0).contains(&t) {
                        out = Some(t);
                    }
                }
            }
            let _ = D3DKMTCloseAdapter(&D3DKMT_CLOSEADAPTER { hAdapter: a.hAdapter });
        }
        out
    }
}

const SENSORS: [&str; 3] = [
    r"\GPU Engine(*)\Utilization Percentage",
    r"\GPU Adapter Memory(*)\Dedicated Usage",
    r"\Thermal Zone Information(*)\High Precision Temperature",
];

/// GPU % / VRAM / températures CPU et GPU (null = non fourni par ce PC).
pub fn sensors() -> Value {
    static Q: Mutex<Option<Pdh>> = Mutex::new(None);
    let gpu = main_gpu();
    let tag = gpu.map(|g| g.tag.as_str()).unwrap_or("");
    let mut q = Q.lock().unwrap();
    Pdh::collect(&mut q, &SENSORS, 40);
    let (mut usage, mut vram, mut cpu_t) = (None, None, None);
    if let Some(p) = q.as_ref() {
        let eng = p.values(0);
        if !eng.is_empty() {
            // moteur 3D additionné par carte, puis la carte principale (sinon la plus chargée)
            let mut per: HashMap<String, f64> = HashMap::new();
            for (n, v) in eng.iter().filter(|(n, _)| n.contains("engtype_3d")) {
                if let Some(i) = n.find("luid_") {
                    let l = n[i..].split("_phys").next().unwrap_or("").to_string();
                    *per.entry(l).or_default() += v;
                }
            }
            let u = per.get(tag).copied().unwrap_or_else(|| per.values().copied().fold(0.0, f64::max));
            usage = Some(u.clamp(0.0, 100.0));
        }
        let mem = p.values(1);
        if !mem.is_empty() {
            let b = mem
                .iter()
                .filter(|(n, _)| tag.is_empty() || n.contains(tag))
                .map(|(_, v)| *v)
                .fold(0.0, f64::max);
            vram = Some(b / 1073741824.0);
        }
        cpu_t = p
            .values(2)
            .iter()
            .map(|(_, v)| v / 10.0 - 273.15)
            .filter(|t| (20.0..115.0).contains(t))
            .fold(None, |a: Option<f64>, t| Some(a.map_or(t, |a| a.max(t))));
        // beaucoup de cartes mères exposent une zone ACPI figée (fausse) : ignorée si elle ne bouge jamais
        static SPAN: Mutex<(f64, f64, u32)> = Mutex::new((f64::MAX, f64::MIN, 0));
        if let Some(t) = cpu_t {
            let mut s = SPAN.lock().unwrap();
            *s = (s.0.min(t), s.1.max(t), s.2 + 1);
            if s.2 >= 12 && s.1 - s.0 < 0.3 {
                cpu_t = None;
            }
        }
    }
    json!({
        "gpu": usage,
        "gpu_name": gpu.map(|g| g.name.clone()),
        "vram_used": vram,
        "vram_total": gpu.map(|g| g.vram_total),
        "gpu_temp": gpu.and_then(|g| gpu_temp(g.luid)),
        "cpu_temp": cpu_t,
    })
}

/// Débit descendant total du PC (octets/s), carte réseau la plus active.
pub fn net_down() -> f64 {
    static Q: Mutex<Option<Pdh>> = Mutex::new(None);
    const P: [&str; 1] = [r"\Network Interface(*)\Bytes Received/sec"];
    let mut q = Q.lock().unwrap();
    Pdh::collect(&mut q, &P, 300);
    q.as_ref()
        .map(|p| p.values(0).iter().map(|(_, v)| *v).fold(0.0, f64::max))
        .unwrap_or(0.0)
}
