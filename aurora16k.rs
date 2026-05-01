// aurora16k.rs — no_std CPU SDF intro targeting ≤16 KB UPX-compressed.
// Crystal temple in a forest clearing: rolling terrain, conifer forest,
// aurora/mountain sky, reflective water pool, energy beam, fireflies,
// camera choreography, material-specific shading, 4 rings, crystal shards,
// gateway arches, standing stones, rune ring, moon shaft, sky clouds.
//
// Build:   make aurora16k          (UPX packed, ~16 KB)
//          make aurora16k_standard  (uncompressed for measurement)
// Run:     ./aurora16k
//
// Linux x86-64 only.  No GPU, no libc, no crates.  Raw syscalls via sys.rs.

#![no_std]
#![no_main]

mod sys;

use core::f32::consts::PI;
use core::ops::{Add, Div, Mul, Neg, Sub};
use sys::{F32Ext, Out, Term, alloc_anon, clock_monotonic, elapsed, fast_floor,
          term_size};

// Pixel framebuffer: RGB bytes written by render_to_fb, read by fb_to_sixel.
static mut FB_PTR: *mut u8 = core::ptr::null_mut();

// ── Min / max / pow ───────────────────────────────────────────────────────────

#[inline(always)] fn fmn(a:f32,b:f32)->f32 { if a<b{a}else{b} }
#[inline(always)] fn fmx(a:f32,b:f32)->f32 { if a>b{a}else{b} }

/// Evaluate sin(x) for four inputs simultaneously using SSE2 (4-wide throughput).
/// Pass 0.0 for unused lanes.  Falls back to scalar on non-x86_64 targets.
#[inline(always)]
fn fast_sin_4(a: f32, b: f32, c: f32, d: f32) -> (f32, f32, f32, f32) {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        use core::arch::x86_64::*;
        let x      = _mm_set_ps(d, c, b, a);
        let inv2pi = _mm_set1_ps(0.159_154_94_f32);
        let twopi  = _mm_set1_ps(6.283_185_3_f32);
        let half   = _mm_set1_ps(0.5_f32);
        let one    = _mm_set1_ps(1.0_f32);
        let v  = _mm_add_ps(_mm_mul_ps(x, inv2pi), half);
        let ti = _mm_cvttps_epi32(v);
        let tf = _mm_cvtepi32_ps(ti);
        let n  = _mm_sub_ps(tf, _mm_and_ps(_mm_cmpgt_ps(tf, v), one));
        let x      = _mm_sub_ps(x, _mm_mul_ps(n, twopi));
        let pi     = _mm_set1_ps(3.141_592_7_f32);
        let pih    = _mm_set1_ps(1.570_796_3_f32);
        let negpi  = _mm_sub_ps(_mm_setzero_ps(), pi);
        let negpih = _mm_sub_ps(_mm_setzero_ps(), pih);
        let hi = _mm_cmpgt_ps(x, pih);
        let lo = _mm_cmplt_ps(x, negpih);
        let x  = _mm_or_ps(_mm_and_ps(hi, _mm_sub_ps(pi,    x)), _mm_andnot_ps(hi, x));
        let x  = _mm_or_ps(_mm_and_ps(lo, _mm_sub_ps(negpi, x)), _mm_andnot_ps(lo, x));
        // Minimax: x*(1 + x²*(-0.166605 + x²*0.007609))
        let c1 = _mm_set1_ps(-0.166_605_f32);
        let c2 = _mm_set1_ps( 0.007_609_f32);
        let x2 = _mm_mul_ps(x, x);
        let p  = _mm_add_ps(one, _mm_mul_ps(x2, _mm_add_ps(c1, _mm_mul_ps(x2, c2))));
        let r  = _mm_mul_ps(x, p);
        let mut out = [0_f32; 4];
        _mm_storeu_ps(out.as_mut_ptr(), r);
        (out[0], out[1], out[2], out[3])
    }
    #[cfg(not(target_arch = "x86_64"))]
    { (a.sin(), b.sin(), c.sin(), d.sin()) }
}

#[inline(always)]
fn pow38(x:f32)->f32 {
    let x2=x*x; let x4=x2*x2; let x8=x4*x4; let x16=x8*x8; let x32=x16*x16;
    x32*x4*x2
}

// ── Vector ────────────────────────────────────────────────────────────────────

#[derive(Copy,Clone)] struct V { x:f32, y:f32, z:f32 }

impl V {
    fn new(x:f32,y:f32,z:f32)->Self { Self{x,y,z} }
    fn dot(self,b:Self)->f32  { self.x*b.x+self.y*b.y+self.z*b.z }
    fn cross(self,b:Self)->Self {
        Self::new(self.y*b.z-self.z*b.y, self.z*b.x-self.x*b.z, self.x*b.y-self.y*b.x)
    }
    fn len(self)->f32   { self.dot(self).sqrt() }
    fn norm(self)->Self { self/fmx(self.len(),1e-6) }
    fn abs(self)->Self  { Self::new(self.x.abs(),self.y.abs(),self.z.abs()) }
    fn vmax(self,b:Self)->Self { Self::new(fmx(self.x,b.x),fmx(self.y,b.y),fmx(self.z,b.z)) }
}

impl Add for V { type Output=Self; fn add(self,b:Self)->Self{Self::new(self.x+b.x,self.y+b.y,self.z+b.z)} }
impl Sub for V { type Output=Self; fn sub(self,b:Self)->Self{Self::new(self.x-b.x,self.y-b.y,self.z-b.z)} }
impl Mul<f32> for V { type Output=Self; fn mul(self,s:f32)->Self{Self::new(self.x*s,self.y*s,self.z*s)} }
impl Div<f32> for V { type Output=Self; fn div(self,s:f32)->Self{Self::new(self.x/s,self.y/s,self.z/s)} }
impl Neg for V { type Output=Self; fn neg(self)->Self{Self::new(-self.x,-self.y,-self.z)} }

// ── Scalar helpers ────────────────────────────────────────────────────────────

fn clamp(x:f32,a:f32,b:f32)->f32 { fmn(fmx(x,a),b) }
fn mix(a:f32,b:f32,t:f32)->f32   { a+(b-a)*t }
fn smoothstep(a:f32,b:f32,x:f32)->f32 { let t=clamp((x-a)/(b-a),0.0,1.0); t*t*(3.0-2.0*t) }

fn hash(n:f32)->f32 {
    let n=(n*127.1+311.7).sin()*43758.5453;
    n - fast_floor(n)
}
fn rep(x:f32,c:f32)->f32 {
    let s=x+0.5*c; s - c*fast_floor(s/c) - 0.5*c
}

// ── 4-wide SIMD noise: three trig-based octaves, one SSE2 call ────────────────

