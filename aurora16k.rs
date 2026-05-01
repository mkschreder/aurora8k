// aurora16k.rs — GL-accelerated no_std SDF intro.
// Raymarcher runs entirely in a GLSL fragment shader on the GPU.
// Raw RGB24 frames written to stdout for piping to ffplay / ffmpeg.
//
// Build: make aurora16k_standard
// Run:   make run16      (pipes to ffplay for live preview)
//        make record16   (records full 90 s animation to aurora16k.mp4)
//
// Linux x86-64. Requires libEGL and libGL (Mesa or proprietary).
// Context: EGL 1.5 pbuffer + OpenGL 3.3 Core.  No libc in Rust code.
#![no_std]
#![no_main]

mod sys;
use sys::{clock_monotonic, elapsed, write_raw, write_stderr};

// ── Resolution ───────────────────────────────────────────────────────────────
const W:  usize = 640;
const PH: usize = 360;

// glReadPixels writes into this BSS buffer (bottom-to-top row order from GL).
static mut FRAMEBUF: [u8; W * PH * 3] = [0u8; W * PH * 3];
// Flip buffer: rows copied top-to-bottom for stdout (zero ELF cost, BSS).
static mut FLIPBUF:  [u8; W * PH * 3] = [0u8; W * PH * 3];

// ── EGL ───────────────────────────────────────────────────────────────────────
type P = *mut ();  // generic opaque handle for EGL / GL objects

const EGL_NONE:                        i32 = 0x3038;
const EGL_PBUFFER_BIT:                 i32 = 0x0001;
const EGL_SURFACE_TYPE:                i32 = 0x3033;
const EGL_RENDERABLE_TYPE:             i32 = 0x3040;
const EGL_OPENGL_BIT:                  i32 = 0x0008;
const EGL_DEPTH_SIZE:                  i32 = 0x3025;
const EGL_WIDTH:                       i32 = 0x3057;
const EGL_HEIGHT:                      i32 = 0x3056;
const EGL_OPENGL_API:                  u32 = 0x30A2;
const EGL_CONTEXT_MAJOR_VERSION:       i32 = 0x3098;
const EGL_CONTEXT_MINOR_VERSION:       i32 = 0x30FB;
const EGL_CONTEXT_OPENGL_PROFILE_MASK: i32 = 0x30FD;
const EGL_CONTEXT_OPENGL_CORE_PROFILE_BIT: i32 = 0x1;

#[link(name = "EGL")]
extern "C" {
    fn eglGetDisplay(display_id: P) -> P;
    fn eglInitialize(dpy: P, major: *mut i32, minor: *mut i32) -> u32;
    fn eglBindAPI(api: u32) -> u32;
    fn eglChooseConfig(dpy: P, attrs: *const i32, cfgs: *mut P,
                       cfg_size: i32, num: *mut i32) -> u32;
    fn eglCreatePbufferSurface(dpy: P, cfg: P, attrs: *const i32) -> P;
    fn eglCreateContext(dpy: P, cfg: P, share: P, attrs: *const i32) -> P;
    fn eglMakeCurrent(dpy: P, draw: P, read: P, ctx: P) -> u32;
}

// ── OpenGL ────────────────────────────────────────────────────────────────────
const GL_VERTEX_SHADER:   u32 = 0x8B31;
const GL_FRAGMENT_SHADER: u32 = 0x8B30;
const GL_TRIANGLES:       u32 = 0x0004;
const GL_RGB:             u32 = 0x1907;
const GL_UNSIGNED_BYTE:   u32 = 0x1401;

#[link(name = "GL")]
extern "C" {
    fn glViewport(x: i32, y: i32, w: i32, h: i32);
    fn glCreateShader(kind: u32) -> u32;
    fn glShaderSource(sh: u32, n: i32, src: *const *const u8, len: *const i32);
    fn glCompileShader(sh: u32);
    fn glGetShaderiv(sh: u32, pname: u32, params: *mut i32);
    fn glGetShaderInfoLog(sh: u32, max: i32, length: *mut i32, log: *mut u8);
    fn glCreateProgram() -> u32;
    fn glAttachShader(prog: u32, sh: u32);
    fn glLinkProgram(prog: u32);
    fn glUseProgram(prog: u32);
    fn glGetUniformLocation(prog: u32, name: *const u8) -> i32;
    fn glUniform1f(loc: i32, v: f32);
    fn glUniform2f(loc: i32, v0: f32, v1: f32);
    fn glGenVertexArrays(n: i32, arrays: *mut u32);
    fn glBindVertexArray(vao: u32);
    fn glDrawArrays(mode: u32, first: i32, count: i32);
    fn glReadPixels(x: i32, y: i32, w: i32, h: i32, fmt: u32, typ: u32, data: *mut u8);
}

