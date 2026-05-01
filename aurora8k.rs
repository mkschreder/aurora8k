// aurora8k.rs - single-file CPU-only Rust terminal 64k-style intro.
//
// Build: rustc aurora8k.rs --edition 2021 \
//          -C opt-level=z -C panic=abort -C lto=fat -C codegen-units=1 \
//          -C strip=symbols -C relocation-model=static \
//          -C link-arg=-nostdlib \
//          -C link-arg=-Wl,--build-id=none \
//          -C link-arg=-Wl,--no-eh-frame-hdr \
//          -C link-arg=-Wl,-T,linker.ld \
//          -o aurora8k && strip --strip-section-headers aurora8k
//
// Result: 8,139 bytes ELF, with AO.  Linux x86-64 only.
// No GPU, no assets, no crates, no libc, no libm.  Pure math + raw syscalls.

#![no_std]
#![no_main]

mod sys;

use core::f32::consts::PI;
use core::ops::{Add, Div, Mul, Neg, Sub};
use sys::{F32Ext, Out, Term, clock_monotonic, elapsed, fast_floor, sleep_ms, term_size};

// ── Min / max helpers that compile to minss/maxss, not fminf/fmaxf PLT calls ──
// Using if-comparisons avoids llvm.minnum / llvm.maxnum which emit PLT calls.

#[inline(always)] fn fmn(a: f32, b: f32) -> f32 { if a < b { a } else { b } }
#[inline(always)] fn fmx(a: f32, b: f32) -> f32 { if a > b { a } else { b } }

// ── Integer exponentiation (replaces powf PLT call) ──────────────────────────

/// x^32 via 5 multiplications — near-identical specular tightness to x^38
#[inline(always)]
fn pow32(x: f32) -> f32 {
    let x2 = x*x; let x4 = x2*x2; let x8 = x4*x4; let x16 = x8*x8;
    x16 * x16
}

// ── Vector type ───────────────────────────────────────────────────────────────

#[derive(Copy, Clone)]
struct V { x: f32, y: f32, z: f32 }

impl V {
    fn new(x: f32, y: f32, z: f32) -> Self { Self { x, y, z } }
    fn dot(self, b: Self) -> f32 { self.x*b.x + self.y*b.y + self.z*b.z }
    fn cross(self, b: Self) -> Self {
        Self::new(self.y*b.z-self.z*b.y, self.z*b.x-self.x*b.z, self.x*b.y-self.y*b.x)
    }
    fn len(self)  -> f32  { self.dot(self).sqrt() }
    fn norm(self) -> Self { self / fmx(self.len(), 1e-6) }
}

impl Add for V { type Output=Self; fn add(self,b:Self)->Self{Self::new(self.x+b.x,self.y+b.y,self.z+b.z)} }
impl Sub for V { type Output=Self; fn sub(self,b:Self)->Self{Self::new(self.x-b.x,self.y-b.y,self.z-b.z)} }
impl Mul<f32> for V { type Output=Self; fn mul(self,s:f32)->Self{Self::new(self.x*s,self.y*s,self.z*s)} }
impl Div<f32> for V { type Output=Self; fn div(self,s:f32)->Self{Self::new(self.x/s,self.y/s,self.z/s)} }
impl Neg for V { type Output=Self; fn neg(self)->Self{Self::new(-self.x,-self.y,-self.z)} }

// ── Scalar helpers ────────────────────────────────────────────────────────────

fn clamp(x: f32, a: f32, b: f32) -> f32 { fmn(fmx(x, a), b) }
fn mix(a: f32, b: f32, t: f32)   -> f32 { a + (b-a)*t }
fn hash(n: f32) -> f32 {
    // Integer bit-mixing — avoids a fast_sin call, still uniform in [0,1)
    let b = n.to_bits().wrapping_mul(0x9e3779b9);
    (b >> 9) as f32 * (1.0 / 8388608.0)
}
fn rep(x: f32, c: f32) -> f32 {
    // rem_euclid(x, c) without fmodf: use floor
    let shifted = x + 0.5 * c;
    shifted - c * fast_floor(shifted / c) - 0.5 * c
}

