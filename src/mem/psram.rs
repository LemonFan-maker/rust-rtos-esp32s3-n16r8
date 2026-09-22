use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ops::{Deref, DerefMut};
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheMode {
    Auto,
    Cached,
    Direct,
}

impl Default for CacheMode {
    fn default() -> Self {
        CacheMode::Auto
    }
}

#[derive(Debug, Clone)]
pub struct PsramConfig {
    pub cache_mode: CacheMode,
    pub realtime: bool,
    pub alignment: usize,
}

impl Default for PsramConfig {
    fn default() -> Self {
        Self {
            cache_mode: CacheMode::Auto,
            realtime: false,
            alignment: 32,
        }
    }
}

impl PsramConfig {
    pub fn realtime() -> Self {
        Self {
            cache_mode: CacheMode::Cached,
            realtime: true,
            alignment: 32,
        }
    }

    pub fn bulk_transfer() -> Self {
        Self {
            cache_mode: CacheMode::Direct,
            realtime: false,
            alignment: 32,
        }
    }

    pub fn with_cache_mode(mut self, mode: CacheMode) -> Self {
        self.cache_mode = mode;
        self
    }

    pub fn with_alignment(mut self, align: usize) -> Self {
        self.alignment = align;
        self
    }
}

static PSRAM_INITIALIZED: AtomicBool = AtomicBool::new(false);
static PSRAM_BASE: AtomicUsize = AtomicUsize::new(0);
static PSRAM_SIZE: AtomicUsize = AtomicUsize::new(0);
static PSRAM_END: AtomicUsize = AtomicUsize::new(0);
static PSRAM_FREE_HEAD: AtomicUsize = AtomicUsize::new(0);
static PSRAM_USED: AtomicUsize = AtomicUsize::new(0);

const HDR: usize = 32;
const MIN_PAYLOAD: usize = 8;
const BLOCK_MAGIC: u32 = 0x5053_424B;

#[repr(C)]
struct BlockHeader {
    magic: u32,
    is_free: u32,
    size: usize,
    next: usize,
}

const _: () = assert!(core::mem::size_of::<BlockHeader>() <= HDR);
const _: () = assert!(HDR % 8 == 0);

pub fn init() -> Result<PsramInfo, PsramError> {
    if PSRAM_INITIALIZED.load(Ordering::Acquire) {
        return Ok(PsramInfo {
            base: PSRAM_BASE.load(Ordering::Relaxed),
            size: PSRAM_SIZE.load(Ordering::Relaxed),
        });
    }

    let token = unsafe { esp_hal::peripherals::PSRAM::steal() };
    let (ptr, len) = esp_hal::psram::psram_raw_parts(&token);
    drop(token);

    let raw_base = ptr as usize;
    if raw_base == 0 || len < HDR * 4 {
        return Err(PsramError::NotInitialized);
    }

    let start = (raw_base + HDR - 1) & !(HDR - 1);
    let end = (raw_base + len) & !(HDR - 1);
    if end.saturating_sub(start) < HDR * 2 {
        return Err(PsramError::OutOfMemory);
    }

    unsafe {
        let h = &mut *(start as *mut BlockHeader);
        h.magic = BLOCK_MAGIC;
        h.is_free = 1;
        h.size = end - start;
        h.next = 0;
    }
    PSRAM_FREE_HEAD.store(start, Ordering::Relaxed);
    PSRAM_BASE.store(raw_base, Ordering::Relaxed);
    PSRAM_END.store(end, Ordering::Relaxed);
    PSRAM_SIZE.store(end - start, Ordering::Relaxed);
    PSRAM_USED.store(0, Ordering::Relaxed);
    PSRAM_INITIALIZED.store(true, Ordering::Release);

    Ok(PsramInfo {
        base: raw_base,
        size: end - start,
    })
}

