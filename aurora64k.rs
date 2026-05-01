// aurora64k.rs - a single-file CPU-only Rust terminal 64k-style intro.
// Build: rustc aurora64k.rs -O -C opt-level=z -C panic=abort -C lto=fat -C codegen-units=1 -o aurora64k
// Run:   ./aurora64k            # about 90 seconds
//        ./aurora64k 30         # 30 seconds
// Notes: truecolor terminal recommended. No crates, no GPU, no assets, only procedural CPU rendering.

use std::f32::consts::PI;
use std::fmt::Write as FmtWrite;
use std::io::{self, Write as IoWrite};
use std::thread::sleep;
use std::time::{Duration, Instant};

#[derive(Copy, Clone)]
struct V { x: f32, y: f32, z: f32 }

impl V {
    fn new(x: f32, y: f32, z: f32) -> Self { Self { x, y, z } }
    fn dot(self, b: Self) -> f32 { self.x*b.x + self.y*b.y + self.z*b.z }
    fn cross(self, b: Self) -> Self {
        Self::new(self.y*b.z-self.z*b.y, self.z*b.x-self.x*b.z, self.x*b.y-self.y*b.x)
    }
    fn len(self) -> f32 { self.dot(self).sqrt() }
    fn norm(self) -> Self { self / self.len().max(1e-6) }
    fn abs(self) -> Self { Self::new(self.x.abs(), self.y.abs(), self.z.abs()) }
    fn max(self, b: Self) -> Self { Self::new(self.x.max(b.x), self.y.max(b.y), self.z.max(b.z)) }
}

use std::ops::{Add, Div, Mul, Neg, Sub};
impl Add for V { type Output=Self; fn add(self,b:Self)->Self{Self::new(self.x+b.x,self.y+b.y,self.z+b.z)} }
impl Sub for V { type Output=Self; fn sub(self,b:Self)->Self{Self::new(self.x-b.x,self.y-b.y,self.z-b.z)} }
impl Mul<f32> for V { type Output=Self; fn mul(self,s:f32)->Self{Self::new(self.x*s,self.y*s,self.z*s)} }
impl Div<f32> for V { type Output=Self; fn div(self,s:f32)->Self{Self::new(self.x/s,self.y/s,self.z/s)} }
impl Neg for V { type Output=Self; fn neg(self)->Self{Self::new(-self.x,-self.y,-self.z)} }

fn clamp(x:f32,a:f32,b:f32)->f32 { x.max(a).min(b) }
fn mix(a:f32,b:f32,t:f32)->f32 { a + (b-a)*t }
fn smoothstep(a:f32,b:f32,x:f32)->f32 { let t=clamp((x-a)/(b-a),0.0,1.0); t*t*(3.0-2.0*t) }
fn hash(mut n:f32)->f32 { n = (n*17.13).sin()*43758.5453; n - n.floor() }
fn rep(x:f32, c:f32)->f32 { (x + 0.5*c).rem_euclid(c) - 0.5*c }

fn rot_y(p:V, a:f32)->V { let (s,c)=a.sin_cos(); V::new(c*p.x+s*p.z, p.y, -s*p.x+c*p.z) }
fn rot_x(p:V, a:f32)->V { let (s,c)=a.sin_cos(); V::new(p.x, c*p.y-s*p.z, s*p.y+c*p.z) }
fn rot_z(p:V, a:f32)->V { let (s,c)=a.sin_cos(); V::new(c*p.x-s*p.y, s*p.x+c*p.y, p.z) }