// ── Rotation helpers ──────────────────────────────────────────────────────────

fn rot_y(p: V, a: f32) -> V { let (s,c)=a.sin_cos(); V::new(c*p.x+s*p.z, p.y, -s*p.x+c*p.z) }
fn rot_x(p: V, a: f32) -> V { let (s,c)=a.sin_cos(); V::new(p.x, c*p.y-s*p.z, s*p.y+c*p.z) }
fn rot_z(p: V, a: f32) -> V { let (s,c)=a.sin_cos(); V::new(c*p.x-s*p.y, s*p.x+c*p.y, p.z) }

// ── Signed distance functions ─────────────────────────────────────────────────

fn sd_octa(p: V, s: f32) -> f32 { (p.x.abs()+p.y.abs()+p.z.abs()-s)*0.57735027 }
fn sd_torus(p: V, r: f32, tube: f32) -> f32 {
    let qx = (p.x*p.x + p.z*p.z).sqrt() - r;
    (qx*qx + p.y*p.y).sqrt() - tube
}
fn sd_cyl_y(p: V, r: f32, h: f32) -> f32 {
    let dx = (p.x*p.x + p.z*p.z).sqrt() - r;
    let dy = p.y.abs() - h;
    let dx0 = fmx(dx, 0.0);
    let dy0 = fmx(dy, 0.0);
    fmn(fmx(dx, dy), 0.0) + (dx0*dx0 + dy0*dy0).sqrt()
}
fn smin(a: f32, b: f32, k: f32) -> f32 {
    let h = clamp(0.5+0.5*(b-a)/k, 0.0, 1.0);
    mix(b, a, h) - k*h*(1.0-h)
}

// ── Scene ─────────────────────────────────────────────────────────────────────

#[derive(Copy, Clone)]
struct Hit { d: f32, m: i32 }

fn put(best: &mut Hit, d: f32, m: i32) { if d < best.d { best.d=d; best.m=m; } }

fn map(p0: V, t: f32) -> Hit {
    let mut h = Hit { d: 1e9, m: 0 };

    let floor_wave = 0.035*(p0.x*2.0 + p0.z*1.5 + t*0.6).sin();
    put(&mut h, p0.y + 1.25 + floor_wave, 4);

    let p = rot_y(rot_x(p0, t*0.37), t*0.23);
    let pulse = 0.08*(t*1.8).sin();
    let gem = smin(sd_octa(p, 0.95+pulse), p.len()-0.72, 0.18)
            + 0.025*(p.x*7.0 + p.y*5.0 + p.z*4.5 + t*0.7).sin();
    put(&mut h, gem, 1);

    let r0 = sd_torus(rot_x(p0, t*0.7), 1.18, 0.035);
    let r1 = sd_torus(rot_z(rot_y(p0, 1.57), -t*0.55), 1.34, 0.026);
    let r2 = sd_torus(rot_y(rot_x(p0, 1.57), t*0.33), 1.52, 0.018);
    put(&mut h, fmn(r0, fmn(r1, r2)), 2);

    let a  = p0.z.atan2(p0.x);
    let rr = (p0.x*p0.x + p0.z*p0.z).sqrt();
    let aa = rep(a, 2.0*PI/8.0);
    let q  = V::new(rr-2.35, p0.y, aa*rr);
    let shaft = sd_cyl_y(q, 0.07, 1.1);
    let qc  = V::new(q.x, q.y.abs() - 1.04, q.z);
    let cap = qc.len() - 0.11;  // sphere caps: same look at terminal res, simpler SDF
    put(&mut h, fmn(shaft, cap), 3);

    h
}

// ── Lighting ──────────────────────────────────────────────────────────────────