fn noise3(p:V)->f32 {
    let (s1,s2,s3,_)=fast_sin_4(
        p.x*1.70+p.z*2.30,
        p.x*3.10-p.y*1.90+p.z*0.70,
        p.x*7.30+p.y*2.10+p.z*1.30,
        0.0,
    );
    (s1 + s2*0.50 + s3*0.25) / 1.75
}

// ── Rotation helper (rot_z only — rot_x/rot_y are inlined via MapCtx) ─────────

fn rot_z(p:V,a:f32)->V { let (s,c)=a.sin_cos(); V::new(c*p.x-s*p.y,s*p.x+c*p.y,p.z) }

// ── SDF primitives ────────────────────────────────────────────────────────────

fn sd_box(p:V,b:V)->f32 {
    let q=p.abs()-b;
    q.vmax(V::new(0.0,0.0,0.0)).len() + fmn(fmx(q.x,fmx(q.y,q.z)),0.0)
}
fn sd_octa(p:V,s:f32)->f32 { (p.x.abs()+p.y.abs()+p.z.abs()-s)*0.57735027 }
fn sd_torus(p:V,r:f32,tube:f32)->f32 {
    let qx=(p.x*p.x+p.z*p.z).sqrt()-r;
    (qx*qx+p.y*p.y).sqrt()-tube
}
fn sd_cyl_y(p:V,r:f32,h:f32)->f32 {
    let dx=(p.x*p.x+p.z*p.z).sqrt()-r;
    let dy=p.y.abs()-h;
    let dx0=fmx(dx,0.0); let dy0=fmx(dy,0.0);
    fmn(fmx(dx,dy),0.0)+(dx0*dx0+dy0*dy0).sqrt()
}
fn smin(a:f32,b:f32,k:f32)->f32 {
    let h=clamp(0.5+0.5*(b-a)/k,0.0,1.0); mix(b,a,h)-k*h*(1.0-h)
}

// ── Terrain — SIMD-accelerated ────────────────────────────────────────────────
// Domain-warped multi-octave noise: flat clearing at centre, hills beyond.
// Inlines two noise3 warp calls + 5 terrain octaves using fast_sin_4 batching.

fn terrain_h(x:f32,z:f32)->f32 {
    let flat=smoothstep(3.0,7.5,(x*x+z*z).sqrt());
    // Compute noise3(x*0.3, 0, z*0.3) and noise3(x*0.3+4, 0, z*0.3+4) via two SIMD calls
    let p1x=x*0.3; let p1z=z*0.3;
    let p2x=p1x+4.0; let p2z=p1z+4.0;
    // Batch 1: first two octaves of warp-x and first octave of warp-z
    let (a1,a2,a3,b1)=fast_sin_4(
        p1x*1.70+p1z*2.30,
        p1x*3.10+p1z*0.70,          // p1y=0
        p1x*7.30+p1z*1.30,          // p1y=0
        p2x*1.70+p2z*2.30,
    );
    let (b2,b3,_,_)=fast_sin_4(p2x*3.10+p2z*0.70, p2x*7.30+p2z*1.30, 0.0, 0.0);
    let wx=(a1 + a2*0.50 + a3*0.25) / 1.75 * 0.6;
    let wz=(b1 + b2*0.50 + b3*0.25) / 1.75 * 0.6;
    let nx=x+wx; let nz=z+wz;
    // Batch terrain octaves 1-4 in one SIMD call, octave 5 scalar
    let (t1,t2,t3,t4)=fast_sin_4(
        nx*0.21+nz*0.18,
        nx*0.67-nz*0.43,
        (nx-nz)*1.31,
        nx*2.3+nz*1.9,
    );
    let t5=(nx*4.7-nz*3.1).sin();
    let n=0.40*t1 + 0.22*t2 + 0.11*t3 + 0.06*t4 + 0.03*t5;
    -1.25 + n*flat
}

// ── Per-frame context — eliminates repeated sin_cos from the map() hot path ───
// Precomputed once per frame, passed by reference to all geometry functions.

struct MapCtx {
    t:       f32,
    // Central gem: rot_x(p0, t*0.37) then rot_y(_, t*0.23)
    gem_sx:  f32, gem_cx: f32,
    gem_sy:  f32, gem_cy: f32,
    pulse_s: f32,               // (t*1.8).sin() — gem size pulse
    // Inner counter-rotating gem: rot_x(p0, -t*0.51) then rot_y(_, t*0.44)
    gm2_sx:  f32, gm2_cx: f32,
    gm2_sy:  f32, gm2_cy: f32,
    pulse2_s:f32,               // (t*2.3).sin() — inner gem size
    // Ring 0: rot_x(p0, t*0.70)
    r0_sx:   f32, r0_cx: f32,
    // Ring 1: rot_z( rot_y(p0, PI/2), -t*0.55 )  — rot_y(PI/2) is coord-swap
    r1_sz:   f32, r1_cz: f32,
    // Ring 2: rot_y( rot_x(p0, PI/2), t*0.33 )   — rot_x(PI/2) is coord-swap
    r2_sy:   f32, r2_cy: f32,
    // Ring 3: rot_x( rot_z(p0, 0.78), -t*0.44 )  — rot_z(0.78) is const
    r3_sx:   f32, r3_cx: f32,
    floor_s: f32,               // (t*0.3).sin() — floor inlay offset
    lp2_y:   f32,               // 1.3 + 0.8*(t*1.3).sin() — fill light y
}

impl MapCtx {
    fn new(t:f32)->Self {
        let (gem_sx, gem_cx) = (t*0.37).sin_cos();
        let (gem_sy, gem_cy) = (t*0.23).sin_cos();
        let (gm2_sx, gm2_cx) = (-t*0.51).sin_cos();
        let (gm2_sy, gm2_cy) = (t*0.44).sin_cos();
        let (r0_sx,  r0_cx)  = (t*0.70).sin_cos();
        let (r1_sz,  r1_cz)  = (-t*0.55).sin_cos();
        let (r2_sy,  r2_cy)  = (t*0.33).sin_cos();
        let (r3_sx,  r3_cx)  = (-t*0.44).sin_cos();
        Self {
            t,
            gem_sx, gem_cx, gem_sy, gem_cy,
            pulse_s: (t*1.8).sin(),
            gm2_sx, gm2_cx, gm2_sy, gm2_cy,
            pulse2_s: (t*2.3).sin(),
            r0_sx, r0_cx, r1_sz, r1_cz, r2_sy, r2_cy, r3_sx, r3_cx,
            floor_s: (t*0.3).sin(),
            lp2_y: 1.3+0.8*(t*1.3).sin(),
        }
    }
}

// ── Scene ─────────────────────────────────────────────────────────────────────

#[derive(Copy,Clone)] struct Hit { d:f32, m:i32 }

fn put(h:&mut Hit,d:f32,m:i32) { if d<h.d { h.d=d; h.m=m; } }
fn put_h(h:&mut Hit,o:Hit)     { if o.d<h.d { *h=o; } }