fn sd_box(p:V, b:V)->f32 {
    let q = p.abs() - b;
    q.max(V::new(0.0,0.0,0.0)).len() + q.x.max(q.y.max(q.z)).min(0.0)
}
fn sd_octa(p:V, s:f32)->f32 { (p.x.abs()+p.y.abs()+p.z.abs()-s)*0.57735027 }
fn sd_torus(p:V, r:f32, tube:f32)->f32 {
    let qx = (p.x*p.x + p.z*p.z).sqrt() - r;
    (qx*qx + p.y*p.y).sqrt() - tube
}
fn sd_cyl_y(p:V, r:f32, h:f32)->f32 {
    let dx = (p.x*p.x + p.z*p.z).sqrt() - r;
    let dy = p.y.abs() - h;
    dx.max(dy).min(0.0) + (dx.max(0.0)*dx.max(0.0) + dy.max(0.0)*dy.max(0.0)).sqrt()
}
fn smin(a:f32,b:f32,k:f32)->f32 { let h=clamp(0.5+0.5*(b-a)/k,0.0,1.0); mix(b,a,h)-k*h*(1.0-h) }

#[derive(Copy, Clone)]
struct Hit { d:f32, m:i32 }
fn put(best:&mut Hit, d:f32, m:i32) { if d < best.d { best.d=d; best.m=m; } }

fn map(p0:V, t:f32)->Hit {
    let mut h = Hit { d: 1e9, m: 0 };

    // Living ground: a shallow procedural height field that still behaves conservatively enough for this demo.
    let floor_wave = 0.035*((p0.x*2.0+t*0.8).sin()*(p0.z*1.5-t*0.4).sin());
    put(&mut h, p0.y + 1.25 + floor_wave, 4);

    // Central gem: CSG-ish union of octahedra and sphere with cheap displacement.
    let mut p = rot_y(rot_x(p0, t*0.37), t*0.23);
    let pulse = 0.08*(t*1.8).sin();
    let gem = smin(sd_octa(p, 0.95+pulse), p.len()-0.72, 0.18)
            + 0.025*((p.x*13.0).sin()*(p.y*11.0+t).sin()*(p.z*9.0).sin());
    put(&mut h, gem, 1);

    // Three orbital rings, intentionally repetitive: good for the source and for the visual language.
    let r0 = sd_torus(rot_x(p0, t*0.7), 1.18, 0.035);
    let r1 = sd_torus(rot_z(rot_y(p0, 1.57), -t*0.55), 1.34, 0.026);
    let r2 = sd_torus(rot_y(rot_x(p0, 1.57), t*0.33), 1.52, 0.018);
    put(&mut h, r0.min(r1).min(r2), 2);

    // Polar repetition: eight temple columns for the price of one.
    let a = p0.z.atan2(p0.x);
    let rr = (p0.x*p0.x + p0.z*p0.z).sqrt();
    let sector = 2.0*PI/8.0;
    let aa = rep(a, sector);
    let q = V::new(rr-2.35, p0.y, aa*rr);
    let shaft = sd_cyl_y(q, 0.07, 1.1);
    let cap = sd_torus(q + V::new(0.0, -1.04, 0.0), 0.11, 0.025)
            .min(sd_torus(q + V::new(0.0,  1.04, 0.0), 0.11, 0.025));
    put(&mut h, shaft.min(cap), 3);

    // Infinite tiled floor inlay: repeated boxes cut into the floor visually.
    let g = V::new(rep(p0.x + 0.3*(t*0.3).sin(), 0.75), p0.y+1.215, rep(p0.z, 0.75));
    let inlay = sd_box(g, V::new(0.20, 0.018, 0.018)).min(sd_box(g, V::new(0.018, 0.018, 0.20)));
    put(&mut h, inlay, 5);
    h
}

fn normal(p:V,t:f32)->V {
    let e = 0.004;
    V::new(
        map(p+V::new(e,0.0,0.0),t).d - map(p-V::new(e,0.0,0.0),t).d,
        map(p+V::new(0.0,e,0.0),t).d - map(p-V::new(0.0,e,0.0),t).d,
        map(p+V::new(0.0,0.0,e),t).d - map(p-V::new(0.0,0.0,e),t).d,
    ).norm()
}

fn shadow(ro:V, rd:V, t:f32)->f32 {
    let mut res: f32 = 1.0;
    let mut d = 0.035;
    for _ in 0..28 {
        let h = map(ro + rd*d, t).d;
        if h < 0.002 { return 0.0; }
        res = res.min(9.0*h/d);
        d += clamp(h, 0.025, 0.28);
        if d > 7.0 { break; }
    }
    clamp(res,0.0,1.0)
}