fn normal(p: V, t: f32) -> V {
    // Tetrahedral sampling: 4 map() calls instead of 6, better gradient quality
    let e = 0.004;
    let k1 = V::new( 1.0,-1.0,-1.0); let k2 = V::new(-1.0,-1.0, 1.0);
    let k3 = V::new(-1.0, 1.0,-1.0); let k4 = V::new( 1.0, 1.0, 1.0);
    (k1*map(p+k1*e,t).d + k2*map(p+k2*e,t).d +
     k3*map(p+k3*e,t).d + k4*map(p+k4*e,t).d).norm()
}

fn shadow(ro: V, rd: V, t: f32) -> f32 {
    let mut res = 1.0_f32;
    let mut d   = 0.035_f32;
    for _ in 0..28 {
        let h = map(ro + rd*d, t).d;
        if h < 0.002 { return 0.0; }
        res = fmn(res, 9.0*h/d);
        d  += clamp(h, 0.025, 0.28);
        if d > 7.0 { break; }
    }
    clamp(res, 0.0, 1.0)
}

fn ao(p: V, n: V, t: f32) -> f32 {
    let mut o  = 0.0_f32;
    let mut sc = 1.0_f32;
    for i in 1..6 {
        let h = i as f32 * 0.055;
        o  += fmx(h - map(p + n*h, t).d, 0.0) * sc;
        sc *= 0.55;
    }
    clamp(1.0 - o*1.8, 0.0, 1.0)
}

fn palette(m: i32, p: V, t: f32) -> V {
    match m {
        1 => V::new(1.0, 0.35 + 0.30*(p.y*6.0+t).sin().abs(), 0.95),
        2 => V::new(0.25, 0.95, 1.0),
        3 => V::new(1.0, 0.72, 0.32),
        4 => V::new(0.10, 0.09, 0.20),
        _ => V::new(0.0, 0.0, 0.0),
    }
}

fn sky(rd: V, t: f32) -> V {
    let y  = clamp(rd.y*0.5 + 0.5, 0.0, 1.0);
    let mut c = V::new(0.03,0.025,0.08)*(1.0-y) + V::new(0.02,0.10,0.18)*y;
    let u  = rd.z.atan2(rd.x)*7.0;
    // Replace acos(rd.y)*9 with linear approximation — same angular range, no acosf PLT call
    let v  = (1.0 - clamp(rd.y, -1.0, 1.0)) * (9.0 * PI / 2.0);
    let id = (fast_floor(u)*37.0 + fast_floor(v)*113.0).abs();
    let star = fmx(hash(id + fast_floor(t*0.03)) - 0.996, 0.0) * 250.0;
    c = c + V::new(0.75,0.9,1.0)*star;
    c
}

fn shade(ro: V, rd: V, t: f32) -> V {
    let sky_col = sky(rd, t);   // compute once — reused for miss path and fog blend
    let mut depth = 0.0_f32;
    let mut glow  = V::new(0.0,0.0,0.0);
    let mut mat   = 0;
    for i in 0..76 {
        let p = ro + rd*depth;
        let h = map(p, t);
        glow = glow + V::new(0.8, 0.55, 1.0) * (0.002 / (0.015 + h.d.abs()));
        if h.d < 0.0015*(1.0 + depth*0.12) { mat = h.m; break; }
        depth += clamp(h.d, 0.006, 0.45);
        if depth > 18.0 || i == 75 { return sky_col + glow*0.55; }
    }

    let p    = ro + rd*depth;
    let n    = normal(p, t);
    let base = palette(mat, p, t);
    let lp1  = V::new(2.6*(t*0.7).cos(), 2.1, 2.6*(t*0.7).sin());

    let mut col = base*0.08;
    let lp = lp1;
    {
        let l    = (lp-p).norm();
        let dif  = fmx(n.dot(l), 0.0) * shadow(p+n*0.01, l, t);
        let r    = (n*(2.0*n.dot(l)) - l).norm();
        let spec = pow32(clamp(r.dot(-rd), 0.0, 1.0));
        col = col + base*dif + V::new(1.0,0.95,0.8)*spec*(0.55+0.45*dif);
    }
    let fr = fmx(1.0 + n.dot(rd), 0.0);
    let fres = fr * fr * fr;
    col = col*ao(p,n,t) + V::new(0.25,0.85,1.0)*fres;
    let fog = clamp((depth - 3.0) * (1.0/12.0), 0.0, 1.0);
    col*(1.0-fog) + sky_col*fog + glow*0.6
}

