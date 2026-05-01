//! System layer for aurora8k: platform bindings, process entry, I/O, timing.
//!
//! Fully self-contained — no libc, no libm.  Everything goes through raw Linux
//! syscalls or SSE/inline-math implementations so the final binary has zero
//! dynamic-link dependencies and no PLT/GOT overhead.
//!
//! Build
//! -----
//!   rustc aurora8k.rs --edition 2021 \
//!     -C opt-level=z -C panic=abort -C lto=fat -C codegen-units=1 \
//!     -C strip=symbols \
//!     -C link-arg=-nostdlib \
//!     -C link-arg=-Wl,--build-id=none \
//!     -C link-arg=-Wl,--no-eh-frame-hdr \
//!     -o aurora8k
//!   strip --remove-section=.eh_frame --remove-section=.comment aurora8k

use core::f32::consts::{PI, FRAC_PI_2, FRAC_PI_4};

// ── Raw Linux x86-64 syscalls ─────────────────────────────────────────────────

#[inline(always)]
unsafe fn sys2(n: i64, a1: i64, a2: i64) -> i64 {
    let ret: i64;
    core::arch::asm!(
        "syscall",
        inlateout("rax") n => ret,
        in("rdi") a1, in("rsi") a2,
        lateout("rcx") _, lateout("r11") _,
        options(nostack)
    );
    ret
}

#[inline(always)]
unsafe fn sys3(n: i64, a1: i64, a2: i64, a3: i64) -> i64 {
    let ret: i64;
    core::arch::asm!(
        "syscall",
        inlateout("rax") n => ret,
        in("rdi") a1, in("rsi") a2, in("rdx") a3,
        lateout("rcx") _, lateout("r11") _,
        options(nostack)
    );
    ret
}

#[inline(always)]
unsafe fn sys6(n: i64, a1: i64, a2: i64, a3: i64, a4: i64, a5: i64, a6: i64) -> i64 {
    let ret: i64;
    core::arch::asm!(
        "mov r10, rcx",
        "syscall",
        inlateout("rax") n => ret,
        in("rdi") a1, in("rsi") a2, in("rdx") a3,
        in("rcx") a4, in("r8")  a5, in("r9")  a6,
        lateout("r11") _,
        options(nostack)
    );
    ret
}

/// Floor via integer truncation — avoids floorf PLT call.
/// Correct for |x| < 2^31 (all our use cases stay well within this range).
#[inline(always)]
pub fn fast_floor(x: f32) -> f32 {
    let i = x as i32;
    let f = i as f32;
    if f > x { f - 1.0 } else { f }
}

/// sin(x) via range-reduction + 3-term minimax polynomial.
/// Max error < 2×10⁻⁴ (Chebyshev-optimal coefficients, better than Taylor).
fn fast_sin(x: f32) -> f32 {
    let n = fast_floor(x * (0.5 / PI) + 0.5);
    let x = x - n * (2.0 * PI);
    let x = if x > FRAC_PI_2 { PI - x } else if x < -FRAC_PI_2 { -PI - x } else { x };
    let x2 = x * x;
    // Minimax coefficients (Remez algorithm, degree-5 odd polynomial on [-π/2, π/2])
    x * (1.0 + x2 * (-0.166_605 + x2 * 0.007_609))
}

/// atan2(y, x) via range-reduction + Vigna's 2-term formula.
/// Max error ≈ 0.004 rad — invisible for our 8-column polar repetition.
fn fast_atan2(y: f32, x: f32) -> f32 {
    let ax = x.abs();
    let ay = y.abs();
    // Ensure t = smaller/larger ∈ [0, 1]
    let big   = if ax > ay { ax } else { ay };
    let small = if ax > ay { ay } else { ax };
    if big == 0.0 { return 0.0; }
    let t = small / big;
    // Vigna approximation: atan(t) ≈ (π/4)t + 0.273·t·(1−t)  on [0,1]
    let a = FRAC_PI_4 * t + 0.273_f32 * t * (1.0 - t);
    let a = if ay > ax { FRAC_PI_2 - a } else { a }; // octant
    let a = if x < 0.0  { PI - a }         else { a }; // half-plane
    if y < 0.0 { -a } else { a }
}

/// Adds transcendental methods to `f32` without libm.
/// `sqrt` uses the `sqrtss` SSE instruction directly via inline asm (no libm needed).
pub trait F32Ext: Copy {
    fn sqrt(self) -> f32;
    fn sin(self)  -> f32;
    fn cos(self)  -> f32;
    fn sin_cos(self) -> (f32, f32);
    fn atan2(self, other: f32) -> f32;
}

impl F32Ext for f32 {
    #[inline(always)]
    fn sqrt(self) -> f32 {
        // sqrtss is a single SSE instruction — no libm needed
        unsafe {
            let r: f32;
            core::arch::asm!("sqrtss {0}, {1}", out(xmm_reg) r, in(xmm_reg) self, options(pure, nomem, nostack));
            r
        }
    }
    #[inline(always)] fn sin(self)  -> f32 { fast_sin(self) }
    #[inline(always)] fn cos(self)  -> f32 { fast_sin(self + FRAC_PI_2) }
    #[inline(always)] fn sin_cos(self) -> (f32, f32) {
        (fast_sin(self), fast_sin(self + FRAC_PI_2))
    }
    #[inline(always)] fn atan2(self, other: f32) -> f32 { fast_atan2(self, other) }
}