#[derive(Debug, Clone, Copy)]
pub struct PsramInfo {
    pub base: usize,
    pub size: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PsramError {
    NotInitialized,
    OutOfMemory,
    AlignmentError,
    ZeroSize,
}

#[inline]
fn align_up(v: usize, align: usize) -> usize {
    (v + align - 1) & !(align - 1)
}

fn psram_alloc_raw(size: usize, align: usize) -> Result<*mut u8, PsramError> {
    if size == 0 {
        return Err(PsramError::ZeroSize);
    }
    if align == 0 || align & (align - 1) != 0 {
        return Err(PsramError::AlignmentError);
    }
    if !PSRAM_INITIALIZED.load(Ordering::Acquire) {
        return Err(PsramError::NotInitialized);
    }
    if size > PSRAM_SIZE.load(Ordering::Relaxed) {
        return Err(PsramError::OutOfMemory);
    }

    critical_section::with(|_| {
        let mut prev = 0usize;
        let mut cur = PSRAM_FREE_HEAD.load(Ordering::Relaxed);

        while cur != 0 {
            let fs = cur;
            let fsize = unsafe { (*(fs as *const BlockHeader)).size };
            let nnext = unsafe { (*(fs as *const BlockHeader)).next };
            debug_assert_eq!(unsafe { (*(fs as *const BlockHeader)).magic }, BLOCK_MAGIC);
            debug_assert_eq!(unsafe { (*(fs as *const BlockHeader)).is_free }, 1);
            debug_assert!(fs % HDR == 0 && fsize % HDR == 0);

            let (pa, q) = if align <= HDR {
                (fs, fs + HDR)
            } else {
                let q = align_up(fs + HDR, align);
                (q - HDR, q)
            };

            let payload_end = match q.checked_add(size) {
                Some(pe) if pe <= fs + fsize => pe,
                _ => {
                    prev = fs;
                    cur = nnext;
                    continue;
                }
            };

            let right_start = align_up(payload_end, HDR);
            let right = (fs + fsize).saturating_sub(right_start);
            let split_right = right >= HDR + MIN_PAYLOAD;

            unsafe {
                if pa != fs {
                    let ph = &mut *(fs as *mut BlockHeader);
                    ph.magic = BLOCK_MAGIC;
                    ph.is_free = 1;
                    ph.size = pa - fs;
                    ph.next = if split_right { right_start } else { nnext };

                    let ah = &mut *(pa as *mut BlockHeader);
                    ah.magic = BLOCK_MAGIC;
                    ah.is_free = 0;
                    ah.size = if split_right {
                        right_start - pa
                    } else {
                        fs + fsize - pa
                    };
                    ah.next = 0;

                    if split_right {
                        let rh = &mut *(right_start as *mut BlockHeader);
                        rh.magic = BLOCK_MAGIC;
                        rh.is_free = 1;
                        rh.size = right;
                        rh.next = nnext;
                    }
                } else if split_right {
                    let ch = &mut *(fs as *mut BlockHeader);
                    ch.magic = BLOCK_MAGIC;
                    ch.is_free = 0;
                    ch.size = right_start - fs;
                    ch.next = 0;

                    let rh = &mut *(right_start as *mut BlockHeader);
                    rh.magic = BLOCK_MAGIC;
                    rh.is_free = 1;
                    rh.size = right;
                    rh.next = nnext;

                    if prev == 0 {
                        PSRAM_FREE_HEAD.store(right_start, Ordering::Relaxed);
                    } else {
                        (&mut *(prev as *mut BlockHeader)).next = right_start;
                    }
                } else {
                    let ch = &mut *(fs as *mut BlockHeader);
                    ch.magic = BLOCK_MAGIC;
                    ch.is_free = 0;
                    ch.size = fsize;
                    ch.next = 0;

                    if prev == 0 {
                        PSRAM_FREE_HEAD.store(nnext, Ordering::Relaxed);
                    } else {
                        (&mut *(prev as *mut BlockHeader)).next = nnext;
                    }
                }
            }

            PSRAM_USED.fetch_add(size, Ordering::Relaxed);
            return Ok(q as *mut u8);
        }

        Err(PsramError::OutOfMemory)
    })
}

fn psram_free_raw(payload: *mut u8, size: usize) {
    critical_section::with(|_| {
        let bs = payload as usize - HDR;
        unsafe {
            let h = &mut *(bs as *mut BlockHeader);
            debug_assert_eq!(h.magic, BLOCK_MAGIC);
            debug_assert_eq!(h.is_free, 0);
            debug_assert!(bs % HDR == 0 && h.size % HDR == 0);
            debug_assert!(PSRAM_USED.load(Ordering::Relaxed) >= size);
            let be = bs + h.size;
            h.is_free = 1;
            h.next = 0;

            let mut prev = 0usize;
            let mut next = PSRAM_FREE_HEAD.load(Ordering::Relaxed);
            while next != 0 && next < bs {
                prev = next;
                next = (*(next as *const BlockHeader)).next;
            }

            if next != 0 && be == next {
                let nh = &*(next as *const BlockHeader);
                debug_assert_eq!(nh.magic, BLOCK_MAGIC);
                h.size = be + nh.size - bs;
                next = nh.next;
            }

            if prev != 0 && (*(prev as *const BlockHeader)).size + prev == bs {
                let ph = &mut *(prev as *mut BlockHeader);
                debug_assert_eq!(ph.magic, BLOCK_MAGIC);
                ph.size = bs + h.size - prev;
                ph.next = next;
            } else {
                h.next = next;
                if prev == 0 {
                    PSRAM_FREE_HEAD.store(bs, Ordering::Relaxed);
                } else {
                    (&mut *(prev as *mut BlockHeader)).next = bs;
                }
            }
        }
        PSRAM_USED.fetch_sub(size, Ordering::Relaxed);
    })
}

pub struct PsramBox<T> {
    ptr: NonNull<T>,
    config: PsramConfig,
    _marker: PhantomData<T>,
}

impl<T> PsramBox<T> {
    pub fn new(value: T) -> Result<Self, PsramError> {
        Self::new_with_config(value, PsramConfig::default())
    }