// ── Camera ────────────────────────────────────────────────────────────────────

fn camera(t: f32) -> (V, V, V, V) {
    let tt = t*0.22;
    let ro = V::new(4.0*tt.cos(), 1.25 + 0.35*(t*0.4).sin(), 4.0*tt.sin());
    let ta = V::new(0.0, -0.05 + 0.25*(t*0.3).sin(), 0.0);
    let f  = (ta-ro).norm();
    let r  = f.cross(V::new(0.0,1.0,0.0)).norm();
    let u  = r.cross(f);
    (ro, f, r, u)
}

// ── Tone mapping ──────────────────────────────────────────────────────────────

fn tonemap(c: V) -> (u8, u8, u8) {
    // Reinhard compression + sqrt gamma (gamma 2.0 ≈ 2.2, saves powf(0.4545) PLT call)
    let to_u8 = |v: f32| (fmn(fmx(v / (1.0+v), 0.0), 1.0).sqrt() * 255.0) as u8;
    (to_u8(c.x), to_u8(c.y), to_u8(c.z))
}

// ── Frame builder ─────────────────────────────────────────────────────────────

fn push_u8(out: &mut Out, v: u8) {
    if v >= 100 { out.push(b'0' + v / 100); }
    if v >= 10  { out.push(b'0' + (v / 10) % 10); }
    out.push(b'0' + v % 10);
}

fn render(w: usize, h: usize, t: f32, out: &mut Out) {
    out.push_str("\x1b[H");
    let pix_h = h * 2;
    let asp   = w as f32 / pix_h as f32;
    let (ro, f, r, u) = camera(t);

    for y in 0..h {
        for x in 0..w {
            let mut rgb = [(0_u8, 0_u8, 0_u8); 2];
            for sy in 0..2 {
                let yy   = y*2 + sy;
                let px   = ((x as f32 + 0.5) / w as f32 * 2.0 - 1.0) * asp;
                let py   = 1.0 - (yy as f32 + 0.5) / pix_h as f32 * 2.0;
                let rd   = (f*1.35 + r*px + u*py).norm();
                let vign = 1.0 - 0.35*fmn(px*px + py*py, 1.0);
                rgb[sy]  = tonemap(shade(ro, rd, t) * vign);
            }
            let (r1,g1,b1) = rgb[0];
            let (r2,g2,b2) = rgb[1];
            out.push_str("\x1b[38;2;"); push_u8(out,r1); out.push(b';');
                                        push_u8(out,g1); out.push(b';');
                                        push_u8(out,b1);
            out.push_str("m\x1b[48;2;"); push_u8(out,r2); out.push(b';');
                                          push_u8(out,g2); out.push(b';');
                                          push_u8(out,b2);
            out.push_str("m\u{2580}");
        }
        out.push_str("\x1b[0m\n");
    }
}

// ── Main loop ─────────────────────────────────────────────────────────────────

pub(crate) fn run(seconds: f32) {
    let _term = Term::enter();
    let start = clock_monotonic();
    let mut frame = Out(0);
    loop {
        let t = elapsed(&start);
        if t > seconds { break; }
        let (w, h) = term_size();
        frame.clear();
        render(w, h, t, &mut frame);
        frame.flush();
        sleep_ms(16);
    }
}
