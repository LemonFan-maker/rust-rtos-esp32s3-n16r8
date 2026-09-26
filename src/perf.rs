//! 周期级延迟测量设施
//!
//! 直接读取Xtensa CCOUNT计数器(每CPU时钟周期加一, 240 MHz下分辨率约4.17 ns),
//! 提供零分配、无锁、可在任务与中断上下文直接调用的延迟统计能力。
//!
//! CCOUNT为32位计数器, 240 MHz下约17.9 s回绕一次; 所有区间计算均使用
//! `wrapping_sub`, 只要单次测量区间短于一个回绕周期即保证正确。

use portable_atomic::{AtomicU32, AtomicU64, Ordering};

/// 读取当前CPU周期计数(CCOUNT)。
#[inline(always)]
pub fn cycles() -> u32 {
    #[cfg(target_arch = "xtensa")]
    {
        let value: u32;
        // SAFETY: rsr.ccount仅读取只读性能计数器, 不访问内存、不改变标志位、
        // 不占用栈; 与xtensa-lx crate中get_cycle_count()的实现一致。
        unsafe {
            core::arch::asm!(
                "rsr.ccount {0}",
                out(reg) value,
                options(nostack, nomem, preserves_flags)
            );
        }
        value
    }
    #[cfg(not(target_arch = "xtensa"))]
    {
        0
    }
}

/// 返回自`start`以来经过的周期数(处理32位回绕)。
#[inline(always)]
pub fn elapsed_since(start: u32) -> u32 {
    cycles().wrapping_sub(start)
}

/// 运行时CPU频率(Hz)。须在`esp_hal::init()`完成后调用。
#[inline]
pub fn cpu_freq_hz() -> u32 {
    #[cfg(target_arch = "xtensa")]
    {
        esp_hal::clock::Clocks::get().cpu_clock.as_hz()
    }
    #[cfg(not(target_arch = "xtensa"))]
    {
        crate::config::CPU_FREQ_HZ
    }
}

/// 周期数按给定频率换算为纳秒(纯整数运算, 不依赖浮点)。
#[inline(always)]
pub const fn cycles_to_ns(cycles: u32, freq_hz: u32) -> u64 {
    (cycles as u64 * 1_000_000_000) / (freq_hz as u64)
}

/// 一次统计快照。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    pub samples: u64,
    pub min_cycles: u32,
    pub max_cycles: u32,
    pub avg_cycles: u64,
}

impl Stats {
    pub fn min_ns(&self, freq_hz: u32) -> u64 {
        cycles_to_ns(self.min_cycles, freq_hz)
    }

    pub fn max_ns(&self, freq_hz: u32) -> u64 {
        cycles_to_ns(self.max_cycles, freq_hz)
    }

    pub fn avg_ns(&self, freq_hz: u32) -> u64 {
        (self.avg_cycles * 1_000_000_000) / (freq_hz as u64)
    }
}

/// 无锁延迟统计器: min/max/sum/count全部为原子量,
/// `record()`不含临界区与分配, 可安全用于中断上下文。
pub struct LatencyStats {
    min: AtomicU32,
    max: AtomicU32,
    sum: AtomicU64,
    count: AtomicU64,
}

impl LatencyStats {
    pub const fn new() -> Self {
        Self {
            min: AtomicU32::new(u32::MAX),
            max: AtomicU32::new(0),
            sum: AtomicU64::new(0),
            count: AtomicU64::new(0),
        }
    }

    /// 记录一个样本(周期数)。
    #[inline]
    pub fn record(&self, cycles: u32) {
        self.sum.fetch_add(cycles as u64, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);

        let mut cur = self.min.load(Ordering::Relaxed);
        while cycles < cur {
            match self.min.compare_exchange_weak(
                cur,
                cycles,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(x) => cur = x,
            }
        }

        let mut cur = self.max.load(Ordering::Relaxed);
        while cycles > cur {
            match self.max.compare_exchange_weak(
                cur,
                cycles,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(x) => cur = x,
            }
        }
    }

    /// 返回统计快照; 无样本时返回None。
    pub fn snapshot(&self) -> Option<Stats> {
        let count = self.count.load(Ordering::Relaxed);
        if count == 0 {
            return None;
        }
        Some(Stats {
            samples: count,
            min_cycles: self.min.load(Ordering::Relaxed),
            max_cycles: self.max.load(Ordering::Relaxed),
            avg_cycles: self.sum.load(Ordering::Relaxed) / count,
        })
    }

    /// 清零统计。
    pub fn reset(&self) {
        self.min.store(u32::MAX, Ordering::Relaxed);
        self.max.store(0, Ordering::Relaxed);
        self.sum.store(0, Ordering::Relaxed);
        self.count.store(0, Ordering::Relaxed);
    }
}

impl Default for LatencyStats {
    fn default() -> Self {
        Self::new()
    }
}

unsafe impl Sync for LatencyStats {}

/// RAII作用域计时: `begin()`读取起始周期, Drop时把区间记入统计器。
pub struct ScopedSample<'a> {
    stats: &'a LatencyStats,
    start: u32,
}

impl<'a> ScopedSample<'a> {
    #[inline(always)]
    pub fn begin(stats: &'a LatencyStats) -> Self {
        Self {
            stats,
            start: cycles(),
        }
    }

    /// 放弃本次测量(Drop时不记录)。
    #[inline(always)]
    pub fn cancel(self) {
        core::mem::forget(self);
    }
}

impl Drop for ScopedSample<'_> {
    #[inline(always)]
    fn drop(&mut self) {
        self.stats.record(cycles().wrapping_sub(self.start));
    }
}

/// 执行闭包并把耗时记入统计器, 返回闭包结果。
#[inline(always)]
pub fn measure<F, R>(stats: &LatencyStats, f: F) -> R
where
    F: FnOnce() -> R,
{
    let sample = ScopedSample::begin(stats);
    let result = f();
    core::mem::drop(sample);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_tracks_min_max_avg() {
        let s = LatencyStats::new();
        assert!(s.snapshot().is_none());
        s.record(100);
        s.record(40);
        s.record(250);
        s.record(60);
        let snap = s.snapshot().unwrap();
        assert_eq!(snap.samples, 4);
        assert_eq!(snap.min_cycles, 40);
        assert_eq!(snap.max_cycles, 250);
        assert_eq!(snap.avg_cycles, (100 + 40 + 250 + 60) / 4);
    }

    #[test]
    fn reset_clears_state() {
        let s = LatencyStats::new();
        s.record(7);
        s.reset();
        assert!(s.snapshot().is_none());
        s.record(9);
        assert_eq!(s.snapshot().unwrap().min_cycles, 9);
    }

    #[test]
    fn cycles_to_ns_exact_at_240mhz() {
        assert_eq!(cycles_to_ns(240, 240_000_000), 1000);
        assert_eq!(cycles_to_ns(0, 240_000_000), 0);
        assert_eq!(cycles_to_ns(u32::MAX, 240_000_000) > 17_000_000_000, true);
    }
}