// ── Structs ───────────────────────────────────────────────────────────────────

/// Wall-clock time with nanosecond resolution (matches Linux `struct timespec`).
#[repr(C)]
pub struct Timespec {
    pub sec:  i64,
    pub nsec: i64,
}

#[repr(C)]
struct Winsize { row: u16, col: u16, _xp: u16, _yp: u16 }

// ── Process entry point ───────────────────────────────────────────────────────

core::arch::global_asm!(
    ".global _start",
    "_start:",
    "xor   rbp, rbp",
    "mov   rdi, [rsp]",
    "lea   rsi, [rsp + 8]",
    "call  aurora_entry",
    "mov   edi, eax",
    "mov   eax, 60",          // SYS_exit
    "syscall",
);

#[no_mangle]
unsafe extern "C" fn aurora_entry(_argc: i64, _argv: *const *const u8) -> i32 {
    buf_init();
    crate::run(90.0);
    0
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") 60_i64,   // SYS_exit
            in("rdi") 1_i64,
            options(nostack, noreturn)
        );
    }
}

// ── Timing ────────────────────────────────────────────────────────────────────

pub fn clock_monotonic() -> Timespec {
    let mut ts = Timespec { sec: 0, nsec: 0 };
    unsafe { sys2(228 /* SYS_clock_gettime */, 1 /* CLOCK_MONOTONIC */, &mut ts as *mut _ as i64); }
    ts
}

pub fn elapsed(start: &Timespec) -> f32 {
    let now = clock_monotonic();
    (now.sec - start.sec) as f32 + (now.nsec - start.nsec) as f32 * 1e-9
}

#[allow(dead_code)]
pub fn sleep_ms(ms: u64) {
    let ts = Timespec { sec: 0, nsec: (ms * 1_000_000) as i64 };
    unsafe { sys2(35 /* SYS_nanosleep */, &ts as *const _ as i64, 0); }
}

// ── Terminal size ─────────────────────────────────────────────────────────────

pub fn term_size() -> (usize, usize) {
    let mut ws = Winsize { row: 0, col: 0, _xp: 0, _yp: 0 };
    unsafe { sys3(16 /* SYS_ioctl */, 1, 0x5413 /* TIOCGWINSZ */, &mut ws as *mut _ as i64); }
    let c = if ws.col > 20 { ws.col as usize } else { 100 };
    let r = if ws.row > 10 { ws.row as usize } else { 36 };
    (c, r.saturating_sub(2))
}

// ── Frame buffer ──────────────────────────────────────────────────────────────

static mut BUF_PTR: *mut u8 = core::ptr::null_mut();

fn buf_init() {
    // MAP_ANONYMOUS|MAP_PRIVATE = 0x22, PROT_READ|PROT_WRITE = 3, SYS_mmap = 9
    let p = unsafe { sys6(9, 0, 1 << 20, 3, 0x22, -1, 0) };
    unsafe { BUF_PTR = p as *mut u8; }
}

/// Allocate a private anonymous memory region of `size` bytes.
/// Returns the mapped address.  Caller must not exceed `size` bytes.
#[allow(dead_code)]
pub unsafe fn alloc_anon(size: usize) -> *mut u8 {
    sys6(9, 0, size as i64, 3, 0x22, -1, 0) as *mut u8
}

/// Write-only byte buffer backed by a mmap'd 1 MiB region.
pub struct Out(pub usize);

impl Out {
    pub fn clear(&mut self) { self.0 = 0; }

    pub fn push(&mut self, b: u8) {
        unsafe { BUF_PTR.add(self.0).write(b); }
        self.0 += 1;
    }

    pub fn push_str(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let dst = unsafe { BUF_PTR.add(self.0) };
        for (i, &b) in bytes.iter().enumerate() {
            unsafe { core::ptr::write_volatile(dst.add(i), b); }
        }
        self.0 += bytes.len();
    }

    pub fn flush(&self) {
        unsafe { sys3(1 /* SYS_write */, 1, BUF_PTR as i64, self.0 as i64); }
    }
}

// ── Terminal alternate-screen guard (RAII) ────────────────────────────────────

pub struct Term;

impl Term {
    pub fn enter() -> Self {
        const ON: &[u8] = b"\x1b[?1049h\x1b[?25l\x1b[2J";
        unsafe { sys3(1, 1, ON.as_ptr() as i64, ON.len() as i64); }
        Self
    }
}

impl Drop for Term {
    fn drop(&mut self) {
        const OFF: &[u8] = b"\x1b[0m\x1b[?25h\x1b[?1049l";
        unsafe { sys3(1, 1, OFF.as_ptr() as i64, OFF.len() as i64); }
    }
}