fn forest_hit(p0:V,t:f32)->Hit {
    let mut h=Hit{d:1e9,m:0};
    let r0=(p0.x*p0.x+p0.z*p0.z).sqrt();
    if r0<3.0||r0>14.5 { return h; }  // conservative: max jitter+reach ≈ 1.17+1.32=2.49

    let cell=2.2_f32;
    let gx=fast_floor(p0.x/cell);
    let gz=fast_floor(p0.z/cell);
    for di in -1_i32..2 {
        for dj in -1_i32..2 {
            let cx=(gx+di as f32)*cell;
            let cz=(gz+dj as f32)*cell;
            // Skip cells whose centre is too far for any tree to overlap p0
            let ddx=p0.x-cx; let ddz=p0.z-cz;
            if ddx*ddx+ddz*ddz > 6.5 { continue; }  // conservative: (jitter+reach)²≈6.19
            let cr=(cx*cx+cz*cz).sqrt();
            if cr<3.5||cr>14.2 { continue; }

            let seed=cx*13.71+cz*29.31;
            let h0=hash(seed);
            if h0<0.35 { continue; }
            let h1=hash(seed+1.0); let h2=hash(seed+2.0);
            let tx=cx+(h1-0.5)*cell*0.75;
            let tz=cz+(h2-0.5)*cell*0.75;
            let tr=(tx*tx+tz*tz).sqrt();
            if tr<3.8||tr>13.5 { continue; }

            let tree_ht=1.6+h0*1.0; let tree_r=0.44+h1*0.28;
            let base_y=-1.25+0.40*(tx*0.21+tz*0.18).sin()+0.22*(tx*0.67-tz*0.43).sin();
            let tp=p0-V::new(tx,base_y,tz);

            if (tp.x*tp.x+tp.z*tp.z).sqrt()>tree_r+0.6 { continue; }
            if tp.y < -0.3||tp.y>tree_ht+0.4 { continue; }

            put(&mut h, sd_cyl_y(tp-V::new(0.0,tree_ht*0.3,0.0), 0.05, tree_ht*0.3), 6);
            put(&mut h, tp.len() - tree_r*0.30, 6);

            let wind=0.055*(t*1.15+tx*0.7+tz*0.5).sin();

            for i in 0..5_i32 {
                let frac=i as f32 * 0.20;
                let fy=tree_ht*(0.38+frac);
                let fr=tree_r*(1.05-frac*1.1);
                if fr<=0.0 { break; }
                let sway=wind*fy*0.25;
                let fq=tp-V::new(sway, fy, sway*0.5);
                put(&mut h, (fq.x*fq.x+fq.z*fq.z+(fq.y*1.55)*(fq.y*1.55)).sqrt()-fr, 7);
            }
        }
    }
    h
}

// ring3 constant rotation component: rot_z(p0, 0.78) is a fixed rotation
// cos(0.78) ≈ 0.71073, sin(0.78) ≈ 0.70360
const COS_R3Z: f32 = 0.710_73;
const SIN_R3Z: f32 = 0.703_60;

