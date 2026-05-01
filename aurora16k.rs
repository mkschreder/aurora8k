#![no_std]
#![no_main]

mod sys;
use sys::{clock_monotonic, elapsed, write_raw, write_stderr};

const W:  usize = 640;
const PH: usize = 360;

static mut FRAMEBUF: [u8; W * PH * 3] = [0u8; W * PH * 3];
static mut FLIPBUF:  [u8; W * PH * 3] = [0u8; W * PH * 3];

// ── EGL ───────────────────────────────────────────────────────────────────────
type P = *mut ();

const EGL_PBUFFER_BIT:                 i32 = 0x0001;
const EGL_SURFACE_TYPE:                i32 = 0x3033;
const EGL_RENDERABLE_TYPE:             i32 = 0x3040;
const EGL_OPENGL_BIT:                  i32 = 0x0008;
const EGL_DEPTH_SIZE:                  i32 = 0x3025;
const EGL_WIDTH:                       i32 = 0x3057;
const EGL_HEIGHT:                      i32 = 0x3056;
const EGL_NONE:                        i32 = 0x3038;
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

const VERT: &[u8] = include_bytes!("aurora16k.vert");
const FRAG: &[u8] = include_bytes!("aurora16k.frag");

unsafe fn compile(src: &[u8], kind: u32) -> u32 {
    let sh = glCreateShader(kind);
    let ptr = src.as_ptr();
    let len = src.len() as i32;
    glShaderSource(sh, 1, &ptr, &len);
    glCompileShader(sh);
    sh
}

pub(crate) fn run(seconds: f32, record: bool) {
    unsafe {
        let dpy = eglGetDisplay(core::ptr::null_mut());
        eglInitialize(dpy, core::ptr::null_mut(), core::ptr::null_mut());
        eglBindAPI(EGL_OPENGL_API);

        let cfg_attrs: [i32; 7] = [
            EGL_SURFACE_TYPE,    EGL_PBUFFER_BIT,
            EGL_RENDERABLE_TYPE, EGL_OPENGL_BIT,
            EGL_DEPTH_SIZE,      0,
            EGL_NONE,
        ];
        let mut cfg: P = core::ptr::null_mut();
        let mut ncfg: i32 = 0;
        eglChooseConfig(dpy, cfg_attrs.as_ptr(), &mut cfg, 1, &mut ncfg);

        let pb_attrs: [i32; 5] = [EGL_WIDTH, W as i32, EGL_HEIGHT, PH as i32, EGL_NONE];
        let surface = eglCreatePbufferSurface(dpy, cfg, pb_attrs.as_ptr());

        let ctx_attrs: [i32; 7] = [
            EGL_CONTEXT_MAJOR_VERSION,       3,
            EGL_CONTEXT_MINOR_VERSION,       3,
            EGL_CONTEXT_OPENGL_PROFILE_MASK, EGL_CONTEXT_OPENGL_CORE_PROFILE_BIT,
            EGL_NONE,
        ];
        let ctx = eglCreateContext(dpy, cfg, core::ptr::null_mut(), ctx_attrs.as_ptr());
        eglMakeCurrent(dpy, surface, surface, ctx);

        let vert = compile(VERT, GL_VERTEX_SHADER);
        let frag = compile(FRAG, GL_FRAGMENT_SHADER);
        let prog = glCreateProgram();
        glAttachShader(prog, vert);
        glAttachShader(prog, frag);
        glLinkProgram(prog);
        glUseProgram(prog);

        let mut vao: u32 = 0;
        glGenVertexArrays(1, &mut vao);
        glBindVertexArray(vao);
        glViewport(0, 0, W as i32, PH as i32);

        let t_loc = glGetUniformLocation(prog, b"T\0".as_ptr());
        let r_loc = glGetUniformLocation(prog, b"R\0".as_ptr());
        glUniform2f(r_loc, W as f32, PH as f32);

        let start = clock_monotonic();
        let mut frame_t = 0.0_f32;

        loop {
            let t = if record { frame_t } else { elapsed(&start) };
            if t > seconds { break; }

            glUniform1f(t_loc, t);
            glDrawArrays(GL_TRIANGLES, 0, 3);

            glReadPixels(0, 0, W as i32, PH as i32,
                         GL_RGB, GL_UNSIGNED_BYTE, FRAMEBUF.as_mut_ptr());

            // glReadPixels is bottom-to-top; flip rows before writing to stdout.
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
