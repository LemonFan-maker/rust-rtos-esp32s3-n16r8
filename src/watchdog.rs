//! 看门狗与任务心跳监督
//!
//! 两层机制:
//! 1. [`Watchdog`]: 封装ESP32-S3的RWDT(复位看门狗), 超时未喂狗则复位系统;
//! 2. [`Supervisor`]: 多任务心跳登记表。喂狗任务只有在**所有**已登记任务
//!    都在各自截止期内跳动过时才喂狗, 从而把"任一关键任务卡死"转化为复位。
//!
//! Supervisor为纯原子逻辑, 时钟可注入, 可在主机侧单元测试。

use core::cell::Cell;

use portable_atomic::{AtomicU64, AtomicU8, Ordering};

const HB_FREE: u8 = 0;
const HB_ARMED: u8 = 1;
const HB_DONE: u8 = 2;

/// 单个任务的心跳槽位。
pub struct Heartbeat {
    /// 最近一次心跳时刻(ms); 0表示从未跳动
    last_beat_ms: AtomicU64,
    /// 允许的最大静默时间(ms), 仅登记时写入一次
    deadline_ms: Cell<u64>,
    /// 槽位状态: 0=空闲, 1=登记, 2=注销
    state: AtomicU8,
}

impl Heartbeat {
    const fn free() -> Self {
        Self {
            last_beat_ms: AtomicU64::new(0),
            deadline_ms: Cell::new(0),
            state: AtomicU8::new(HB_FREE),
        }
    }

    /// 记录一次心跳(指定时刻, ms)。
    #[inline]
    pub fn beat_at(&self, now_ms: u64) {
        self.last_beat_ms.store(now_ms, Ordering::Release);
    }

    /// 记录一次心跳, 时刻取embassy系统时间。
    #[inline]
    pub fn beat(&self) {
        self.beat_at(embassy_time::Instant::now().as_millis());
    }

    /// 在给定时刻判断是否仍存活。
    #[inline]
    pub fn is_alive_at(&self, now_ms: u64) -> bool {
        match self.state.load(Ordering::Acquire) {
            HB_ARMED => {
                let last = self.last_beat_ms.load(Ordering::Acquire);
                last != 0 && now_ms.saturating_sub(last) <= self.deadline_ms.get()
            }
            // 空闲与已注销槽位不阻塞喂狗
            _ => true,
        }
    }

    /// 当前是否存活(取embassy系统时间)。
    #[inline]
    pub fn is_alive(&self) -> bool {
        self.is_alive_at(embassy_time::Instant::now().as_millis())
    }

    /// 注销该槽位, 不再参与监督判定。
    #[inline]
    pub fn unregister(&self) {
        self.state.store(HB_DONE, Ordering::Release);
    }
}

// SAFETY: Heartbeat跨任务共享。state/last_beat_ms均为原子量; deadline_ms(Cell)仅在
// register()把state由FREE原子迁移到ARMED的独占窗口内写入一次, 之后只读,
// 且该写入通过state的Acquire/Release与所有读者同步。
unsafe impl Sync for Heartbeat {}

/// 静态任务心跳监督器: 固定N个槽位, 无堆分配。
pub struct Supervisor<const N: usize> {
    slots: [Heartbeat; N],
}

impl<const N: usize> Supervisor<N> {
    pub const fn new() -> Self {
        const FREE: Heartbeat = Heartbeat::free();
        Self { slots: [FREE; N] }
    }