fn map(p0:V,cx:&MapCtx)->Hit {
    let mut h=Hit{d:1e9,m:0};

    put(&mut h, p0.y - terrain_h(p0.x,p0.z), 4);

    // ── Temple complex ────────────────────────────────────────────────────────

    let a  =p0.z.atan2(p0.x);
    let rr =(p0.x*p0.x+p0.z*p0.z).sqrt();

    // Central gem + rings — at rr>2.5 their SDF is always >0.8, never minimum.
    // Skip the 4 rotmatmul + 4 sd_torus + 2 sd_octa for ~40% of march steps.
    if rr < 2.5 {
        // Central gem — precomputed rot_x then rot_y
        let gx=V::new(p0.x,
                      cx.gem_cx*p0.y - cx.gem_sx*p0.z,
                      cx.gem_sx*p0.y + cx.gem_cx*p0.z);
        let p =V::new(cx.gem_cy*gx.x + cx.gem_sy*gx.z,
                      gx.y,
                      -cx.gem_sy*gx.x + cx.gem_cy*gx.z);
        let pulse=0.08*cx.pulse_s;
        let gem=smin(sd_octa(p,0.95+pulse),p.len()-0.72,0.18)
               +0.025*((p.x*13.0).sin()*(p.y*11.0+cx.t).sin()*(p.z*9.0).sin());
        put(&mut h, gem, 1);

        // Inner gem — precomputed counter-rotation
        let g2x=V::new(p0.x,
                       cx.gm2_cx*p0.y - cx.gm2_sx*p0.z,
                       cx.gm2_sx*p0.y + cx.gm2_cx*p0.z);
        let p2 =V::new(cx.gm2_cy*g2x.x + cx.gm2_sy*g2x.z,
                       g2x.y,
                       -cx.gm2_sy*g2x.x + cx.gm2_cy*g2x.z);
        put(&mut h, sd_octa(p2, 0.48+0.04*cx.pulse2_s), 9);

        // Ring 0: rot_x(p0, t*0.70) — precomputed
        let r0q=V::new(p0.x,
                       cx.r0_cx*p0.y - cx.r0_sx*p0.z,
                       cx.r0_sx*p0.y + cx.r0_cx*p0.z);
        let r0=sd_torus(r0q, 1.18, 0.035);

        // Ring 1: rot_z( rot_y(p0, PI/2), -t*0.55 )
        //   rot_y(p0, PI/2) = (p0.z, p0.y, -p0.x) — exact 90° coordinate swap
        let t1 =V::new(p0.z, p0.y, -p0.x);
        let r1q=V::new(cx.r1_cz*t1.x - cx.r1_sz*t1.y,
                       cx.r1_sz*t1.x + cx.r1_cz*t1.y,
                       t1.z);
        let r1=sd_torus(r1q, 1.34, 0.026);

        // Ring 2: rot_y( rot_x(p0, PI/2), t*0.33 )
        //   rot_x(p0, PI/2) = (p0.x, -p0.z, p0.y) — exact 90° coordinate swap
        let t2 =V::new(p0.x, -p0.z, p0.y);
        let r2q=V::new(cx.r2_cy*t2.x + cx.r2_sy*t2.z,
                       t2.y,
                       -cx.r2_sy*t2.x + cx.r2_cy*t2.z);
        let r2=sd_torus(r2q, 1.52, 0.018);

        // Ring 3: rot_x( rot_z(p0, 0.78), -t*0.44 )
        //   rot_z(p0, 0.78) uses compile-time constants
        let r3_base=V::new(COS_R3Z*p0.x - SIN_R3Z*p0.y,
                           SIN_R3Z*p0.x + COS_R3Z*p0.y,
                           p0.z);
        let r3q=V::new(r3_base.x,
                       cx.r3_cx*r3_base.y - cx.r3_sx*r3_base.z,
                       cx.r3_sx*r3_base.y + cx.r3_cx*r3_base.z);
        let r3=sd_torus(r3q, 1.68, 0.013);
        put(&mut h, fmn(r0,fmn(r1,fmn(r2,r3))), 2);
    }

    // Columns at r=2.35 — skip when clearly out of range
    if rr > 1.8 && rr < 3.2 {
        let aa =rep(a,2.0*PI/8.0);
        let q  =V::new(rr-2.35,p0.y+0.15,aa*rr);  // +0.15: shaft bottom (half-h=1.1) reaches floor at y=-1.25
        let shaft=sd_cyl_y(q,0.07,1.1);
        let cap=fmn(sd_torus(q+V::new(0.0,-1.04,0.0),0.11,0.025),
                   sd_torus(q+V::new(0.0, 1.04,0.0),0.11,0.025));
        put(&mut h, fmn(shaft,cap), 3);
    }

    // Animated floor inlay with precomputed t-part
    let g=V::new(rep(p0.x+0.3*cx.floor_s,0.75),p0.y+1.215,rep(p0.z,0.75));
    put(&mut h, fmn(sd_box(g,V::new(0.20,0.018,0.018)),
                    sd_box(g,V::new(0.018,0.018,0.20))), 5);

    let aru =rep(a, 2.0*PI/16.0);
    let rune_q=V::new(rr-1.90, p0.y+1.215, aru*rr);
    put(&mut h, sd_box(rune_q, V::new(0.022,0.022,0.095)), 5);

    let a12 =a;  // same atan2 as `a` — reuse to avoid second atan2 call
    // Shards at r=1.72 — skip when clearly out of reach (saves hash + sd_box + sin)
    if rr > 1.2 && rr < 2.3 {
        // Consistent sector id: floor(a/s + 0.5) matches rep()'s modular boundary.
        let sector12=2.0*PI/12.0;
        let sid12=fast_floor(a12/sector12+0.5);
        let aa12=a12-sid12*sector12;
        let hs  =hash(sid12*7.0);
        let sq  =V::new(rr-1.72, p0.y-0.28*(hs*6.0+cx.t*0.5).sin(), aa12*rr);
        put(&mut h, sd_box(rot_z(sq,0.3+hs*0.55),V::new(0.04,0.33,0.09)), 9);
    }

    // Arches at r=3.45 — skip when clearly out of reach (saves 3 sd_box calls)
    if rr > 2.0 && rr < 5.0 {
        let a4  =rep(a,PI/2.0);
        let gp  =V::new(rr-3.45, p0.y+1.25, a4*rr*1.85);
        let pil_l=sd_box(gp+V::new(0.0,0.0,-0.55),V::new(0.10,1.00,0.10));
        let pil_r=sd_box(gp+V::new(0.0,0.0, 0.55),V::new(0.10,1.00,0.10));
        let lintel=sd_box(gp+V::new(0.0,1.02,0.0),V::new(0.10,0.13,0.67));
        put(&mut h, fmn(fmn(pil_l,pil_r),lintel), 3);
    }

    // Standing stones at r=4.75 — skip when clearly out of reach (saves hash + sin + sd_box)
    if rr > 3.5 && rr < 5.8 {
        let sector7=2.0*PI/7.0;
        let sid7=fast_floor(a/sector7+0.5);
        let a7=a-sid7*sector7;
        let stone_p=V::new(rr-4.75, p0.y+1.25, a7*rr*2.0);
        let stone_seed=sid7*9.3;
        let s_tilt=0.08*(stone_seed).sin();
        put(&mut h, sd_box(rot_z(stone_p,s_tilt),V::new(0.10,0.70+hash(stone_seed)*0.30,0.08)), 3);
    }

    let pr =(p0.x*p0.x+p0.z*p0.z).sqrt()-2.55;
    let wave=0.013*((p0.x*4.2+cx.t*2.1).sin()+(p0.z*3.7-cx.t*1.8).sin())
            +0.006*(noise3(V::new(p0.x*2.0,cx.t*0.5,p0.z*2.0)));
    put(&mut h, fmx((p0.y+1.228+wave).abs()-0.006, pr.abs()-0.55), 8);

    let pool_kerb=fmx((p0.y+1.10).abs()-0.12, ((p0.x*p0.x+p0.z*p0.z).sqrt()-2.55).abs()-0.62);
    put(&mut h, pool_kerb, 3);

    let altar_r=(p0.x*p0.x+p0.z*p0.z).sqrt();
    let altar_q=V::new(altar_r-0.0, p0.y+1.21, rep(a,2.0*PI/8.0)*altar_r);
    put(&mut h, sd_box(altar_q, V::new(0.72,0.035,0.32)), 3);
    put(&mut h, sd_cyl_y(V::new(p0.x,p0.y+1.18,p0.z), 0.28, 0.07), 3);

    // Inner monoliths at r=1.25 — skip when clearly out of reach
    if rr > 0.8 && rr < 1.9 {
        let sector5=2.0*PI/5.0;
        let sid5=fast_floor(a/sector5+0.5);
        let a5=a-sid5*sector5;
        let m5_seed=sid5*11.7;
        let m5_p=V::new(rr-1.25, p0.y+1.25, a5*rr*2.5);
        put(&mut h, sd_box(rot_z(m5_p,0.05*(m5_seed).sin()),
                           V::new(0.06, 0.35+hash(m5_seed)*0.20, 0.05)), 9);
    }

    // Forest — guarded by both XZ radius AND y range (skip for sky/underground rays)
    // r_xz == rr here (already computed)
    if rr>3.0&&rr<14.5 && p0.y>-2.5 && p0.y<3.2 {  // expanded: inner canopy at r≈3.08, outer at r≈14.2
        put_h(&mut h, forest_hit(p0,cx.t));
    }

    // Distant ruins — only evaluated when ray is far enough from origin to matter.
    // Guards the expensive terrain_h call inside; ruins at r=11.5 can't affect
    // the SDF when rr < 9.5 (minimum SDF ≥ 2.0, always dominated by nearer geometry).
    if rr > 9.5 {
        // Consistent sector id: use the same rounding as rep() (floor(a/s + 0.5))
        // so the random seed boundary matches the geometry boundary exactly.
        let sector6=2.0*PI/6.0;
        let sid6=fast_floor(a/sector6+0.5);
        let a6=a-sid6*sector6;
        let ruin_seed=sid6*17.1;
        let ruin_h=0.55+hash(ruin_seed)*0.65;
        let ruin_lean=0.3*hash(ruin_seed+1.0)-0.15;
        // ruin_p.x and .z only — y dropped because ruin_pp computes it from p0.y
        let ruin_p=V::new(rr-11.5, 0.0, a6*rr);
        let base_off=terrain_h(p0.x*(11.5/fmx(rr,0.01)), p0.z*(11.5/fmx(rr,0.01)));
        // Old code had p0.y+1.25-base_off-ruin_h; the +1.25 was wrong — it offset
        // the cylinder centre by 1.25 units above the intended terrain-height anchor.
        let ruin_pp=V::new(ruin_p.x, p0.y-(base_off+ruin_h), ruin_p.z);
        put(&mut h, sd_cyl_y(rot_z(ruin_pp,ruin_lean), 0.12, ruin_h), 3);
    }

    h
}