// ── Vertex shader: fullscreen triangle from gl_VertexID, no VBO needed ────────
const VERT: &[u8] = b"#version 330 core
void main(){
const vec2 v[3]=vec2[3](vec2(-1,-1),vec2(3,-1),vec2(-1,3));
gl_Position=vec4(v[gl_VertexID],0,1);}";

// ── Fragment shader: full raymarcher ported to GLSL ───────────────────────────
//
// Uniform T  = animation time in seconds
// Uniform R  = vec2(width, height) — resolution
// Output  O  = rgba colour (sqrt-gamma Reinhard tonemap baked in)
//
// The shader is a line-by-line port of the Rust aurora16k scene.
// Differences from the Rust version:
//   • V struct → vec3 built-in; all dot/length/normalize/cross are built-in.
//   • fast_sin_4(a,b,c,d) → four sin() calls (GPU trig is free).
//   • fast_floor(x)        → floor(x).
//   • pow38/pow32          → pow(max(x,0.), N).
//   • match m { ... }      → chain of if/else if.
//   • MapCtx               → Ctx struct, makeCtx() factory, passed by value.
//   • Hit{d,m}             → vec2(distance, material_id).
//   • Tonemap / grade       → inlined in main(); outputs sqrt(Reinhard) → [0,1].
//   • gl_FragCoord.y is bottom=0, so py = fragCoord.y/R.y*2.-1. matches the
//     Rust convention (py>0 = up).  Rows are written in reverse order by the
//     host so ffmpeg receives the expected top-to-bottom layout.
const FRAG: &[u8] = b"#version 330 core
uniform float T;
uniform vec2 R;
out vec4 O;
const float PI=3.14159265,C3Z=0.71073,S3Z=0.70360;
float hash(float n){return fract(sin(n)*43758.5453);}
float rep(float x,float c){float s=x+.5*c;return s-c*floor(s/c)-.5*c;}
float noise3(vec3 p){return(sin(p.x*1.7+p.z*2.3)+sin(p.x*3.1-p.y*1.9+p.z*.7)*.5+sin(p.x*7.3+p.y*2.1+p.z*1.3)*.25)/1.75;}
vec3 rot_z(vec3 p,float a){float s=sin(a),c=cos(a);return vec3(c*p.x-s*p.y,s*p.x+c*p.y,p.z);}
float sd_box(vec3 p,vec3 b){vec3 q=abs(p)-b;return length(max(q,0.))+min(max(q.x,max(q.y,q.z)),0.);}
float sd_octa(vec3 p,float s){return(abs(p.x)+abs(p.y)+abs(p.z)-s)*.57735027;}
float sd_torus(vec3 p,float r,float t){return length(vec2(length(p.xz)-r,p.y))-t;}
float sd_cyl_y(vec3 p,float r,float h){float dx=length(p.xz)-r,dy=abs(p.y)-h;return min(max(dx,dy),0.)+length(max(vec2(dx,dy),0.));}
float smin(float a,float b,float k){float h=clamp(.5+.5*(b-a)/k,0.,1.);return mix(b,a,h)-k*h*(1.-h);}
float terrain_h(float x,float z){
float flat_=smoothstep(3.,7.5,length(vec2(x,z)));
float p1x=x*.3,p1z=z*.3,p2x=p1x+4.,p2z=p1z+4.;
float wx=(sin(p1x*1.7+p1z*2.3)+sin(p1x*3.1+p1z*.7)*.5+sin(p1x*7.3+p1z*1.3)*.25)/1.75*.6;
float wz=(sin(p2x*1.7+p2z*2.3)+sin(p2x*3.1+p2z*.7)*.5+sin(p2x*7.3+p2z*1.3)*.25)/1.75*.6;
float nx=x+wx,nz=z+wz;
return -1.25+(sin(nx*.21+nz*.18)*.40+sin(nx*.67-nz*.43)*.22+sin((nx-nz)*1.31)*.11+sin(nx*2.3+nz*1.9)*.06+sin(nx*4.7-nz*3.1)*.03)*flat_;}
struct Ctx{float gem_sx,gem_cx,gem_sy,gem_cy,pulse_s,gm2_sx,gm2_cx,gm2_sy,gm2_cy,pulse2_s,r0_sx,r0_cx,r1_sz,r1_cz,r2_sy,r2_cy,r3_sx,r3_cx,floor_s,lp2_y;};
Ctx makeCtx(){Ctx c;
c.gem_sx=sin(T*.37);c.gem_cx=cos(T*.37);c.gem_sy=sin(T*.23);c.gem_cy=cos(T*.23);
c.gm2_sx=sin(-T*.51);c.gm2_cx=cos(-T*.51);c.gm2_sy=sin(T*.44);c.gm2_cy=cos(T*.44);
c.r0_sx=sin(T*.70);c.r0_cx=cos(T*.70);c.r1_sz=sin(-T*.55);c.r1_cz=cos(-T*.55);
c.r2_sy=sin(T*.33);c.r2_cy=cos(T*.33);c.r3_sx=sin(-T*.44);c.r3_cx=cos(-T*.44);
c.pulse_s=sin(T*1.8);c.pulse2_s=sin(T*2.3);c.floor_s=sin(T*.3);c.lp2_y=1.3+.8*sin(T*1.3);
return c;}
vec2 hitp(vec2 h,float d,float m){return d<h.x?vec2(d,m):h;}
vec2 forest_hit(vec3 p0){
vec2 h=vec2(1e9,0.);float r0=length(p0.xz);
if(r0<3.||r0>14.5)return h;
float cell=2.2,gx=floor(p0.x/cell),gz=floor(p0.z/cell);
for(int di=-1;di<=1;di++)for(int dj=-1;dj<=1;dj++){
float cx_=(gx+float(di))*cell,cz_=(gz+float(dj))*cell;
vec2 dd=vec2(p0.x-cx_,p0.z-cz_);
if(dot(dd,dd)>6.5)continue;
float cr=length(vec2(cx_,cz_));
if(cr<3.5||cr>14.2)continue;
float seed=cx_*13.71+cz_*29.31,h0=hash(seed);
if(h0<.35)continue;
float h1=hash(seed+1.),h2=hash(seed+2.);
float tx=cx_+(h1-.5)*cell*.75,tz=cz_+(h2-.5)*cell*.75;
if(length(vec2(tx,tz))<3.8||length(vec2(tx,tz))>13.5)continue;
float tree_ht=1.6+h0,tree_r=.44+h1*.28;
float by_=-1.25+.40*sin(tx*.21+tz*.18)+.22*sin(tx*.67-tz*.43);
vec3 tp=p0-vec3(tx,by_,tz);
if(length(tp.xz)>tree_r+.6)continue;
if(tp.y<-.3||tp.y>tree_ht+.4)continue;
h=hitp(h,sd_cyl_y(tp-vec3(0,tree_ht*.3,0),.05,tree_ht*.3),6.);
h=hitp(h,length(tp)-tree_r*.30,6.);
float wind=.055*sin(T*1.15+tx*.7+tz*.5);
for(int i=0;i<5;i++){float fi=float(i)*.20,fy=tree_ht*(.38+fi),fr=tree_r*(1.05-fi*1.1);
if(fr<=0.)break;float sw=wind*fy*.25;vec3 fq=tp-vec3(sw,fy,sw*.5);
h=hitp(h,length(vec3(fq.x,fq.y*1.55,fq.z))-fr,7.);}}
return h;}
vec2 map_(vec3 p0,Ctx cx){
vec2 h=vec2(1e9,0.);
h=hitp(h,p0.y-terrain_h(p0.x,p0.z),4.);
float a=atan(p0.z,p0.x),rr=length(p0.xz);
if(rr<2.5){
vec3 gx=vec3(p0.x,cx.gem_cx*p0.y-cx.gem_sx*p0.z,cx.gem_sx*p0.y+cx.gem_cx*p0.z);
vec3 p=vec3(cx.gem_cy*gx.x+cx.gem_sy*gx.z,gx.y,-cx.gem_sy*gx.x+cx.gem_cy*gx.z);
h=hitp(h,smin(sd_octa(p,.95+.08*cx.pulse_s),length(p)-.72,.18)+.025*sin(p.x*13.)*sin(p.y*11.+T)*sin(p.z*9.),1.);
vec3 g2x=vec3(p0.x,cx.gm2_cx*p0.y-cx.gm2_sx*p0.z,cx.gm2_sx*p0.y+cx.gm2_cx*p0.z);
vec3 p2=vec3(cx.gm2_cy*g2x.x+cx.gm2_sy*g2x.z,g2x.y,-cx.gm2_sy*g2x.x+cx.gm2_cy*g2x.z);
h=hitp(h,sd_octa(p2,.48+.04*cx.pulse2_s),9.);
vec3 r0q=vec3(p0.x,cx.r0_cx*p0.y-cx.r0_sx*p0.z,cx.r0_sx*p0.y+cx.r0_cx*p0.z);
vec3 t1v=vec3(p0.z,p0.y,-p0.x);
vec3 r1q=vec3(cx.r1_cz*t1v.x-cx.r1_sz*t1v.y,cx.r1_sz*t1v.x+cx.r1_cz*t1v.y,t1v.z);
vec3 t2v=vec3(p0.x,-p0.z,p0.y);
vec3 r2q=vec3(cx.r2_cy*t2v.x+cx.r2_sy*t2v.z,t2v.y,-cx.r2_sy*t2v.x+cx.r2_cy*t2v.z);
vec3 r3b=vec3(C3Z*p0.x-S3Z*p0.y,S3Z*p0.x+C3Z*p0.y,p0.z);
vec3 r3q=vec3(r3b.x,cx.r3_cx*r3b.y-cx.r3_sx*r3b.z,cx.r3_sx*r3b.y+cx.r3_cx*r3b.z);
h=hitp(h,min(sd_torus(r0q,1.18,.035),min(sd_torus(r1q,1.34,.026),min(sd_torus(r2q,1.52,.018),sd_torus(r3q,1.68,.013)))),2.);}
if(rr>1.8&&rr<3.2){float aa=rep(a,2.*PI/8.);vec3 q=vec3(rr-2.35,p0.y+.15,aa*rr);
h=hitp(h,min(sd_cyl_y(q,.07,1.1),min(sd_torus(q+vec3(0,-1.04,0),.11,.025),sd_torus(q+vec3(0,1.04,0),.11,.025))),3.);}
vec3 gv=vec3(rep(p0.x+.3*cx.floor_s,.75),p0.y+1.215,rep(p0.z,.75));
h=hitp(h,min(sd_box(gv,vec3(.20,.018,.018)),sd_box(gv,vec3(.018,.018,.20))),5.);
h=hitp(h,sd_box(vec3(rr-1.90,p0.y+1.215,rep(a,2.*PI/16.)*rr),vec3(.022,.022,.095)),5.);
if(rr>1.2&&rr<2.3){float s12=2.*PI/12.,sid=floor(a/s12+.5),aa12=a-sid*s12,hs=hash(sid*7.);
h=hitp(h,sd_box(rot_z(vec3(rr-1.72,p0.y-.28*sin(hs*6.+T*.5),aa12*rr),.3+hs*.55),vec3(.04,.33,.09)),9.);}
if(rr>2.0&&rr<5.0){float a4=rep(a,PI*.5);vec3 gp=vec3(rr-3.45,p0.y+1.25,a4*rr*1.85);
h=hitp(h,min(min(sd_box(gp+vec3(0,0,-.55),vec3(.10,1.,.10)),sd_box(gp+vec3(0,0,.55),vec3(.10,1.,.10))),sd_box(gp+vec3(0,1.02,0),vec3(.10,.13,.67))),3.);}
if(rr>3.5&&rr<5.8){float s7=2.*PI/7.,sid7=floor(a/s7+.5),a7=a-sid7*s7,ss=sid7*9.3;
h=hitp(h,sd_box(rot_z(vec3(rr-4.75,p0.y+1.25,a7*rr*2.),.08*sin(ss)),vec3(.10,.70+hash(ss)*.30,.08)),3.);}
float pr=length(p0.xz)-2.55,wave=.013*(sin(p0.x*4.2+T*2.1)+sin(p0.z*3.7-T*1.8))+.006*noise3(vec3(p0.x*2.,T*.5,p0.z*2.));
h=hitp(h,max(abs(p0.y+1.228+wave)-.006,abs(pr)-.55),8.);
h=hitp(h,max(abs(p0.y+1.10)-.12,abs(length(p0.xz)-2.55)-.62),3.);
float ar=length(p0.xz);
h=hitp(h,sd_box(vec3(ar,p0.y+1.21,rep(a,2.*PI/8.)*ar),vec3(.72,.035,.32)),3.);
h=hitp(h,sd_cyl_y(vec3(p0.x,p0.y+1.18,p0.z),.28,.07),3.);
if(rr>.8&&rr<1.9){float s5=2.*PI/5.,sid5=floor(a/s5+.5),a5=a-sid5*s5,m5s=sid5*11.7;
h=hitp(h,sd_box(rot_z(vec3(rr-1.25,p0.y+1.25,a5*rr*2.5),.05*sin(m5s)),vec3(.06,.35+hash(m5s)*.20,.05)),9.);}
if(rr>3.&&rr<14.5&&p0.y>-2.5&&p0.y<3.2){vec2 fh=forest_hit(p0);if(fh.x<h.x)h=fh;}
if(rr>9.5){float s6=2.*PI/6.,sid6=floor(a/s6+.5),a6=a-sid6*s6,rs=sid6*17.1,rh=.55+hash(rs)*.65;
vec3 rpp=vec3(rr-11.5,p0.y-(terrain_h(p0.x*(11.5/max(rr,.01)),p0.z*(11.5/max(rr,.01)))+rh),a6*rr);
h=hitp(h,sd_cyl_y(rot_z(rpp,.3*hash(rs+1.)-.15),.12,rh),3.);}
return h;}
vec3 norm_(vec3 p,float m,Ctx cx){
if(m==4.){float e=.04;return normalize(vec3(terrain_h(p.x-e,p.z)-terrain_h(p.x+e,p.z),2.*e,terrain_h(p.x,p.z-e)-terrain_h(p.x,p.z+e)));}
float e=.004;vec3 k1=vec3(1,-1,-1),k2=vec3(-1,-1,1),k3=vec3(-1,1,-1),k4=vec3(1,1,1);
return normalize(k1*map_(p+k1*e,cx).x+k2*map_(p+k2*e,cx).x+k3*map_(p+k3*e,cx).x+k4*map_(p+k4*e,cx).x);}
float shad_(vec3 ro,vec3 rd,Ctx cx){float res=1.,d=.035;
for(int i=0;i<36;i++){float h=map_(ro+rd*d,cx).x;if(h<.002)return 0.;res=min(res,9.*h/d);d+=clamp(h,.025,.28);if(d>8.)break;}
return clamp(res,0.,1.);}
float ao_(vec3 p,vec3 n,float m,Ctx cx){
if(m==4.){float o=0.,sc=1.;for(int i=1;i<=5;i++){float h=float(i)*.10;vec3 sp=p+n*h;o+=max(h-(sp.y-terrain_h(sp.x,sp.z)),0.)*sc;sc*=.5;}return 1.-clamp(o*3.,0.,1.);}
float o=0.,sc=1.;for(int i=1;i<=9;i++){float h=float(i)*.048;o+=max(h-map_(p+n*h,cx).x,0.)*sc;sc*=.57;}
return clamp(1.-o*1.5,0.,1.);}
vec3 pal_(float m,vec3 p){
if(m==1.){float h=p.y*6.+p.x*3.+T,h2=p.z*5.-p.y*4.+T*1.3;return vec3(.85+.15*abs(sin(h)),.30+.30*abs(sin(h*1.5+1.2)),.88+.12*abs(sin(h2)));}
if(m==2.){float s=.8+.2*abs(sin(p.x*11.+p.y*13.));return vec3(.22*s,.90*s,s);}
if(m==3.){float c=.80+.20*noise3(p*2.5),f=.90+.10*noise3(p*6.),v=smoothstep(.55,.45,abs(noise3(vec3(p.x*4.+p.y*3.,p.y*5.,p.z*4.))));float tx=c*f*(1.-v*.35);return vec3(.98*tx,.70*tx,.30*tx+v*.05);}
if(m==4.){float r=length(p.xz),fb=smoothstep(3.5,6.5,r),rn=.80+.20*noise3(p*2.5),mn=.70+.30*noise3(vec3(p.x*4.,p.z*4.,p.y)),g=fb*.14;
return mix(vec3((.07+g*.35)*rn,(.06+g)*rn,(.12+g*.18)*rn),vec3(.06*mn,.18*mn,.08*mn),clamp(fb,0.,1.));}
if(m==5.){float gw=.7+.3*abs(sin(p.x*8.+p.z*6.+T*1.2));return vec3(.10*gw,.78*gw,.90*gw);}
if(m==6.){float g=.75+.25*noise3(vec3(p.y*4.,p.x*3.,p.z*3.));return vec3(.30*g,.17*g,.09*g);}
if(m==7.){float v=.70+.30*noise3(p*5.);return vec3(.07*v,.24*v+.05*abs(sin(p.y*4.+p.x)),.09*v);}
if(m==8.)return vec3(.05,.14,.42);
if(m==9.){float s=.65+.35*abs(sin(p.y*9.+T*1.5+p.x*5.));return vec3(.70*s+.25,.35*s,.95*s);}
return vec3(0.);}
vec3 sky_(vec3 rd){
float y=clamp(rd.y*.5+.5,0.,1.);
vec3 c=vec3(.018,.014,.055)*(1.-y)+vec3(.014,.075,.175)*y;
float u=atan(rd.z,rd.x)*7.,v=(1.-clamp(rd.y,-1.,1.))*(9.*PI*.5);
float id=abs(floor(u)*37.+floor(v)*113.);
c+=vec3(.80,.92,1.)*smoothstep(.995,1.,hash(id+floor(T*.03)))*(.5+.5*abs(sin(T*3.+id)));
float u2=atan(rd.z,rd.x)*19.,v2=(1.-clamp(rd.y,-1.,1.))*(17.*PI*.5);
float cid=abs(floor(u2)*53.+floor(v2)*137.),cm=smoothstep(.82,.97,clamp(dot(rd,normalize(vec3(.6,.7,-.38))),0.,1.));
c+=vec3(.90,.85,1.)*smoothstep(.984,.998,hash(cid+17.3))*cm*(.4+.6*hash(cid));
float ma=atan(rd.z,rd.x);
c+=vec3(.08,.10,.18)*(smoothstep(.18,.04,abs(ma*.6+rd.y*.8)-.04)+smoothstep(.16,.02,abs(ma*.6+rd.y*.8+.3)-.02)*.5)*smoothstep(0.,.25,rd.y);
vec3 md=normalize(vec3(-.5,.65,-.6));float mdd=clamp(dot(rd,md),0.,1.);
c+=vec3(.92,.94,1.)*smoothstep(.9998,1.,mdd)+vec3(.12,.22,.40)*pow(smoothstep(.93,1.,mdd),2.)*.50+vec3(.04,.08,.18)*smoothstep(.70,1.,mdd)*.15;
float ang=atan(rd.z,rd.x),ridge=.065+.040*sin(ang*3.)+.022*sin(ang*9.+T*.04)+.012*sin(ang*23.)+.006*sin(ang*51.+T*.10);
float mtn=smoothstep(ridge+.018,ridge-.010,rd.y);
c=c*(1.-mtn*.87)+vec3(.010,.018,.038)*mtn;
c+=vec3(.25,.30,.55)*smoothstep(.15,-.05,rd.y)*pow(clamp(dot(rd,normalize(vec3(md.x,0,md.z))),0.,1.),2.)*.18;
float ch=smoothstep(.08,.26,rd.y)*(1.-smoothstep(.28,.52,rd.y));
c+=vec3(.07,.09,.16)*pow((.5+.5*sin(ang*7.+T*.06))*(.5+.5*sin(ang*13.-rd.y*9.+T*.04)),2.)*ch;
float hm=smoothstep(.04,.42,rd.y)*(1.-smoothstep(.48,.88,rd.y));
c+=vec3(.04,.65,.38)*(.5+.5*sin(ang*3.2+T*.17))*hm*pow(.5+.5*sin(ang*17.+rd.y*23.+T*1.20),2.)*.28
  +vec3(.55,.08,.92)*(.5+.5*sin(ang*5.1+T*.24+2.))*hm*pow(.5+.5*sin(ang*13.+rd.y*15.+T*.90),2.)*.22
  +vec3(.08,.40,.85)*(.5+.5*sin(ang*7.3+T*.11+4.5))*hm*pow(.5+.5*sin(ang*23.+rd.y*31.+T*1.55),2.)*.7*.18
  +vec3(.80,.30,.60)*(.5+.5*sin(ang*11.+T*.31+1.2))*hm*pow(.5+.5*sin(ang*29.+rd.y*37.+T*2.1),2.)*.6*.15;
c+=vec3(.12,.08,.30)*pow(clamp(dot(rd,normalize(vec3(-.4,.55,.72))),0.,1.),3.)*smoothstep(.12,.40,rd.y)*.25;
return c;}
vec3 shade_(vec3 ro,vec3 rd,Ctx cx){
vec3 sky_col=sky_(rd);float depth=0.;vec3 glow=vec3(0.);float mat=0.;
for(int i=0;i<80;i++){
vec3 p=ro+rd*depth;vec2 h=map_(p,cx);
glow+=vec3(.7,.2,1.)*.0015/(.012+abs(h.x))+vec3(.1,.8,1.)*.0008/(.020+abs(h.x+.05));
float xz2=dot(p.xz,p.xz);
glow+=vec3(.25,.65,1.)*.003/(.018+xz2)*clamp(p.y+1.2,0.,1.)*clamp(2.8-p.y,0.,1.);
glow+=vec3(.60,.65,.90)*.0005/(.06+xz2*.15)*clamp(p.y+.5,0.,1.)*clamp(4.-p.y,0.,1.)*.3;
vec3 fp=p+vec3(.5*sin(T*.17),T*.20,.4*cos(T*.14));
float cell=2.,ix=floor(fp.x/cell),iy=floor(fp.y/cell),iz=floor(fp.z/cell);
float seed=ix*17.+iy*57.+iz*113.;
vec3 pp=vec3(ix*cell+hash(seed+1.)*cell,iy*cell+hash(seed+2.)*cell,iz*cell+hash(seed+3.)*cell);
float fd=length(fp-pp);
glow+=vec3(.40,.95,.55)*.0008/(.018+fd*fd)*smoothstep(9.,2.5,length(p.xz));
glow+=vec3(.06,.09,.20)*.00035*(1.-smoothstep(-.4,.8,p.y));
if(h.x<.0015*(1.+depth*.12)){mat=h.y;break;}
depth+=clamp(h.x*.80,.006,.35);
if(depth>18.||i==79)return sky_col+glow*.55;}
vec3 p=ro+rd*depth,n=norm_(p,mat,cx),base=pal_(mat,p);
vec3 emit=(mat==1.)?base*.20:(mat==2.)?base*.32:(mat==5.)?base*.35:(mat==9.)?base*.25:vec3(0.);
float ss=(mat==1.||mat==9.)?2.2:(mat==2.)?1.8:(mat==8.)?3.5:(mat==6.)?.18:(mat==7.)?.07:(mat==5.)?.8:1.;
vec3 bn=n;
if(mat==4.){float e=.08;float bx=noise3(vec3(p.x+e,p.y,p.z))-noise3(vec3(p.x-e,p.y,p.z));float bz=noise3(vec3(p.x,p.y,p.z+e))-noise3(vec3(p.x,p.y,p.z-e));bn=normalize(n+vec3(bx,0,bz)*.35);}
else if(mat==6.){float e=.06;float bx=noise3(vec3(p.y*2.+e,p.z,p.x))-noise3(vec3(p.y*2.-e,p.z,p.x));float by_=noise3(vec3(p.x,p.y*2.+e,p.z))-noise3(vec3(p.x,p.y*2.-e,p.z));bn=normalize(n+vec3(bx,by_,0)*.30);}
n=bn;
vec3 lp1=vec3(2.6*cx.r0_cx,2.1,2.6*cx.r0_sx),lp2=vec3(-2.,cx.lp2_y,-2.5);
vec3 sky_fill=n.y>0.?sky_(n)*(0.18*n.y):vec3(0.);
vec3 col=base*.06+emit+base*max(n.y,0.)*.05+sky_fill*base;
vec3 lps[2];lps[0]=lp1;lps[1]=lp2;
for(int li=0;li<2;li++){
vec3 l=normalize(lps[li]-p);float dif=max(dot(n,l),0.)*shad_(p+n*.01,l,cx);
vec3 rv=normalize(n*(2.*dot(n,l))-l);float spec=pow(max(dot(rv,-rd),0.),38.);
col+=base*dif+vec3(1.,.95,.8)*spec*ss*(.55+.45*dif);}
float fr=clamp(1.+dot(n,rd),0.,1.),fres=fr*fr*fr,ao_v=ao_(p,n,mat,cx);
col=col*ao_v+vec3(.25,.85,1.)*fres;
vec3 rim=(mat==1.||mat==9.)?vec3(.50,.20,1.)*1.8:(mat==2.)?vec3(.10,.80,1.)*1.5:(mat==7.)?vec3(.05,.50,.15)*1.2:(mat==8.)?vec3(.10,.40,.90)*2.:vec3(.22,.75,1.);
col+=rim*fres*ao_v;
if(mat==7.){float bl=max(-dot(n,normalize(lp1-p)),0.);col+=vec3(.08,.35,.10)*bl*bl*.55*smoothstep(8.,3.,depth);}
if(mat==5.)col+=vec3(.08,.60,.70)*(.4+.6*abs(sin(p.x*12.+p.z*10.+T*1.8)))*.25;
col+=sky_(vec3(0,1,0))*max(n.y,0.)*.06*ao_v;
vec3 fv=normalize(vec3(0,-1.21,0)-p);float fd2=length(vec3(0,-1.21,0)-p);
col+=base*vec3(.10,.78,.90)*max(-dot(n,fv),0.)/(1.+fd2*fd2*.4)*.20;
if(mat==8.){float wf=min(fres+.4,1.);col=col*(1.-wf)+sky_(rd-n*(2.*dot(rd,n)))*wf;col*=(.3+.7*abs(sin(p.x*8.+p.z*7.+T*2.5)));}
if(mat==4.){float rp2=length(p.xz);float ipz=smoothstep(4.,2.5,rp2);if(ipz>.01){float cu=sin(p.x*9.+T*2.2)*sin(p.z*8.-T*1.9)*.5+sin(p.x*6.3+p.z*10.4)*.5;col+=vec3(.06,.18,.45)*max(cu,0.)*.55*ipz;}}
float fog=smoothstep(3.,16.,depth),mist=smoothstep(.5,-.9,p.y)*.42;
return col*(1.-fog)*(1.-mist)+sky_col*fog*(1.-mist)+vec3(.038,.065,.155)*mist+glow*.6;}
void cam_(out vec3 ro,out vec3 f,out vec3 r,out vec3 u){
int seg=int(T*(1./18.));int act=seg%5;float loc=T-float(seg)*18.,k=smoothstep(0.,5.,loc);
float dist,ht,spd;
if(act==0){dist=7.-k*3.;ht=-.2+k*1.7;spd=.07;}
else if(act==1){dist=4.2;ht=1.2+.3*sin(T*.35);spd=.14;}
else if(act==2){dist=2.3+.3*sin(T*.8);ht=.2+.5*sin(T*.6);spd=.40;}
else if(act==3){dist=4.+k*4.5;ht=1.5+k*2.4;spd=.09;}
else{dist=6.5;ht=3.+k*1.8;spd=.06;}
float tt=T*spd;ro=vec3(dist*cos(tt),ht,dist*sin(tt));
float ly=(act==4)?-.28+.12*sin(T*.25):-.08+.18*sin(T*.28);
f=normalize(vec3(0,ly,0)-ro);r=normalize(cross(f,vec3(0,1,0)));u=cross(r,f);}
void main(){
Ctx cx=makeCtx();vec3 ro,f,r,u;cam_(ro,f,r,u);
float asp=R.x/R.y,px=(gl_FragCoord.x/R.x*2.-1.)*asp,py=gl_FragCoord.y/R.y*2.-1.;
vec3 rd=normalize(f*1.35+r*px+u*py);
float vign=clamp(1.-.38*min(px*px+py*py,1.)-.12*max(max(abs(px),abs(py))-.75,0.),0.,1.);
vec3 raw=shade_(ro,rd,cx)*vign;
float prog=clamp(T/90.,0.,1.);
vec3 warm=vec3(1.+prog*.07,1.,1.-prog*.06),lift=vec3(.012,.012,.022);
float lum=dot(raw,vec3(.299,.587,.114)),contrast=1.+.12*(lum-.45);
vec3 gc=(raw+lift)*warm*max(contrast,.3);
O=vec4(sqrt(clamp(gc/(1.+gc),vec3(0.),vec3(1.))),1.);}";

