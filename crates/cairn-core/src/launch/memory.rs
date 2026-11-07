//! Automatic heap memory calculation and garbage collection tuning.

use crate::config::{GcPolicy, MemoryPolicy};
use crate::platform::MemoryInfo;

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

#[derive(Debug, Clone, Copy)]
pub struct MemoryInput {
    pub mod_count: usize,
    pub has_optifine: bool,
    pub modded: bool,
    pub mem: MemoryInfo,
}

/// Calculates recommended maximum heap size (in MiB) based on mod count and available system memory.
pub fn auto_heap_mib(input: MemoryInput) -> u32 {
    let n = input.mod_count as f64;
    let available = input.mem.available as f64 / GIB;
    let total = input.mem.total as f64 / GIB;

    let (vanilla_base, modded_base, soft_cap) = if total >= 24.0 {
        (4.5, 5.5, if n > 350.0 { 14.0 } else { 12.0 })
    } else if total >= 12.0 {
        (4.0, 5.0, if n > 350.0 { 10.0 } else { 8.5 })
    } else if total >= 7.0 {
        (3.25, 4.0, 6.0)
    } else {
        (2.0, 2.5, 3.0)
    };

    let (floor, recommended) = if input.modded {
        let fl = if total >= 8.0 { (3.0 + n / 100.0).min(5.0) } else { (2.0 + n / 120.0).min(3.5) };
        // Add roughly 25 MiB of heap per installed mod.
        (fl, (modded_base + n * 0.025).min(soft_cap))
    } else if input.has_optifine {
        let fl = if total >= 8.0 { 3.0 } else { 2.0 };
        (fl, vanilla_base + 1.0)
    } else {
        let fl = if total >= 8.0 { 2.5 } else { 1.5 };
        (fl, vanilla_base)
    };

    // Reserve memory headroom for the operating system and background processes.
    let os_reserve = (total * 0.18).clamp(1.2, 4.0);
    let hard_cap = (total - os_reserve).clamp(1.5, 16.0);

    // Dynamic available memory buffer to avoid paging spikes at startup.
    let avail_reserve = (available * 0.15).clamp(0.5, 1.5);
    let avail_budget = (available - avail_reserve).max(floor);

    let gib = recommended.min(avail_budget).max(floor).min(hard_cap);
    let mib = (gib * 1024.0 / 256.0).round() * 256.0;
    mib.max(512.0) as u32
}

pub fn heap_mib(policy: MemoryPolicy, input: MemoryInput) -> u32 {
    match policy {
        MemoryPolicy::Auto => auto_heap_mib(input),
        MemoryPolicy::Fixed { mib } => mib.max(256),
    }
}

/// Generates GC flags unless overridden by user arguments.
pub fn gc_args(policy: GcPolicy, java_major: u32, heap_mib: u32, user_args: &[String]) -> Vec<String> {
    let user_picked_gc = user_args.iter().any(|a| a.starts_with("-XX:+Use") && a.ends_with("GC"));
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: &str| out.push(s.to_string());
    // Preserve full stack traces in fast throw for crash diagnostics.
    push("-XX:-OmitStackTraceInFastThrow");
    if java_major <= 7 {
        push("-XX:MaxPermSize=256m");
    }
    if policy == GcPolicy::None || user_picked_gc {
        return out;
    }
    let use_zgc = policy == GcPolicy::Zgc && java_major >= 15;
    push("-XX:+UnlockExperimentalVMOptions");
    if use_zgc {
        push("-XX:+UseZGC");
        // Generational ZGC must be explicitly enabled on Java 21-22; default in 23, and removed in 24.
        if (21..=22).contains(&java_major) {
            push("-XX:+ZGenerational");
        }
    } else {
        push("-XX:+UseG1GC");
        push("-XX:G1NewSizePercent=20");
        push("-XX:G1ReservePercent=20");
        push("-XX:MaxGCPauseMillis=50");
        out.push(format!("-XX:G1HeapRegionSize={}M", if heap_mib >= 4096 { 16 } else { 8 }));
        if java_major == 8 {
            out.push("-XX:+ParallelRefProcEnabled".into());
        }
        if java_major >= 12 {
            // Return unused heap to the OS when idle.
            out.push("-XX:MinHeapFreeRatio=20".into());
            out.push("-XX:MaxHeapFreeRatio=40".into());
        }
    }
    if java_major >= 25 {
        // Compact object headers save 10% to 20% heap in typical workloads.
        out.push("-XX:+UseCompactObjectHeaders".into());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem(total_gib: u64, avail_gib: u64) -> MemoryInfo {
        MemoryInfo { total: total_gib << 30, available: avail_gib << 30 }
    }

    #[test]
    fn vanilla_gets_generous_baseline() {
        let m32 = auto_heap_mib(MemoryInput { mod_count: 0, has_optifine: false, modded: false, mem: mem(32, 20) });
        assert_eq!(m32, 4608);
        let m16 = auto_heap_mib(MemoryInput { mod_count: 0, has_optifine: false, modded: false, mem: mem(16, 10) });
        assert_eq!(m16, 4096);
    }

    #[test]
    fn large_pack_scaling() {
        let m = auto_heap_mib(MemoryInput { mod_count: 300, has_optifine: false, modded: true, mem: mem(32, 20) });
        assert_eq!(m, 12288);
        let m = auto_heap_mib(MemoryInput { mod_count: 150, has_optifine: false, modded: true, mem: mem(32, 20) });
        assert_eq!(m, 9472);
    }

    #[test]
    fn heavy_pack_is_capped_by_free_memory() {
        let m = auto_heap_mib(MemoryInput { mod_count: 300, has_optifine: false, modded: true, mem: mem(16, 6) });
        assert_eq!(m, 5120);
    }

    #[test]
    fn never_below_floor() {
        let m = auto_heap_mib(MemoryInput { mod_count: 150, has_optifine: false, modded: true, mem: mem(8, 1) });
        assert_eq!(m, 4608);
    }

    #[test]
    fn user_gc_choice_wins() {
        let args = gc_args(GcPolicy::Auto, 21, 4096, &["-XX:+UseShenandoahGC".into()]);
        assert!(!args.iter().any(|a| a == "-XX:+UseG1GC"));
    }
}