// ── Lighting ──────────────────────────────────────────────────────────────────

fn normal(p:V, mat:i32, cx:&MapCtx)->V {
    // For terrain hits, use fast finite-difference gradient on terrain_h only.
    // Replaces 4 full map() calls (each evaluating the entire temple complex)
    // with 4 cheap terrain_h evaluations — same visual quality, much faster.
    if mat == 4 {
        let e = 0.04_f32;
        let hxp = terrain_h(p.x + e, p.z);
        let hxn = terrain_h(p.x - e, p.z);
        let hzp = terrain_h(p.x, p.z + e);
        let hzn = terrain_h(p.x, p.z - e);
        return V::new(hxn - hxp, 2.0*e, hzn - hzp).norm();
    }
    let e=0.004;
    let k1=V::new(1.0,-1.0,-1.0); let k2=V::new(-1.0,-1.0,1.0);
    let k3=V::new(-1.0,1.0,-1.0); let k4=V::new(1.0,1.0,1.0);
    (k1*map(p+k1*e,cx).d+k2*map(p+k2*e,cx).d+
     k3*map(p+k3*e,cx).d+k4*map(p+k4*e,cx).d).norm()
}

fn shadow(ro:V,rd:V,cx:&MapCtx)->f32 {
    let mut res=1.0_f32; let mut d=0.035_f32;
    for _ in 0..36 {
        let h=map(ro+rd*d,cx).d;
        if h<0.002 { return 0.0; }
        res=fmn(res,9.0*h/d);
        d+=clamp(h,0.025,0.28);
        if d>8.0 { break; }
    }
    clamp(res,0.0,1.0)
}

fn ao(p:V,n:V,mat:i32,cx:&MapCtx)->f32 {
    // For terrain hits, skip full map() — only sample terrain height.
    // Terrain AO is dominated by terrain self-occlusion, not temple geometry.
    if mat == 4 {
        let mut o=0.0_f32; let mut sc=1.0_f32;
        for i in 1..6_i32 {
            let h = i as f32 * 0.10;
            let sp = p + n * h;
            let above = sp.y - terrain_h(sp.x, sp.z);
            o += fmx(h - above, 0.0) * sc;
            sc *= 0.5;
        }
        return 1.0 - clamp(o * 3.0, 0.0, 1.0);
    }
    let mut o=0.0_f32; let mut sc=1.0_f32;
    for i in 1..10 {
        let h=i as f32*0.048;
        o+=fmx(h-map(p+n*h,cx).d,0.0)*sc;
        sc*=0.57;
    }
    clamp(1.0-o*1.5,0.0,1.0)
}

fn palette(m:i32,p:V,t:f32)->V {
    match m {
        1 => {
            let hue=p.y*6.0+p.x*3.0+t;
            let hue2=p.z*5.0-p.y*4.0+t*1.3;
            let r=0.85+0.15*(hue).sin().abs();
            let g=0.30+0.30*(hue*1.5+1.2).sin().abs();
            let b=0.88+0.12*(hue2).sin().abs();
            V::new(r, g, b)
        },
        2 => {
            let s=0.8+0.2*(p.x*11.0+p.y*13.0).sin().abs();
            V::new(0.22*s, 0.90*s, 1.0*s)
        },
        3 => {
            let coarse=0.80+0.20*noise3(p*2.5);
            let fine  =0.90+0.10*noise3(p*6.0);
            let vein  =smoothstep(0.55,0.45,noise3(V::new(p.x*4.0+p.y*3.0,p.y*5.0,p.z*4.0)).abs());
            let tex   =coarse*fine*(1.0-vein*0.35);
            V::new(0.98*tex, 0.70*tex, 0.30*tex+vein*0.05)
        },
        4 => {
            let r=(p.x*p.x+p.z*p.z).sqrt();
            let forest_blend=smoothstep(3.5,6.5,r);
            // At terrain hit points p.y ≈ terrain_h(p.x,p.z), so height_above ≈ 0
            // and smoothstep(0.05, 0.0, ~0.001) ≈ 1.0 — skip the terrain_h call.
            let moisture = 1.0_f32;
            let rock_noise=0.80+0.20*noise3(p*2.5);
            let moss_noise=0.70+0.30*noise3(V::new(p.x*4.0,p.z*4.0,p.y));
            let g=forest_blend*0.14;
            let rock_col=V::new((0.07+g*0.35)*rock_noise, (0.06+g)*rock_noise, (0.12+g*0.18)*rock_noise);
            let moss_col=V::new(0.06*moss_noise, 0.18*moss_noise, 0.08*moss_noise);
            let t_ = clamp(moisture*forest_blend, 0.0, 1.0);
            rock_col*(1.0-t_) + moss_col*t_
        },
        5 => {
            let glow=0.7+0.3*(p.x*8.0+p.z*6.0+t*1.2).sin().abs();
            V::new(0.10*glow, 0.78*glow, 0.90*glow)
        },
        6 => {
            let grain=0.75+0.25*noise3(V::new(p.y*4.0,p.x*3.0,p.z*3.0));
            V::new(0.30*grain, 0.17*grain, 0.09*grain)
        },
        7 => {
            let v=0.70+0.30*noise3(p*5.0);
            V::new(0.07*v, 0.24*v+0.05*(p.y*4.0+p.x).sin().abs(), 0.09*v)
        },
        8 => V::new(0.05, 0.14, 0.42),
        9 => {
            let s=0.65+0.35*(p.y*9.0+t*1.5+p.x*5.0).sin().abs();
            V::new(0.70*s+0.25, 0.35*s, 0.95*s)
        },
        _ => V::new(0.0, 0.0, 0.0),
    }
}