// ── EGL + GL setup ─────────────────────────────────────────────────────────────

unsafe fn compile(src: &[u8], kind: u32) -> u32 {
    let sh = glCreateShader(kind);
    let ptr = src.as_ptr();
    let len = src.len() as i32;
    glShaderSource(sh, 1, &ptr, &len);
    glCompileShader(sh);

    // Surface compile errors to stderr so failures are visible
    let mut ok: i32 = 0;
    glGetShaderiv(sh, 0x8B81 /*GL_COMPILE_STATUS*/, &mut ok);
    if ok == 0 {
        let mut log = [0u8; 1024];
        let mut n: i32 = 0;
        glGetShaderInfoLog(sh, 1024, &mut n, log.as_mut_ptr());
        write_stderr(log.as_ptr(), n as usize);
        write_stderr(b"\n".as_ptr(), 1);
    }
    sh
}

// ── Main render loop ───────────────────────────────────────────────────────────

pub(crate) fn run(seconds: f32, record: bool) {
    unsafe {
        // ── EGL: pbuffer context, no window needed ─────────────────────────────
        let dpy = eglGetDisplay(core::ptr::null_mut());   // EGL_DEFAULT_DISPLAY
        eglInitialize(dpy, core::ptr::null_mut(), core::ptr::null_mut());
        eglBindAPI(EGL_OPENGL_API);

        let cfg_attrs: [i32; 9] = [
            EGL_SURFACE_TYPE,    EGL_PBUFFER_BIT,
            EGL_RENDERABLE_TYPE, EGL_OPENGL_BIT,
            EGL_DEPTH_SIZE,      0,
            EGL_NONE, 0, 0,
        ];
        let mut cfg: P = core::ptr::null_mut();
        let mut ncfg: i32 = 0;
        eglChooseConfig(dpy, cfg_attrs.as_ptr(), &mut cfg, 1, &mut ncfg);

        let pb_attrs: [i32; 5] = [
            EGL_WIDTH,  W  as i32,
            EGL_HEIGHT, PH as i32,
            EGL_NONE,
        ];
        let surface = eglCreatePbufferSurface(dpy, cfg, pb_attrs.as_ptr());

        let ctx_attrs: [i32; 7] = [
            EGL_CONTEXT_MAJOR_VERSION,       3,
            EGL_CONTEXT_MINOR_VERSION,       3,
            EGL_CONTEXT_OPENGL_PROFILE_MASK, EGL_CONTEXT_OPENGL_CORE_PROFILE_BIT,
            EGL_NONE,
        ];
        let ctx = eglCreateContext(dpy, cfg, core::ptr::null_mut(), ctx_attrs.as_ptr());
        eglMakeCurrent(dpy, surface, surface, ctx);

        // ── Shader program ─────────────────────────────────────────────────────
        let vert = compile(VERT, GL_VERTEX_SHADER);
        let frag = compile(FRAG, GL_FRAGMENT_SHADER);
        let prog = glCreateProgram();
        glAttachShader(prog, vert);
        glAttachShader(prog, frag);
        glLinkProgram(prog);
        glUseProgram(prog);

        // VAO required by OpenGL 3.3 Core even without vertex data
        let mut vao: u32 = 0;
        glGenVertexArrays(1, &mut vao);
        glBindVertexArray(vao);

        glViewport(0, 0, W as i32, PH as i32);

        let t_loc = glGetUniformLocation(prog, b"T\0".as_ptr());
        let r_loc = glGetUniformLocation(prog, b"R\0".as_ptr());
        glUniform2f(r_loc, W as f32, PH as f32);

        // ── Render loop ────────────────────────────────────────────────────────
        let start = clock_monotonic();
        let mut frame_t = 0.0_f32;

        loop {
            let t = if record { frame_t } else { elapsed(&start) };
            if t > seconds { break; }

            glUniform1f(t_loc, t);
            glDrawArrays(GL_TRIANGLES, 0, 3);

            // GL row order is bottom-to-top; read into FRAMEBUF
            glReadPixels(0, 0, W as i32, PH as i32,
                         GL_RGB, GL_UNSIGNED_BYTE, FRAMEBUF.as_mut_ptr());

            // Flip rows into FLIPBUF (1 memcpy per row) then write once —
            // reduces 180 write syscalls/frame to a single 172,800-byte write.
            let row = W * 3;
            for y in 0..PH {
                core::ptr::copy_nonoverlapping(
                    FRAMEBUF.as_ptr().add((PH - 1 - y) * row),
                    FLIPBUF.as_mut_ptr().add(y * row),
                    row,
                );
            }
            write_raw(FLIPBUF.as_ptr(), W * PH * 3);

            if record {
                write_stderr(b".".as_ptr(), 1);
                frame_t += 1.0 / 30.0;
            }
        }
    }
}