    pub fn new_with_config(value: T, config: PsramConfig) -> Result<Self, PsramError> {
        let size = core::mem::size_of::<T>();
        let align = config.alignment.max(core::mem::align_of::<T>());

        let ptr = psram_alloc_raw(size, align)?;
        let typed_ptr = ptr as *mut T;

        unsafe {
            typed_ptr.write(value);
        }

        Ok(Self {
            ptr: unsafe { NonNull::new_unchecked(typed_ptr) },
            config,
            _marker: PhantomData,
        })
    }

    pub fn new_uninit() -> Result<PsramBox<MaybeUninit<T>>, PsramError> {
        Self::new_uninit_with_config(PsramConfig::default())
    }

    pub fn new_uninit_with_config(
        config: PsramConfig,
    ) -> Result<PsramBox<MaybeUninit<T>>, PsramError> {
        let size = core::mem::size_of::<T>();
        let align = config.alignment.max(core::mem::align_of::<T>());

        let ptr = psram_alloc_raw(size, align)?;
        let typed_ptr = ptr as *mut MaybeUninit<T>;

        Ok(PsramBox {
            ptr: unsafe { NonNull::new_unchecked(typed_ptr) },
            config,
            _marker: PhantomData,
        })
    }

    pub fn as_ptr(&self) -> *const T {
        self.ptr.as_ptr()
    }

    pub fn as_mut_ptr(&mut self) -> *mut T {
        self.ptr.as_ptr()
    }

    pub fn config(&self) -> &PsramConfig {
        &self.config
    }

    pub fn is_in_psram(&self) -> bool {
        let addr = self.ptr.as_ptr() as usize;
        let base = PSRAM_BASE.load(Ordering::Relaxed);
        let end = PSRAM_END.load(Ordering::Relaxed);
        end != 0 && addr >= base && addr < end
    }
}

impl<T> PsramBox<MaybeUninit<T>> {
    pub unsafe fn assume_init(self) -> PsramBox<T> {
        let ptr = self.ptr.as_ptr() as *mut T;
        let config = self.config.clone();
        core::mem::forget(self);

        PsramBox {
            ptr: NonNull::new_unchecked(ptr),
            config,
            _marker: PhantomData,
        }
    }
}

impl<T> Deref for PsramBox<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { self.ptr.as_ref() }
    }
}