fn sky(rd:V,t:f32)->V {
    let y=clamp(rd.y*0.5+0.5,0.0,1.0);
    let mut c=V::new(0.018,0.014,0.055)*(1.0-y)
            +V::new(0.014,0.075,0.175)*y;

    let u =rd.z.atan2(rd.x)*7.0;
    let v =(1.0-clamp(rd.y,-1.0,1.0))*(9.0*PI/2.0);
    let id=(fast_floor(u)*37.0+fast_floor(v)*113.0).abs();
    let star=smoothstep(0.995,1.0,hash(id+fast_floor(t*0.03)));
    let twinkle=0.5+0.5*(t*3.0+id).sin().abs();
    c=c+V::new(0.80,0.92,1.0)*star*twinkle;

    let cluster_dir=V::new(0.6,0.7,-0.38).norm();
    let cdot=clamp(rd.dot(cluster_dir),0.0,1.0);
    let cluster_mask=smoothstep(0.82,0.97,cdot);
    let u2=rd.z.atan2(rd.x)*19.0; let v2=(1.0-clamp(rd.y,-1.0,1.0))*(17.0*PI/2.0);
    let cid=(fast_floor(u2)*53.0+fast_floor(v2)*137.0).abs();
    let cstar=smoothstep(0.984,0.998,hash(cid+17.3))*cluster_mask;
    c=c+V::new(0.90,0.85,1.0)*cstar*(0.4+0.6*hash(cid));

    let mw_ang=rd.z.atan2(rd.x);
    let mw_band=smoothstep(0.18,0.04,(mw_ang*0.6+rd.y*0.8).abs()-0.04)
               +smoothstep(0.16,0.02,((mw_ang*0.6+rd.y*0.8)+0.3).abs()-0.02)*0.5;
    c=c+V::new(0.08,0.10,0.18)*mw_band*smoothstep(0.0,0.25,rd.y);

    let moon_d=V::new(-0.5,0.65,-0.6).norm();
    let md=clamp(rd.dot(moon_d),0.0,1.0);
    let moon=smoothstep(0.9998,1.0,md);
    let ht1=smoothstep(0.93,1.0,md); let ht2=smoothstep(0.70,1.0,md);
    c=c+V::new(0.92,0.94,1.0)*moon
     +V::new(0.12,0.22,0.40)*ht1*ht1*0.50
     +V::new(0.04,0.08,0.18)*ht2*0.15;

    let ang=rd.z.atan2(rd.x);
    let ridge=0.065+0.040*(ang*3.0).sin()
                   +0.022*(ang*9.0+t*0.04).sin()
                   +0.012*(ang*23.0).sin()
                   +0.006*(ang*51.0+t*0.10).sin();
    let mtn=smoothstep(ridge+0.018,ridge-0.010,rd.y);
    c=c*(1.0-mtn*0.87)+V::new(0.010,0.018,0.038)*mtn;

    let moon_d=V::new(-0.5,0.65,-0.6).norm();
    let horiz_ang=smoothstep(0.15,-0.05,rd.y);
    let moon_side=clamp(rd.dot(V::new(moon_d.x,0.0,moon_d.z).norm()),0.0,1.0);
    let horiz_glow=horiz_ang*moon_side*moon_side;
    c=c+V::new(0.25,0.30,0.55)*horiz_glow*0.18;

    let cld_h=smoothstep(0.08,0.26,rd.y)*(1.0-smoothstep(0.28,0.52,rd.y));
    let cw1=0.5+0.5*(ang*7.0+t*0.06).sin();
    let cw2=0.5+0.5*(ang*13.0-rd.y*9.0+t*0.04).sin();
    let cloud=(cw1*cw2)*(cw1*cw2)*cld_h;
    c=c+V::new(0.07,0.09,0.16)*cloud;

    let height_mask=smoothstep(0.04,0.42,rd.y)*(1.0-smoothstep(0.48,0.88,rd.y));
    let w1=0.5+0.5*(ang*17.0+rd.y*23.0+t*1.20).sin(); let w1=w1*w1;
    let w2=0.5+0.5*(ang*13.0+rd.y*15.0+t*0.90).sin(); let w2=w2*w2;
    let w3=0.5+0.5*(ang*23.0+rd.y*31.0+t*1.55).sin(); let w3=w3*w3;
    let band1=(0.5+0.5*(ang*3.2+t*0.17).sin())*height_mask*w1;
    let band2=(0.5+0.5*(ang*5.1+t*0.24+2.0).sin())*height_mask*w2;
    let band3=(0.5+0.5*(ang*7.3+t*0.11+4.5).sin())*height_mask*w3*0.7;
    let w4=0.5+0.5*(ang*29.0+rd.y*37.0+t*2.1).sin(); let w4=w4*w4;
    let band4=(0.5+0.5*(ang*11.0+t*0.31+1.2).sin())*height_mask*w4*0.6;
    c=c+V::new(0.04,0.65,0.38)*band1*0.28
     +V::new(0.55,0.08,0.92)*band2*0.22
     +V::new(0.08,0.40,0.85)*band3*0.18
     +V::new(0.80,0.30,0.60)*band4*0.15;

    let ndot=clamp(rd.dot(V::new(-0.4,0.55,0.72).norm()),0.0,1.0);
    let nebula=ndot*ndot*ndot*smoothstep(0.12,0.40,rd.y);
    c=c+V::new(0.12,0.08,0.30)*nebula*0.25;

    c
}

