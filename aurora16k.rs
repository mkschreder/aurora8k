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
use sys::{F32Ext, Out, Term, clock_monotonic, elapsed, fast_floor, sleep_ms, term_size};

// ── Min / max / pow ───────────────────────────────────────────────────────────

#[inline(always)] fn fmn(a:f32,b:f32)->f32 { if a<b{a}else{b} }
#[inline(always)] fn fmx(a:f32,b:f32)->f32 { if a>b{a}else{b} }

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

// ── Trig-noise: cheap layered procedural noise ────────────────────────────────
// Three octaves of trig-based value noise.  Reused for terrain, bark, and fog.

fn noise3(p:V)->f32 {
    ((p.x*1.70+p.z*2.30).sin()
    +(p.x*3.10-p.y*1.90+p.z*0.70).sin()*0.50
    +(p.x*7.30+p.y*2.10+p.z*1.30).sin()*0.25) / 1.75
}

// ── Rotation helpers ──────────────────────────────────────────────────────────

fn rot_y(p:V,a:f32)->V { let (s,c)=a.sin_cos(); V::new(c*p.x+s*p.z,p.y,-s*p.x+c*p.z) }
fn rot_x(p:V,a:f32)->V { let (s,c)=a.sin_cos(); V::new(p.x,c*p.y-s*p.z,s*p.y+c*p.z) }
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

// ── Terrain ───────────────────────────────────────────────────────────────────
// Domain-warped multi-octave noise: flat clearing at centre, hills beyond.

fn terrain_h(x:f32,z:f32)->f32 {
    let flat=smoothstep(3.0,7.5,(x*x+z*z).sqrt());
    // Domain warp: shift sample coordinates with low-frequency noise
    let wx=noise3(V::new(x*0.3,0.0,z*0.3))*0.6;
    let wz=noise3(V::new(x*0.3+4.0,0.0,z*0.3+4.0))*0.6;
    let nx=x+wx; let nz=z+wz;
    let n = 0.40*(nx*0.21+nz*0.18).sin()
          + 0.22*(nx*0.67-nz*0.43).sin()
          + 0.11*((nx-nz)*1.31).sin()
          + 0.06*(nx*2.3+nz*1.9).sin()
          + 0.03*(nx*4.7-nz*3.1).sin();
    -1.25 + n*flat
}

// ── Scene ─────────────────────────────────────────────────────────────────────

#[derive(Copy,Clone)] struct Hit { d:f32, m:i32 }

fn put(h:&mut Hit,d:f32,m:i32) { if d<h.d { h.d=d; h.m=m; } }
fn put_h(h:&mut Hit,o:Hit)     { if o.d<h.d { *h=o; } }