impl<T> DerefMut for PsramBox<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { self.ptr.as_mut() }
    }
}

impl<T> Drop for PsramBox<T> {
    fn drop(&mut self) {
        let size = core::mem::size_of::<T>();
        unsafe {
            core::ptr::drop_in_place(self.ptr.as_ptr());
            psram_free_raw(self.ptr.as_ptr() as *mut u8, size);
        }
    }
}

unsafe impl<T: Send> Send for PsramBox<T> {}
unsafe impl<T: Sync> Sync for PsramBox<T> {}

pub fn alloc_array<T: Default + Clone, const N: usize>() -> Result<PsramBox<[T; N]>, PsramError> {
    alloc_array_with_config(PsramConfig::default())
}

pub fn alloc_array_with_config<T: Default + Clone, const N: usize>(
    config: PsramConfig,
) -> Result<PsramBox<[T; N]>, PsramError> {
    let size = core::mem::size_of::<[T; N]>();
    let align = config.alignment.max(core::mem::align_of::<T>());

    let ptr = psram_alloc_raw(size, align)?;
    let typed_ptr = ptr as *mut [T; N];

    unsafe {
        for i in 0..N {
            (*typed_ptr)[i] = T::default();
        }
    }

    Ok(PsramBox {
        ptr: unsafe { NonNull::new_unchecked(typed_ptr) },
        config,
        _marker: PhantomData,
    })
}

pub fn stats() -> PsramStats {
    if !PSRAM_INITIALIZED.load(Ordering::Acquire) {
        return PsramStats {
            total: 0,
            used: 0,
            free: 0,
        };
    }
    critical_section::with(|_| {
        let total = PSRAM_SIZE.load(Ordering::Relaxed);
        let used = PSRAM_USED.load(Ordering::Relaxed);
        PsramStats {
            total,
            used,
            free: total.saturating_sub(used),
        }
    })
}

#[derive(Debug, Clone, Copy)]
pub struct PsramStats {
    pub total: usize,
    pub used: usize,
    pub free: usize,
}

pub mod cache {
    use core::arch::asm;

    #[inline]
    pub unsafe fn flush(addr: *const u8, size: usize) {
        let mut current = addr as usize;
        let end = current + size;

        while current < end {
            #[cfg(target_arch = "xtensa")]
            asm!(
                "dhwbi {0}, 0",
                in(reg) current,
                options(nostack, preserves_flags)
            );

            current += 32;
        }

        #[cfg(target_arch = "xtensa")]
        asm!("memw", options(nostack, preserves_flags));
    }

    #[inline]
    pub unsafe fn invalidate(addr: *const u8, size: usize) {
        let mut current = addr as usize;
        let end = current + size;

        while current < end {
            #[cfg(target_arch = "xtensa")]
            asm!(
                "dhi {0}, 0",
                in(reg) current,
                options(nostack, preserves_flags)
            );

            current += 32;
        }

        #[cfg(target_arch = "xtensa")]
        asm!("memw", options(nostack, preserves_flags));
    }

    #[inline]
    pub unsafe fn flush_and_invalidate(addr: *const u8, size: usize) {
        flush(addr, size);
        invalidate(addr, size);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_mode_default() {
        assert_eq!(CacheMode::default(), CacheMode::Auto);
    }

    #[test]
    fn test_psram_config_default() {
        let config = PsramConfig::default();
        assert_eq!(config.cache_mode, CacheMode::Auto);
        assert!(!config.realtime);
        assert_eq!(config.alignment, 32);
    }

    #[test]
    fn test_header_geometry() {
        assert_eq!(HDR % 8, 0);
        assert!(core::mem::size_of::<BlockHeader>() <= HDR);
    }

    #[test]
    fn test_align_up() {
        assert_eq!(align_up(0, 8), 0);
        assert_eq!(align_up(1, 8), 8);
        assert_eq!(align_up(16, 16), 16);
        assert_eq!(align_up(33, 32), 64);
    }
}