fn shade(ro:V,rd:V,cx:&MapCtx)->V {
    let t=cx.t;
    let sky_col=sky(rd,t);
    let mut depth=0.0_f32;
    let mut glow=V::new(0.0,0.0,0.0);
    let mut mat=0;

    for i in 0..80 {
        let p=ro+rd*depth;
        let h=map(p,cx);

        glow=glow
            +V::new(0.7,0.2,1.0)*(0.0015/(0.012+h.d.abs()))
            +V::new(0.1,0.8,1.0)*(0.0008/(0.020+(h.d+0.05).abs()));

        let xz2=p.x*p.x+p.z*p.z;
        let beam_fade=clamp(p.y+1.2,0.0,1.0)*clamp(2.8-p.y,0.0,1.0);
        glow=glow+V::new(0.25,0.65,1.0)*(0.003/(0.018+xz2))*beam_fade;

        let moon_shaft=clamp(p.y+0.5,0.0,1.0)*clamp(4.0-p.y,0.0,1.0)*0.3;
        glow=glow+V::new(0.60,0.65,0.90)*(0.0005/(0.06+xz2*0.15))*moon_shaft;

        let cell=2.0_f32;
        let drift=V::new(0.5*(t*0.17).sin(), t*0.20, 0.4*(t*0.14).cos());
        let fp=p+drift;
        let ix=fast_floor(fp.x/cell);
        let iy=fast_floor(fp.y/cell);
        let iz=fast_floor(fp.z/cell);
        let seed=ix*17.0+iy*57.0+iz*113.0;
        let pp=V::new(ix*cell+hash(seed+1.0)*cell,
                      iy*cell+hash(seed+2.0)*cell,
                      iz*cell+hash(seed+3.0)*cell);
        let fd=(fp-pp).len();
        let ff_mask=smoothstep(9.0,2.5,(p.x*p.x+p.z*p.z).sqrt());
        glow=glow+V::new(0.40,0.95,0.55)*(0.0008/(0.018+fd*fd))*ff_mask;

        let drift2=V::new(0.3*(t*0.23).cos(), t*0.38, 0.4*(t*0.19).sin());
        let fp2=p+drift2;
        let cell2=1.4_f32;
        let ex=fast_floor(fp2.x/cell2); let ey=fast_floor(fp2.y/cell2); let ez=fast_floor(fp2.z/cell2);
        let eseed=ex*23.0+ey*71.0+ez*137.0;
        let ep=V::new(ex*cell2+hash(eseed+4.0)*cell2,
                      ey*cell2+hash(eseed+5.0)*cell2,
                      ez*cell2+hash(eseed+6.0)*cell2);
        let ed=(fp2-ep).len();
        let em_mask=smoothstep(6.0,1.5,(p.x*p.x+p.z*p.z).sqrt())
                   *clamp(p.y+1.3,0.0,1.0)*clamp(3.0-p.y,0.0,1.0);
        glow=glow+V::new(0.90,0.55,0.15)*(0.0005/(0.022+ed*ed))*em_mask;

        let mist_vol=0.00035*(1.0-smoothstep(-0.4,0.8,p.y));
        glow=glow+V::new(0.06,0.09,0.20)*mist_vol;

        if h.d<0.0015*(1.0+depth*0.12) { mat=h.m; break; }
        depth+=clamp(h.d*0.80,0.006,0.35);  // 0.8× guard: map() is not a strict SDF (gem noise, smin)
        if depth>18.0||i==79 { return sky_col+glow*0.55; }
    }

    let p  =ro+rd*depth;
    let n  =normal(p,mat,cx);
    let base=palette(mat,p,t);

    let emit=match mat {
        1 => base*0.20,
        2 => base*0.32,
        5 => base*0.35,
        9 => base*0.25,
        _ => V::new(0.0,0.0,0.0),
    };

    let spec_scale=match mat { 1|9=>2.2_f32, 2=>1.8, 8=>3.5, 6=>0.18, 7=>0.07, 5=>0.8, _=>1.0 };

    let bn = match mat {
        4 => {
            let e=0.08;
            let bx=noise3(V::new(p.x+e,p.y,p.z))-noise3(V::new(p.x-e,p.y,p.z));
            let bz=noise3(V::new(p.x,p.y,p.z+e))-noise3(V::new(p.x,p.y,p.z-e));
            (n + V::new(bx,0.0,bz)*0.35).norm()
        },
        6 => {
            let e=0.06;
            let bx=noise3(V::new(p.y*2.0+e,p.z,p.x))-noise3(V::new(p.y*2.0-e,p.z,p.x));
            let by=noise3(V::new(p.x,p.y*2.0+e,p.z))-noise3(V::new(p.x,p.y*2.0-e,p.z));
            (n + V::new(bx,by,0.0)*0.30).norm()
        },
        _ => n,
    };
    let n = bn;

    // Precomputed light 1 position: reuse r0 angle (same t*0.7)
    let lp1=V::new(2.6*cx.r0_cx, 2.1, 2.6*cx.r0_sx);
    let lp2=V::new(-2.0, cx.lp2_y, -2.5);
    // Pass the unit normal (n is already normalised) so sky() samples the correct
    // direction.  Multiplying by n.y after the call gives the same cosine-weighted
    // contribution without passing a scaled or zero vector into sky().
    let sky_fill_col=if n.y>0.0 { sky(n,t)*(0.18*n.y) } else { V::new(0.0,0.0,0.0) };
    let mut col=base*0.06+emit+base*fmx(n.y,0.0)*0.05;
    col=col+V::new(sky_fill_col.x*base.x, sky_fill_col.y*base.y, sky_fill_col.z*base.z);
    for lp in [lp1,lp2] {
        let l  =(lp-p).norm();
        let dif=fmx(n.dot(l),0.0)*shadow(p+n*0.01,l,cx);
        let r  =(n*(2.0*n.dot(l))-l).norm();
        let spec=pow38(clamp(r.dot(-rd),0.0,1.0));
        col=col+base*dif+V::new(1.0,0.95,0.8)*spec*spec_scale*(0.55+0.45*dif);
    }

    let fr=clamp(1.0+n.dot(rd),0.0,1.0);  // clamp: overshoot landing can give n·rd>0 → fres>1
    let fres=fr*fr*fr;
    let ao_val=ao(p,n,mat,cx);
    col=col*ao_val+V::new(0.25,0.85,1.0)*fres;

    let rim_col=match mat {
        1 | 9 => V::new(0.50,0.20,1.00)*1.8,
        2     => V::new(0.10,0.80,1.00)*1.5,
        7     => V::new(0.05,0.50,0.15)*1.2,
        8     => V::new(0.10,0.40,0.90)*2.0,
        _     => V::new(0.22,0.75,1.00),
    };
    col=col+rim_col*fres*ao_val;

    if mat==7 {
        let sun_dir=(lp1-p).norm();
        let back_lit=fmx(-n.dot(sun_dir),0.0);
        let scatter=back_lit*back_lit*0.55*smoothstep(8.0,3.0,depth);
        col=col+V::new(0.08,0.35,0.10)*scatter;
    }

    if mat==5 {
        let rune_glow=0.4+0.6*(p.x*12.0+p.z*10.0+t*1.8).sin().abs();
        col=col+V::new(0.08,0.60,0.70)*rune_glow*0.25;
    }

    let sky_up=sky(V::new(0.0,1.0,0.0),t);
    let sky_contrib=fmx(n.y,0.0)*0.06*ao_val;
    col=col+sky_up*sky_contrib;

    let floor_pos=V::new(0.0,-1.21,0.0);
    let floor_vec=(floor_pos-p).norm();
    let floor_dist=(floor_pos-p).len();
    let floor_dif=fmx(-n.dot(floor_vec),0.0)*(1.0/(1.0+floor_dist*floor_dist*0.4));
    let floor_dif_v=V::new(0.10,0.78,0.90)*(floor_dif*0.20);
    col=col+V::new(base.x*floor_dif_v.x, base.y*floor_dif_v.y, base.z*floor_dif_v.z);

    if mat==8 {
        let refl=rd-n*(2.0*rd.dot(n));
        let rsky=sky(refl,t);
        let wfres=fmn(fres+0.4,1.0);
        col=col*(1.0-wfres)+rsky*wfres;
        let caust=0.3+0.7*(p.x*8.0+p.z*7.0+t*2.5).sin().abs();
        col=col*caust;
    }

    if mat==4 {
        let r_pool=(p.x*p.x+p.z*p.z).sqrt();
        let in_pool_zone=smoothstep(4.0,2.5,r_pool);
        if in_pool_zone>0.01 {
            let caust_u=p.x*9.0+t*2.2; let caust_v=p.z*8.0-t*1.9;
            let caustic=caust_u.sin()*caust_v.sin()*0.5
                        +(caust_u*0.7+caust_v*1.3).sin()*0.5;
            let caustic_val=fmx(caustic,0.0)*0.55;
            col=col+V::new(0.06,0.18,0.45)*caustic_val*in_pool_zone;
        }
    }

    let fog=smoothstep(3.0,16.0,depth);
    let mist=smoothstep(0.5,-0.9,p.y)*0.42;
    let mist_col=V::new(0.038,0.065,0.155);
    // Sequential fog → mist blend: attenuate both col and sky_col by mist so
    // fog never adds more energy than the surface already carries at that depth.
    let c1=col*(1.0-fog)+sky_col*fog;
    c1*(1.0-mist)+mist_col*mist+glow*0.6
}