fn ao(p:V, n:V, t:f32)->f32 {
    let mut o = 0.0;
    let mut sc = 1.0;
    for i in 1..6 {
        let h = i as f32 * 0.055;
        o += (h - map(p + n*h, t).d).max(0.0) * sc;
        sc *= 0.55;
    }
    clamp(1.0 - o*1.8, 0.0, 1.0)
}

fn palette(m:i32, p:V, t:f32)->V {
    match m {
        1 => V::new(1.0, 0.35 + 0.30*(p.y*6.0+t).sin().abs(), 0.95),
        2 => V::new(0.25, 0.95, 1.0),
        3 => V::new(1.0, 0.72, 0.32),
        4 => V::new(0.10, 0.09, 0.20),
        5 => V::new(0.12, 0.80, 0.95),
        _ => V::new(0.0,0.0,0.0),
    }
}

fn sky(rd:V, t:f32)->V {
    let y = clamp(rd.y*0.5 + 0.5, 0.0, 1.0);
    let mut c = V::new(0.03,0.025,0.08)*(1.0-y) + V::new(0.02,0.10,0.18)*y;
    let u = rd.z.atan2(rd.x)*7.0;
    let v = clamp(rd.y,-1.0,1.0).acos()*9.0;
    let id = (u.floor()*37.0 + v.floor()*113.0).abs();
    let star = smoothstep(0.996, 1.0, hash(id + (t*0.03).floor()));
    c = c + V::new(0.75,0.9,1.0)*star*(0.45+0.55*(t*3.0+id).sin().abs());
    c
}

fn shade(ro:V, rd:V, t:f32)->V {
    let mut depth = 0.0;
    let mut glow = V::new(0.0,0.0,0.0);
    let mut mat = 0;
    for i in 0..76 {
        let p = ro + rd*depth;
        let h = map(p,t);
        glow = glow + V::new(0.7,0.2,1.0) * (0.0015 / (0.012 + h.d.abs()))
             + V::new(0.1,0.8,1.0) * (0.0008 / (0.020 + (h.d+0.05).abs()));
        if h.d < 0.0015*(1.0 + depth*0.12) { mat = h.m; break; }
        depth += clamp(h.d, 0.006, 0.45);
        if depth > 18.0 || i == 75 { return sky(rd,t) + glow*0.55; }
    }

    let p = ro + rd*depth;
    let n = normal(p,t);
    let base = palette(mat,p,t);
    let lp1 = V::new(2.6*(t*0.7).cos(), 2.1, 2.6*(t*0.7).sin());
    let lp2 = V::new(-2.0, 1.3 + 0.8*(t*1.3).sin(), -2.5);
    let mut col = base*0.08;
    for lp in [lp1, lp2].iter().copied() {
        let l = (lp-p).norm();
        let dif = n.dot(l).max(0.0) * shadow(p+n*0.01, l, t);
        let r = (n*(2.0*n.dot(l)) - l).norm();
        let spec = clamp(r.dot(-rd),0.0,1.0).powf(38.0);
        col = col + base*dif + V::new(1.0,0.95,0.8)*spec*(0.55+0.45*dif);
    }
    let fres = (1.0 + n.dot(rd)).max(0.0).powf(3.0);
    col = col*ao(p,n,t) + V::new(0.25,0.85,1.0)*fres;
    let fog = smoothstep(3.0, 15.0, depth);
    col*(1.0-fog) + sky(rd,t)*fog + glow*0.6
}

fn camera(t:f32)->(V,V,V,V) {
    let tt = t*0.22;
    let ro = V::new(4.0*tt.cos(), 1.25 + 0.35*(t*0.4).sin(), 4.0*tt.sin());
    let ta = V::new(0.0, -0.05 + 0.25*(t*0.3).sin(), 0.0);
    let f = (ta-ro).norm();
    let r = f.cross(V::new(0.0,1.0,0.0)).norm();
    let u = r.cross(f);
    (ro,f,r,u)
}