    /// 登记一个受监督任务: `deadline_ms`内必须至少跳动一次。
    /// 返回的心跳句柄生命周期为'static, 任务侧只需周期调用`beat()`。
    /// 槽位耗尽返回None。
    pub fn register(&'static self, deadline_ms: u64) -> Option<&'static Heartbeat> {
        let now = embassy_time::Instant::now().as_millis();
        self.register_at(deadline_ms, now)
    }

    /// 指定登记时刻的[`register`](Self::register)(供测试与自定义时钟)。
    pub fn register_at(&'static self, deadline_ms: u64, now_ms: u64) -> Option<&'static Heartbeat> {
        for slot in &self.slots {
            if slot
                .state
                .compare_exchange(HB_FREE, HB_ARMED, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                slot.deadline_ms.set(deadline_ms);
                slot.last_beat_ms.store(now_ms, Ordering::Release);
                return Some(slot);
            }
        }
        None
    }

    /// 所有已登记任务均在截止期内时返回true。
    #[inline]
    pub fn all_alive(&self) -> bool {
        self.all_alive_at(embassy_time::Instant::now().as_millis())
    }

    /// 指定时刻下的存活判定(供测试与自定义时钟)。
    #[inline]
    pub fn all_alive_at(&self, now_ms: u64) -> bool {
        self.slots.iter().all(|s| s.is_alive_at(now_ms))
    }

    /// 已登记且未注销的任务数。
    pub fn armed_count(&self) -> usize {
        self.slots
            .iter()
            .filter(|s| s.state.load(Ordering::Relaxed) == HB_ARMED)
            .count()
    }
}

/// RWDT看门狗句柄。
#[cfg(target_arch = "xtensa")]
pub struct Watchdog {
    rwdt: esp_hal::rtc_cntl::Rwdt,
}

#[cfg(target_arch = "xtensa")]
impl Watchdog {
    /// 启用RWDT并设定Stage0超时(ms)。
    pub fn enable(lpwr: esp_hal::peripherals::LPWR<'static>, timeout_ms: u64) -> Self {
        let rtc = esp_hal::rtc_cntl::Rtc::new(lpwr);
        let mut rwdt = rtc.rwdt;
        rwdt.set_timeout(
            esp_hal::rtc_cntl::RwdtStage::Stage0,
            esp_hal::time::Duration::from_millis(timeout_ms),
        );
        rwdt.enable();
        Self { rwdt }
    }

    /// 喂狗。
    #[inline]
    pub fn feed(&mut self) {
        self.rwdt.feed();
    }

    /// 关闭看门狗。
    #[inline]
    pub fn disable(&mut self) {
        self.rwdt.disable();
    }
}

/// 监督喂狗一步: 全部受监督任务存活才喂狗。
/// 返回true表示已喂狗; 返回false表示存在卡死任务, 本次未喂狗,
/// 若持续false直至RWDT超时则系统复位。
#[cfg(target_arch = "xtensa")]
pub fn supervised_feed<const N: usize>(
    wdt: &mut Watchdog,
    supervisor: &Supervisor<N>,
) -> bool {
    if supervisor.all_alive() {
        wdt.feed();
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static S_FRESH: Supervisor<4> = Supervisor::new();
    static S_ARMED: Supervisor<4> = Supervisor::new();
    static S_UNREG: Supervisor<4> = Supervisor::new();
    static S_FULL: Supervisor<2> = Supervisor::new();

    #[test]
    fn fresh_supervisor_is_alive() {
        assert!(S_FRESH.all_alive_at(10_000));
        assert_eq!(S_FRESH.armed_count(), 0);
    }

    #[test]
    fn armed_task_blocks_feed_after_deadline() {
        let hb = S_ARMED.register_at(100, 1_000).unwrap();
        assert_eq!(S_ARMED.armed_count(), 1);
        assert!(S_ARMED.all_alive_at(1_050));
        assert!(S_ARMED.all_alive_at(1_100));
        assert!(!S_ARMED.all_alive_at(1_101));
        hb.beat_at(1_150);
        assert!(S_ARMED.all_alive_at(1_250));
        assert!(!S_ARMED.all_alive_at(1_251));
    }

    #[test]
    fn unregistered_slot_stops_blocking() {
        let hb = S_UNREG.register_at(50, 0).unwrap();
        assert!(!S_UNREG.all_alive_at(100));
        hb.unregister();
        assert!(S_UNREG.all_alive_at(100));
        assert_eq!(S_UNREG.armed_count(), 0);
    }

    #[test]
    fn slots_exhaust_returns_none() {
        assert!(S_FULL.register_at(10, 0).is_some());
        assert!(S_FULL.register_at(10, 0).is_some());
        assert!(S_FULL.register_at(10, 0).is_none());
    }
}