fn forest_hit(p0:V,t:f32)->Hit {
    let mut h=Hit{d:1e9,m:0};
    let r0=(p0.x*p0.x+p0.z*p0.z).sqrt();
    if r0<3.8||r0>13.5 { return h; }

    let cell=2.2_f32;
    let gx=fast_floor(p0.x/cell);
    let gz=fast_floor(p0.z/cell);
    // 3×3 neighbourhood so we never miss an adjacent tree
    for di in -1_i32..2 {
        for dj in -1_i32..2 {
            let cx=(gx+di as f32)*cell;
            let cz=(gz+dj as f32)*cell;
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
            // Approximate terrain height under this tree (2-octave, avoids domain warp)
            let base_y=-1.25+0.40*(tx*0.21+tz*0.18).sin()+0.22*(tx*0.67-tz*0.43).sin();
            let tp=p0-V::new(tx,base_y,tz);

            if (tp.x*tp.x+tp.z*tp.z).sqrt()>tree_r+0.6 { continue; }
            if tp.y < -0.3||tp.y>tree_ht+0.4 { continue; }

            // Trunk with root flare
            put(&mut h, sd_cyl_y(tp-V::new(0.0,tree_ht*0.3,0.0), 0.05, tree_ht*0.3), 6);
            put(&mut h, tp.len() - tree_r*0.30, 6);  // root bulge sphere

            // Wind sway: position-seeded phase so each tree sways independently
            let wind=0.055*(t*1.15+tx*0.7+tz*0.5).sin();

            // Five stacked foliage ellipsoids — richer conifer silhouette
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

fn map(p0:V,t:f32)->Hit {
    let mut h=Hit{d:1e9,m:0};

    // Terrain ground
    put(&mut h, p0.y - terrain_h(p0.x,p0.z), 4);

    // ── Temple complex ────────────────────────────────────────────────────────

    // Central gem: smooth-min blend of octahedron + sphere, 3D surface ripple
    let p=rot_y(rot_x(p0,t*0.37),t*0.23);
    let pulse=0.08*(t*1.8).sin();
    let gem=smin(sd_octa(p,0.95+pulse),p.len()-0.72,0.18)
           +0.025*((p.x*13.0).sin()*(p.y*11.0+t).sin()*(p.z*9.0).sin());
    put(&mut h, gem, 1);

    // Inner gem: smaller octahedron counter-rotating, makes the gem feel alive
    let p2=rot_y(rot_x(p0,-t*0.51),t*0.44);
    put(&mut h, sd_octa(p2, 0.48+0.04*(t*2.3).sin()), 9);

    // Four orbital rings at different inclinations
    let r0=sd_torus(rot_x(p0,t*0.70),              1.18, 0.035);
    let r1=sd_torus(rot_z(rot_y(p0,1.57),-t*0.55), 1.34, 0.026);
    let r2=sd_torus(rot_y(rot_x(p0,1.57), t*0.33), 1.52, 0.018);
    let r3=sd_torus(rot_x(rot_z(p0,0.78),-t*0.44), 1.68, 0.013);
    put(&mut h, fmn(r0,fmn(r1,fmn(r2,r3))), 2);

    // Eight columns via polar domain repetition, with torus caps
    let a  =p0.z.atan2(p0.x);
    let rr =(p0.x*p0.x+p0.z*p0.z).sqrt();
    let aa =rep(a,2.0*PI/8.0);
    let q  =V::new(rr-2.35,p0.y,aa*rr);
    let shaft=sd_cyl_y(q,0.07,1.1);
    let cap=fmn(sd_torus(q+V::new(0.0,-1.04,0.0),0.11,0.025),
               sd_torus(q+V::new(0.0, 1.04,0.0),0.11,0.025));
    put(&mut h, fmn(shaft,cap), 3);

    // Animated floor inlay: cross-pattern of thin boxes
    let g=V::new(rep(p0.x+0.3*(t*0.3).sin(),0.75),p0.y+1.215,rep(p0.z,0.75));
    put(&mut h, fmn(sd_box(g,V::new(0.20,0.018,0.018)),
                    sd_box(g,V::new(0.018,0.018,0.20))), 5);

    // Rune circle: 16 thin stone tablets in ring at r=1.9
    let aru =rep(a, 2.0*PI/16.0);
    let rune_q=V::new(rr-1.90, p0.y+1.215, aru*rr);
    put(&mut h, sd_box(rune_q, V::new(0.022,0.022,0.095)), 5);

    // Twelve floating crystal shards in inner ring, each tilted + bobbing
    let a12 =p0.z.atan2(p0.x);
    let rr12=(p0.x*p0.x+p0.z*p0.z).sqrt();
    let aa12=rep(a12,2.0*PI/12.0);
    let hs  =hash(fast_floor(a12*(12.0/(2.0*PI)))*7.0);
    let sq  =V::new(rr12-1.72, p0.y-0.28*(hs*6.0+t*0.5).sin(), aa12*rr12);
    put(&mut h, sd_box(rot_z(sq,0.3+hs*0.55),V::new(0.04,0.33,0.09)), 9);

    // ── Clearing structures ───────────────────────────────────────────────────

    // Four stone gateway arches at cardinal directions of the clearing edge
    let a4  =rep(a,PI/2.0);                 // 4-fold angular symmetry
    let gp  =V::new(rr-3.45, p0.y+1.25, a4*rr*1.85);
    let pil_l=sd_box(gp+V::new(0.0,0.0,-0.55),V::new(0.10,1.00,0.10));
    let pil_r=sd_box(gp+V::new(0.0,0.0, 0.55),V::new(0.10,1.00,0.10));
    let lintel=sd_box(gp+V::new(0.0,1.02,0.0),V::new(0.10,0.13,0.67));
    put(&mut h, fmn(fmn(pil_l,pil_r),lintel), 3);

    // Seven ancient standing stones beyond gateways, ring at r=4.75
    let a7  =rep(a,2.0*PI/7.0);
    let stone_p=V::new(rr-4.75, p0.y+1.25, a7*rr*2.0);
    let stone_seed=fast_floor(a*(7.0/(2.0*PI)))*9.3;
    let s_tilt=0.08*(stone_seed).sin();
    put(&mut h, sd_box(rot_z(stone_p,s_tilt),V::new(0.10,0.70+hash(stone_seed)*0.30,0.08)), 3);

    // Annular water pool between columns and forest edge
    let pr =(p0.x*p0.x+p0.z*p0.z).sqrt()-2.55;
    let wave=0.013*((p0.x*4.2+t*2.1).sin()+(p0.z*3.7-t*1.8).sin())
            +0.006*(noise3(V::new(p0.x*2.0,t*0.5,p0.z*2.0)));
    put(&mut h, fmx((p0.y+1.228+wave).abs()-0.006, pr.abs()-0.55), 8);

    // Low stone kerb around water pool
    let pool_kerb=fmx((p0.y+1.10).abs()-0.12, ((p0.x*p0.x+p0.z*p0.z).sqrt()-2.55).abs()-0.62);
    put(&mut h, pool_kerb, 3);

    // Inner altar / plinth: low octagonal platform under the gem
    let altar_r=(p0.x*p0.x+p0.z*p0.z).sqrt();
    let altar_q=V::new(altar_r-0.0, p0.y+1.21, rep(a,2.0*PI/8.0)*altar_r);
    put(&mut h, sd_box(altar_q, V::new(0.72,0.035,0.32)), 3);
    // Central plinth column
    put(&mut h, sd_cyl_y(V::new(p0.x,p0.y+1.18,p0.z), 0.28, 0.07), 3);

    // Inner ring of 5 shorter monoliths at r=1.25 (between gem and water)
    let a5 =rep(a,2.0*PI/5.0);
    let m5_seed=fast_floor(a*(5.0/(2.0*PI)))*11.7;
    let m5_p=V::new(rr-1.25, p0.y+1.25, a5*rr*2.5);
    put(&mut h, sd_box(rot_z(m5_p,0.05*(m5_seed).sin()),
                       V::new(0.06, 0.35+hash(m5_seed)*0.20, 0.05)), 9);

    // ── Procedural forest (ring-gated) ────────────────────────────────────────
    let r_xz=(p0.x*p0.x+p0.z*p0.z).sqrt();
    if r_xz>3.5&&r_xz<14.0 { put_h(&mut h, forest_hit(p0,t)); }

    // Distant broken ruins: ring of fallen/tilted columns near forest edge (r=11.5)
    let a6  =rep(a, 2.0*PI/6.0);
    let ruin_seed=fast_floor(a*(6.0/(2.0*PI)))*17.1;
    let ruin_h=0.55+hash(ruin_seed)*0.65;
    let ruin_lean=0.3*hash(ruin_seed+1.0)-0.15;  // random lean angle
    let ruin_p=V::new(r_xz-11.5, p0.y+1.25, a6*r_xz);
    let base_off=terrain_h(p0.x*(11.5/fmx(r_xz,0.01)), p0.z*(11.5/fmx(r_xz,0.01)));
    let ruin_pp=V::new(ruin_p.x, ruin_p.y-base_off-ruin_h, ruin_p.z);
    put(&mut h, sd_cyl_y(rot_z(ruin_pp,ruin_lean), 0.12, ruin_h), 3);

    h
}

// ── Lighting ──────────────────────────────────────────────────────────────────

fn normal(p:V,t:f32)->V {
    let e=0.004;
    let k1=V::new(1.0,-1.0,-1.0); let k2=V::new(-1.0,-1.0,1.0);
    let k3=V::new(-1.0,1.0,-1.0); let k4=V::new(1.0,1.0,1.0);
    (k1*map(p+k1*e,t).d+k2*map(p+k2*e,t).d+
     k3*map(p+k3*e,t).d+k4*map(p+k4*e,t).d).norm()
}

fn shadow(ro:V,rd:V,t:f32)->f32 {
    let mut res=1.0_f32; let mut d=0.035_f32;
    for _ in 0..36 {
        let h=map(ro+rd*d,t).d;
        if h<0.002 { return 0.0; }
        res=fmn(res,9.0*h/d);
        d+=clamp(h,0.025,0.28);
        if d>8.0 { break; }
    }
    clamp(res,0.0,1.0)
}

fn ao(p:V,n:V,t:f32)->f32 {
    let mut o=0.0_f32; let mut sc=1.0_f32;
    for i in 1..10 {
        let h=i as f32*0.048;
        o+=fmx(h-map(p+n*h,t).d,0.0)*sc;
        sc*=0.57;
    }
    clamp(1.0-o*1.5,0.0,1.0)
}

fn palette(m:i32,p:V,t:f32)->V {
    match m {
        1 => {
            // Gem: full spectral prismatic colour shift — position and time driven
            let hue=p.y*6.0+p.x*3.0+t;
            let hue2=p.z*5.0-p.y*4.0+t*1.3;
            let r=0.85+0.15*(hue).sin().abs();
            let g=0.30+0.30*(hue*1.5+1.2).sin().abs();
            let b=0.88+0.12*(hue2).sin().abs();
            V::new(r, g, b)
        },
        2 => {
            // Rings: cyan with subtle metallic shimmer
            let s=0.8+0.2*(p.x*11.0+p.y*13.0).sin().abs();
            V::new(0.22*s, 0.90*s, 1.0*s)
        },
        3 => {
            // Stone: warm aged limestone with crack-like veining from layered noise
            let coarse=0.80+0.20*noise3(p*2.5);
            let fine  =0.90+0.10*noise3(p*6.0);
            let vein  =smoothstep(0.55,0.45,noise3(V::new(p.x*4.0+p.y*3.0,p.y*5.0,p.z*4.0)).abs());
            let tex   =coarse*fine*(1.0-vein*0.35);
            V::new(0.98*tex, 0.70*tex, 0.30*tex+vein*0.05)
        },
        4 => {
            // Terrain: dark stone in clearing → dark earth → forest floor
            // Height above terrain drives moss/dirt transitions
            let r=(p.x*p.x+p.z*p.z).sqrt();
            let forest_blend=smoothstep(3.5,6.5,r);
            let height_above=p.y - terrain_h(p.x,p.z);
            let moisture=smoothstep(0.05,0.0,height_above);  // damp near surface
            let rock_noise=0.80+0.20*noise3(p*2.5);
            let moss_noise=0.70+0.30*noise3(V::new(p.x*4.0,p.z*4.0,p.y));
            let g=forest_blend*0.14;
            let rock_col=V::new((0.07+g*0.35)*rock_noise, (0.06+g)*rock_noise, (0.12+g*0.18)*rock_noise);
            let moss_col=V::new(0.06*moss_noise, 0.18*moss_noise, 0.08*moss_noise);
            let t_ = clamp(moisture*forest_blend, 0.0, 1.0);
            rock_col*(1.0-t_) + moss_col*t_
        },
        5 => {
            // Floor inlay / runes: teal crystal with glow
            let glow=0.7+0.3*(p.x*8.0+p.z*6.0+t*1.2).sin().abs();
            V::new(0.10*glow, 0.78*glow, 0.90*glow)
        },
        6 => {
            // Bark: mottled brown with grain texture
            let grain=0.75+0.25*noise3(V::new(p.y*4.0,p.x*3.0,p.z*3.0));
            V::new(0.30*grain, 0.17*grain, 0.09*grain)
        },
        7 => {
            // Needles: layered green, darker inner, lighter outer
            let v=0.70+0.30*noise3(p*5.0);
            V::new(0.07*v, 0.24*v+0.05*(p.y*4.0+p.x).sin().abs(), 0.09*v)
        },
        8 => {
            // Water: deep blue-teal
            V::new(0.05, 0.14, 0.42)
        },
        9 => {
            // Shards: cool prismatic purple-white
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

    // Stars with per-star twinkling
    let u =rd.z.atan2(rd.x)*7.0;
    let v =(1.0-clamp(rd.y,-1.0,1.0))*(9.0*PI/2.0);
    let id=(fast_floor(u)*37.0+fast_floor(v)*113.0).abs();
    let star=smoothstep(0.995,1.0,hash(id+fast_floor(t*0.03)));
    let twinkle=0.5+0.5*(t*3.0+id).sin().abs();
    c=c+V::new(0.80,0.92,1.0)*star*twinkle;

    // Dense star cluster in one sky region (faked as bright patch)
    let cluster_dir=V::new(0.6,0.7,-0.38).norm();
    let cdot=clamp(rd.dot(cluster_dir),0.0,1.0);
    let cluster_mask=smoothstep(0.82,0.97,cdot);
    let u2=rd.z.atan2(rd.x)*19.0; let v2=(1.0-clamp(rd.y,-1.0,1.0))*(17.0*PI/2.0);
    let cid=(fast_floor(u2)*53.0+fast_floor(v2)*137.0).abs();
    let cstar=smoothstep(0.984,0.998,hash(cid+17.3))*cluster_mask;
    c=c+V::new(0.90,0.85,1.0)*cstar*(0.4+0.6*hash(cid));

    // Milky way: faint glowing band across the sky
    let mw_ang=rd.z.atan2(rd.x);
    let mw_band=smoothstep(0.18,0.04,(mw_ang*0.6+rd.y*0.8).abs()-0.04)
               +smoothstep(0.16,0.02,((mw_ang*0.6+rd.y*0.8)+0.3).abs()-0.02)*0.5;
    c=c+V::new(0.08,0.10,0.18)*mw_band*smoothstep(0.0,0.25,rd.y);

    // Moon disc + layered halo
    let moon_d=V::new(-0.5,0.65,-0.6).norm();
    let md=clamp(rd.dot(moon_d),0.0,1.0);
    let moon=smoothstep(0.9998,1.0,md);
    let ht1=smoothstep(0.93,1.0,md); let ht2=smoothstep(0.70,1.0,md);
    c=c+V::new(0.92,0.94,1.0)*moon
     +V::new(0.12,0.22,0.40)*ht1*ht1*0.50
     +V::new(0.04,0.08,0.18)*ht2*0.15;

    // Mountain silhouette: angular ridge sum, no extra SDF
    let ang=rd.z.atan2(rd.x);
    let ridge=0.065+0.040*(ang*3.0).sin()
                   +0.022*(ang*9.0+t*0.04).sin()
                   +0.012*(ang*23.0).sin()
                   +0.006*(ang*51.0+t*0.10).sin();
    let mtn=smoothstep(ridge+0.018,ridge-0.010,rd.y);
    c=c*(1.0-mtn*0.87)+V::new(0.010,0.018,0.038)*mtn;

    // Atmospheric horizon scattering: warm glow where moon lights the horizon
    let moon_d=V::new(-0.5,0.65,-0.6).norm();
    let horiz_ang=smoothstep(0.15,-0.05,rd.y);  // strong near horizon, zero above
    let moon_side=clamp(rd.dot(V::new(moon_d.x,0.0,moon_d.z).norm()),0.0,1.0);
    let horiz_glow=horiz_ang*moon_side*moon_side;
    c=c+V::new(0.25,0.30,0.55)*horiz_glow*0.18;  // cool moonlit horizon

    // Cloud wisps: 2 overlapping noise layers near horizon
    let cld_h=smoothstep(0.08,0.26,rd.y)*(1.0-smoothstep(0.28,0.52,rd.y));
    let cw1=0.5+0.5*(ang*7.0+t*0.06).sin();
    let cw2=0.5+0.5*(ang*13.0-rd.y*9.0+t*0.04).sin();
    let cloud=(cw1*cw2)*(cw1*cw2)*cld_h;
    c=c+V::new(0.07,0.09,0.16)*cloud;

    // Aurora ribbons — three animated colour bands
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

    // Nebula patch: diffuse glow near the cluster region
    let ndot=clamp(rd.dot(V::new(-0.4,0.55,0.72).norm()),0.0,1.0);
    let nebula=ndot*ndot*ndot*smoothstep(0.12,0.40,rd.y);
    c=c+V::new(0.12,0.08,0.30)*nebula*0.25;

    c
}

fn shade(ro:V,rd:V,t:f32)->V {
    let sky_col=sky(rd,t);
    let mut depth=0.0_f32;
    let mut glow=V::new(0.0,0.0,0.0);
    let mut mat=0;

    for i in 0..80 {
        let p=ro+rd*depth;
        let h=map(p,t);

        // Dual-colour volumetric glow around gem/rings
        glow=glow
            +V::new(0.7,0.2,1.0)*(0.0015/(0.012+h.d.abs()))
            +V::new(0.1,0.8,1.0)*(0.0008/(0.020+(h.d+0.05).abs()));

        // Energy beam: vertical column of light through the crystal
        let xz2=p.x*p.x+p.z*p.z;
        let beam_fade=clamp(p.y+1.2,0.0,1.0)*clamp(2.8-p.y,0.0,1.0);
        glow=glow+V::new(0.25,0.65,1.0)*(0.003/(0.018+xz2))*beam_fade;

        // Moon shaft: faint column of silver light descending from above
        let moon_shaft=clamp(p.y+0.5,0.0,1.0)*clamp(4.0-p.y,0.0,1.0)*0.3;
        glow=glow+V::new(0.60,0.65,0.90)*(0.0005/(0.06+xz2*0.15))*moon_shaft;

        // Fireflies: hashed 3D cell grid, drifting slowly upward
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

        // Ember sparks: second particle field rising faster, warm colour
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

        // Volumetric ground mist: scatters ambient light near terrain
        let mist_vol=0.00035*(1.0-smoothstep(-0.4,0.8,p.y));
        glow=glow+V::new(0.06,0.09,0.20)*mist_vol;

        if h.d<0.0015*(1.0+depth*0.12) { mat=h.m; break; }
        depth+=clamp(h.d,0.006,0.45);
        if depth>18.0||i==79 { return sky_col+glow*0.55; }
    }

    let p  =ro+rd*depth;
    let n  =normal(p,t);
    let base=palette(mat,p,t);

    // Self-emission for glowing materials
    let emit=match mat {
        1 => base*0.20,
        2 => base*0.32,
        5 => base*0.35,
        9 => base*0.25,
        _ => V::new(0.0,0.0,0.0),
    };

    // Material-specific specular scale and shading variants
    let spec_scale=match mat { 1|9=>2.2_f32, 2=>1.8, 8=>3.5, 6=>0.18, 7=>0.07, 5=>0.8, _=>1.0 };

    // Bump-mapped normals for terrain and bark (noise-derived micro-surface)
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

    // Three lights: orbiting key + fill + sky dome fill (aurora-tinted)
    let lp1=V::new(2.6*(t*0.7).cos(), 2.1, 2.6*(t*0.7).sin());
    let lp2=V::new(-2.0, 1.3+0.8*(t*1.3).sin(), -2.5);
    // Sky dome fill: represents indirect aurora/sky illumination from above
    let sky_fill_col=sky(n*fmx(n.y,0.0),t)*0.18;  // cheap sky sample in normal direction
    let mut col=base*0.06+emit+base*fmx(n.y,0.0)*0.05;
    col=col+V::new(sky_fill_col.x*base.x, sky_fill_col.y*base.y, sky_fill_col.z*base.z);
    for lp in [lp1,lp2] {
        let l  =(lp-p).norm();
        let dif=fmx(n.dot(l),0.0)*shadow(p+n*0.01,l,t);
        let r  =(n*(2.0*n.dot(l))-l).norm();
        let spec=pow38(clamp(r.dot(-rd),0.0,1.0));
        col=col+base*dif+V::new(1.0,0.95,0.8)*spec*spec_scale*(0.55+0.45*dif);
    }

    // Fresnel rim + AO
    let fr=fmx(1.0+n.dot(rd),0.0); let fres=fr*fr*fr;
    let ao_val=ao(p,n,t);
    col=col*ao_val+V::new(0.25,0.85,1.0)*fres;

    // Per-material coloured rim: each material has a characteristic edge glow
    let rim_col=match mat {
        1 | 9 => V::new(0.50,0.20,1.00)*1.8,  // gem/shards: purple rim
        2     => V::new(0.10,0.80,1.00)*1.5,   // rings: cyan rim
        7     => V::new(0.05,0.50,0.15)*1.2,   // needles: green backlight
        8     => V::new(0.10,0.40,0.90)*2.0,   // water: strong blue rim
        _     => V::new(0.22,0.75,1.00),        // default: sky blue
    };
    col=col+rim_col*fres*ao_val;

    // Leaves: fake subsurface scattering — back-lit translucency
    if mat==7 {
        let sun_dir=(lp1-p).norm();
        let back_lit=fmx(-n.dot(sun_dir),0.0);
        let scatter=back_lit*back_lit*0.55*smoothstep(8.0,3.0,depth);
        col=col+V::new(0.08,0.35,0.10)*scatter;
    }

    // Rune/inlay: adds emissive contribution to surrounding glow
    if mat==5 {
        let rune_glow=0.4+0.6*(p.x*12.0+p.z*10.0+t*1.8).sin().abs();
        col=col+V::new(0.08,0.60,0.70)*rune_glow*0.25;
    }

    // Aurora ambient: faint sky-coloured fill from overhead
    let sky_up=sky(V::new(0.0,1.0,0.0),t);
    let sky_contrib=fmx(n.y,0.0)*0.06*ao_val;
    col=col+sky_up*sky_contrib;

    // Rune ground glow: warm teal fill light emanating upward from floor inlay
    let floor_pos=V::new(0.0,-1.21,0.0);
    let floor_vec=(floor_pos-p).norm();
    let floor_dist=(floor_pos-p).len();
    let floor_dif=fmx(-n.dot(floor_vec),0.0)*(1.0/(1.0+floor_dist*floor_dist*0.4));
    let floor_dif_v=V::new(0.10,0.78,0.90)*(floor_dif*0.20);
    col=col+V::new(base.x*floor_dif_v.x, base.y*floor_dif_v.y, base.z*floor_dif_v.z);

    // Water: fake mirror reflection on reflected ray + caustic tint
    if mat==8 {
        let refl=rd-n*(2.0*rd.dot(n));
        let rsky=sky(refl,t);
        let wfres=fmn(fres+0.4,1.0);
        col=col*(1.0-wfres)+rsky*wfres;
        // Caustic shimmer on water surface
        let caust=0.3+0.7*(p.x*8.0+p.z*7.0+t*2.5).sin().abs();
        col=col*caust;
    }

    // Water caustic projected light: animated ripple pattern on terrain near pool
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

    // Distance fog + low ground mist blend
    let fog=smoothstep(3.0,16.0,depth);
    let mist=smoothstep(0.5,-0.9,p.y)*0.42;
    let mist_col=V::new(0.038,0.065,0.155);
    col*(1.0-fog)*(1.0-mist)+sky_col*fog+mist_col*mist+glow*0.6
}

// ── Camera choreography ───────────────────────────────────────────────────────
// Five 18-second acts: forest approach → temple orbit → gem close-up
//                      → reveal pull-back → aurora sky finale.

fn camera(t:f32)->(V,V,V,V) {
    let act=(t*(1.0/18.0)) as i32 % 5;
    let k  =smoothstep(0.0,5.0,t-act as f32*18.0);

    let (dist,ht,speed)=match act {
        0 => (7.0-k*3.0,  -0.2_f32+k*1.7, 0.07_f32), // low forest approach
        1 => (4.2,          1.2+0.3*(t*0.35).sin(), 0.14), // temple orbit
        2 => (2.3+0.3*(t*0.8).sin(), 0.2+0.5*(t*0.6).sin(), 0.40), // gem close-up
        3 => (4.0+k*4.5,   1.5+k*2.4,  0.09),          // pull-back reveal
        _ => (6.5,          3.0+k*1.8,  0.06),          // aurora sky finale
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

// Subtle colour grade: warm drift over time, mild contrast S-curve, lifted blacks.
fn grade(c:V,t:f32)->V {
    let progress=t*(1.0/90.0); // 0→1 over demo runtime
    // Slowly warm up colour temperature (cool start → slightly warm end)
    let warm=V::new(1.0+progress*0.07, 1.0, 1.0-progress*0.06);
    // Shadow lift (crush blacks slightly for atmosphere)
    let lift=V::new(0.012, 0.012, 0.022);
    // Mild contrast boost around midtone
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

fn push_u8(out:&mut Out,v:u8) {
    if v>=100 { out.push(b'0'+v/100); }
    if v>=10  { out.push(b'0'+(v/10)%10); }
    out.push(b'0'+v%10);
}

fn push_time(out:&mut Out,t:f32) {
    let s=t as u32;
    let d=(((t-s as f32)*10.0) as u32).min(9);
    if s<10 { out.push_str("  "); } else if s<100 { out.push(b' '); }
    push_u8(out,s as u8);
    out.push(b'.'); out.push(b'0'+(d as u8));
    out.push(b's');
}

fn render(w:usize,h:usize,t:f32,out:&mut Out) {
    out.push_str("\x1b[H");
    let pix_h=h*2;
    let asp=w as f32/pix_h as f32;
    let (ro,f,r,u)=camera(t);

    for y in 0..h {
        for x in 0..w {
            let mut rgb=[(0_u8,0_u8,0_u8);2];
            for sy in 0..2 {
                let yy=y*2+sy;
                let px=((x as f32+0.5)/w as f32*2.0-1.0)*asp;
                let py=1.0-(yy as f32+0.5)/pix_h as f32*2.0;
                let rd=(f*1.35+r*px+u*py).norm();
                // Vignette: soft circular + corner darkening
                let vign=1.0-0.38*fmn(px*px+py*py,1.0)
                            -0.12*fmx(fmx(px.abs(),py.abs())-0.75,0.0);
                // Film grain: cheap hash-based noise for cinematic texture
                let grain_seed=(x as f32*1031.0+yy as f32*2999.0+t*7919.0).sin()*43758.5453;
                let grain=0.03*((grain_seed-fast_floor(grain_seed))*2.0-1.0);
                let raw=shade(ro,rd,t)*vign;
                rgb[sy]=tonemap(V::new(raw.x+grain,raw.y+grain,raw.z+grain*0.8),t);
            }
            let (r1,g1,b1)=rgb[0]; let (r2,g2,b2)=rgb[1];
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

    // Progress bar with elapsed time
    const TITLE: &[u8] = b" AURORA16K / CPU SDF RUST ";
    let pct=fmn(t*(100.0/90.0), 100.0) as usize;
    let barw=w.saturating_sub(TITLE.len()+14);
    let fill=barw*pct/100;
    let left=w.saturating_sub(TITLE.len())/2;
    out.push_str("\x1b[0m");
    for _ in 0..left { out.push(b' '); }
    for &b in TITLE { out.push(b); }
    out.push_str("  \x1b[38;2;80;200;255m");
    for i in 0..barw {
        if i<fill { out.push(0xe2); out.push(0x96); out.push(0xa0); }  // ■
        else       { out.push(0xc2); out.push(0xb7); }                  // ·
    }
    out.push_str("  \x1b[38;2;140;200;255m");
    push_time(out,t);
    out.push_str("\x1b[0m");
}

// ── Main loop ─────────────────────────────────────────────────────────────────

pub(crate) fn run(seconds:f32) {
    let _term=Term::enter();
    let start=clock_monotonic();
    let mut frame=Out(0);
    loop {
        let t=elapsed(&start);
        if t>seconds { break; }
        let (w,h)=term_size();
        frame.clear();
        render(w,h,t,&mut frame);
        frame.flush();
        sleep_ms(16);
    }
}