fn tonemap(c:V)->(u8,u8,u8) {
    let mut r = c.x / (1.0+c.x);
    let mut g = c.y / (1.0+c.y);
    let mut b = c.z / (1.0+c.z);
    r = r.powf(0.4545); g = g.powf(0.4545); b = b.powf(0.4545);
    ((clamp(r,0.0,1.0)*255.0) as u8, (clamp(g,0.0,1.0)*255.0) as u8, (clamp(b,0.0,1.0)*255.0) as u8)
}

fn render(w:usize, h:usize, t:f32, out:&mut String) {
    out.push_str("\x1b[H");
    let pix_h = h*2;
    let asp = w as f32 / pix_h as f32;
    let (ro,f,r,u) = camera(t);
    let title = " AURORA64K / CPU SDF RUST ";
    for y in 0..h {
        for x in 0..w {
            let mut rgb = [(0,0,0); 2];
            for sy in 0..2 {
                let yy = y*2 + sy;
                let px = ((x as f32 + 0.5) / w as f32 * 2.0 - 1.0) * asp;
                let py = 1.0 - (yy as f32 + 0.5) / pix_h as f32 * 2.0;
                let rd = (f*1.35 + r*px + u*py).norm();
                let mut c = shade(ro,rd,t);
                let vign = 1.0 - 0.35*(px*px + py*py).min(1.0);
                c = c * vign;
                rgb[sy] = tonemap(c);
            }
            let (r1,g1,b1) = rgb[0];
            let (r2,g2,b2) = rgb[1];
            let _ = write!(out, "\x1b[38;2;{};{};{}m\x1b[48;2;{};{};{}m▀", r1,g1,b1,r2,g2,b2);
        }
        out.push_str("\x1b[0m\n");
    }
    let pct = ((t/90.0)*100.0).min(100.0);
    let barw = w.saturating_sub(34).min(80);
    let fill = (barw as f32 * pct/100.0) as usize;
    let left = if w > title.len() { (w-title.len())/2 } else { 0 };
    let _ = write!(out, "\x1b[0m{}{}  ", " ".repeat(left), title);
    out.push_str("\x1b[38;2;120;220;255m[");
    for i in 0..barw { out.push(if i < fill { '■' } else { '·' }); }
    let _ = write!(out, "] {:04.1}s\x1b[0m", t);
}

#[repr(C)]
struct Winsize { row:u16, col:u16, xp:u16, yp:u16 }
unsafe extern "C" { fn ioctl(fd:i32, req:u64, ...) -> i32; }
fn term_size()->(usize,usize) {
    const TIOCGWINSZ:u64 = 0x5413;
    let mut ws = Winsize{row:0,col:0,xp:0,yp:0};
    unsafe { ioctl(1, TIOCGWINSZ, &mut ws); }
    let c = if ws.col > 20 { ws.col as usize } else { std::env::var("COLUMNS").ok().and_then(|s|s.parse().ok()).unwrap_or(100) };
    let r = if ws.row > 10 { ws.row as usize } else { std::env::var("LINES").ok().and_then(|s|s.parse().ok()).unwrap_or(36) };
    (c, r.saturating_sub(2))
}

struct Term;
impl Term { fn enter()->Self { print!("\x1b[?1049h\x1b[?25l\x1b[2J"); let _=io::stdout().flush(); Self } }
impl Drop for Term { fn drop(&mut self) { print!("\x1b[0m\x1b[?25h\x1b[?1049l"); let _=io::stdout().flush(); } }

fn main() {
    let seconds = std::env::args().nth(1).and_then(|s| s.parse::<f32>().ok()).unwrap_or(90.0).max(1.0);
    let _term = Term::enter();
    let start = Instant::now();
    let mut frame = String::with_capacity(1<<20);
    loop {
        let t = start.elapsed().as_secs_f32();
        if t > seconds { break; }
        let (w,h) = term_size();
        frame.clear();
        render(w,h,t,&mut frame);
        print!("{}", frame);
        let _ = io::stdout().flush();
        sleep(Duration::from_millis(16));
    }
}