// ── Camera choreography ───────────────────────────────────────────────────────

fn camera(t:f32)->(V,V,V,V) {
    // Compute the current 18s segment and the time offset within it.
    // Using the raw segment index instead of act*18 prevents the phase
    // from drifting after the first 90s cycle (when act would wrap to 0
    // but t would not, making t-act*18 arbitrarily large).
    let seg  =(t*(1.0/18.0)) as i32;
    let act  =seg % 5;
    let local=t - seg as f32*18.0;
    let k    =smoothstep(0.0,5.0,local);

    let (dist,ht,speed)=match act {
        0 => (7.0-k*3.0,  -0.2_f32+k*1.7, 0.07_f32),
        1 => (4.2,          1.2+0.3*(t*0.35).sin(), 0.14),
        2 => (2.3+0.3*(t*0.8).sin(), 0.2+0.5*(t*0.6).sin(), 0.40),
        3 => (4.0+k*4.5,   1.5+k*2.4,  0.09),
        _ => (6.5,          3.0+k*1.8,  0.06),
    };

    let tt=t*speed;
    let ro=V::new(dist*tt.cos(), ht, dist*tt.sin());
    let look_y=if act==4 { -0.28_f32+0.12*(t*0.25).sin() }
               else      { -0.08+0.18*(t*0.28).sin() };
    let ta=V::new(0.0,look_y,0.0);
    let f=(ta-ro).norm();
    let r=f.cross(V::new(0.0,1.0,0.0)).norm();
    let u=r.cross(f);
    (ro,f,r,u)
}

// ── Tone mapping ──────────────────────────────────────────────────────────────

fn grade(c:V,t:f32)->V {
    let progress=clamp(t*(1.0/90.0),0.0,1.0);  // cap at 1 so grading is stable past 90s
    let warm=V::new(1.0+progress*0.07, 1.0, 1.0-progress*0.06);
    let lift=V::new(0.012, 0.012, 0.022);
    let lum=c.x*0.299+c.y*0.587+c.z*0.114;
    let contrast=1.0+0.12*(lum-0.45);
    let c2=c+lift;
    let f=fmx(contrast, 0.3);
    V::new(c2.x*warm.x*f, c2.y*warm.y*f, c2.z*warm.z*f)
}

fn tonemap(c:V,t:f32)->(u8,u8,u8) {
    let gc=grade(c,t);
    let to_u8=|v:f32| (fmn(fmx(v/(1.0+v),0.0),1.0).sqrt()*255.0) as u8;
    (to_u8(gc.x), to_u8(gc.y), to_u8(gc.z))
}

// ── Frame builder ─────────────────────────────────────────────────────────────

/// Output a percentage value 0-100 as decimal (max 2 digits).
fn push_pct(out:&mut Out,v:u8) {
    if v>=10 { out.push(b'0'+v/10); }
    out.push(b'0'+v%10);
}

/// Phase 1: raytrace every pixel into the RGB framebuffer.
fn render_to_fb(w:usize,ph:usize,cx:&MapCtx) {
    let asp=w as f32/ph as f32;
    let (ro,f,r,u)=camera(cx.t);
    for y in 0..ph {
        for x in 0..w {
            let px=((x as f32+0.5)/w as f32*2.0-1.0)*asp;
            let py=1.0-(y as f32+0.5)/ph as f32*2.0;
            let rd=(f*1.35+r*px+u*py).norm();
            let vign=clamp(1.0-0.38*fmn(px*px+py*py,1.0)
                        -0.12*fmx(fmx(px.abs(),py.abs())-0.75,0.0), 0.0,1.0);
            let raw=shade(ro,rd,cx)*vign;
            let (rv,gv,bv)=tonemap(raw,cx.t);
            let base=(y*w+x)*3;
            unsafe {
                FB_PTR.add(base).write(rv);
                FB_PTR.add(base+1).write(gv);
                FB_PTR.add(base+2).write(bv);
            }
        }
    }
}

/// Phase 2: encode the RGB framebuffer as DEC sixel graphics.
///
/// Each sixel character encodes a 1-column × 6-row strip.  We make six
/// passes per sixel band so every pixel row gets its own colour definition
/// before the matching row-bit is drawn.  The colour slot (#0) is redefined
/// for each pixel; terminals that implement early-binding (DEC-standard)
/// display each pixel in the colour that was active when it was drawn.
fn fb_to_sixel(w:usize,ph:usize,out:&mut Out) {
    out.push_str("\x1b[H\x1bPq");          // home cursor, start sixel DCS
    let bands=ph/6;
    for band in 0..bands {
        for r in 0..6_usize {
            let y=band*6+r;
            let sc=b'?'+(1_u8<<r);         // sixel char: only bit r set
            for x in 0..w {
                unsafe {
                    let p=FB_PTR.add((y*w+x)*3);
                    let rv=*p; let gv=*p.add(1); let bv=*p.add(2);
                    // Convert 0-255 → 0-100 for sixel colour parameters
                    let rp=(rv as u16*100/255) as u8;
                    let gp=(gv as u16*100/255) as u8;
                    let bp=(bv as u16*100/255) as u8;
                    out.push_str("#0;2;");
                    push_pct(out,rp); out.push(b';');
                    push_pct(out,gp); out.push(b';');
                    push_pct(out,bp);
                    out.push(sc);
                }
            }
            out.push(b'$');                // sixel carriage-return within band
        }
        out.push(b'-');                    // advance to next sixel band
    }
    out.push_str("\x1b\\");               // String Terminator
}

// ── Main loop ─────────────────────────────────────────────────────────────────

pub(crate) fn run(seconds:f32) {
    let _term=Term::enter();
    // 512 KB pixel framebuffer: enough for up to ~300×170×3 bytes.
    unsafe { FB_PTR=alloc_anon(1<<19); }
    let start=clock_monotonic();
    let mut frame=Out(0);
    loop {
        let t=elapsed(&start);
        if t>seconds { break; }
        let (w,h)=term_size();
        // Round pixel height down to a multiple of 6 for complete sixel bands.
        let ph=(h*2/6)*6;
        let cx=MapCtx::new(t);
        frame.clear();
        render_to_fb(w,ph,&cx);
        fb_to_sixel(w,ph,&mut frame);
        frame.flush();
    }
}
