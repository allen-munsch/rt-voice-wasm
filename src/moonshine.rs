//! Moonshine engine — wraps moonshine.cpp via dlopen (avoids link-time
//! conflicts with whisper.cpp's statically-linked ggml and ONNX Runtime).

use crate::engine::SttEngine;

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::Path;

#[repr(C)]
struct MoonshineCtx {
    _private: [u8; 0],
}

type LoadFn = unsafe extern "C" fn(*const c_char) -> *mut MoonshineCtx;
type FreeFn = unsafe extern "C" fn(*mut MoonshineCtx);
type TranscribeFn = unsafe extern "C" fn(*mut MoonshineCtx, *const f32, i32) -> *mut c_char;
type FreeStringFn = unsafe extern "C" fn(*mut c_char);
type LastErrorFn = unsafe extern "C" fn(*mut MoonshineCtx) -> *const c_char;

pub struct MoonshineEngine {
    ctx: *mut MoonshineCtx,
    _lib: libloading::Library,
    _free: FreeFn,
    _free_string: FreeStringFn,
    _transcribe: TranscribeFn,
    _last_error: LastErrorFn,
}

unsafe impl Send for MoonshineEngine {}
unsafe impl Sync for MoonshineEngine {}

impl MoonshineEngine {
    pub fn init_from_dir(path: impl AsRef<Path>) -> Result<Self, String> {
        let lib_path = std::env::current_dir()
            .map_err(|e| format!("cwd: {e}"))?
            .join("build/moonshine/libmoonshine.so");

        // Safety: loading a shared library. We trust the library path.
        let lib = unsafe {
            libloading::Library::new(&lib_path)
                .map_err(|e| format!("dlopen libmoonshine.so: {e}"))?
        };

        let load: libloading::Symbol<LoadFn> = unsafe {
            lib.get(b"moonshine_load")
                .map_err(|e| format!("dlsym moonshine_load: {e}"))?
        };
        let free_fn: FreeFn = unsafe {
            *lib.get::<FreeFn>(b"moonshine_free")
                .map_err(|e| format!("dlsym moonshine_free: {e}"))?
        };
        let transcribe: TranscribeFn = unsafe {
            *lib.get::<TranscribeFn>(b"moonshine_transcribe")
                .map_err(|e| format!("dlsym moonshine_transcribe: {e}"))?
        };
        let free_string: FreeStringFn = unsafe {
            *lib.get::<FreeStringFn>(b"moonshine_free_string")
                .map_err(|e| format!("dlsym moonshine_free_string: {e}"))?
        };
        let last_error: LastErrorFn = unsafe {
            *lib.get::<LastErrorFn>(b"moonshine_last_error")
                .map_err(|e| format!("dlsym moonshine_last_error: {e}"))?
        };

        let c_path = CString::new(
            path.as_ref()
                .to_str()
                .ok_or("model path is not valid UTF-8")?,
        )
        .map_err(|_| "model path contains NUL byte")?;

        let ctx = unsafe { load(c_path.as_ptr()) };
        if ctx.is_null() {
            return Err("moonshine_load returned NULL".into());
        }

        Ok(MoonshineEngine {
            ctx,
            _lib: lib,
            _free: free_fn,
            _free_string: free_string,
            _transcribe: transcribe,
            _last_error: last_error,
        })
    }

    fn last_error(&self) -> String {
        unsafe {
            let ptr = (self._last_error)(self.ctx);
            if ptr.is_null() {
                return String::new();
            }
            CStr::from_ptr(ptr).to_string_lossy().into_owned()
        }
    }
}

impl SttEngine for MoonshineEngine {
    fn transcribe(&self, samples: &[i16]) -> Result<Vec<String>, String> {
        let float_samples: Vec<f32> = samples.iter().map(|&s| s as f32 / 32768.0).collect();

        let text_ptr = unsafe {
            (self._transcribe)(self.ctx, float_samples.as_ptr(), float_samples.len() as i32)
        };

        if text_ptr.is_null() {
            let err = self.last_error();
            return Err(if err.is_empty() {
                "transcription failed".into()
            } else {
                format!("moonshine: {err}")
            });
        }

        let text = unsafe { CStr::from_ptr(text_ptr).to_string_lossy().into_owned() };
        unsafe { (self._free_string)(text_ptr) };

        Ok(vec![text])
    }
}

impl Drop for MoonshineEngine {
    fn drop(&mut self) {
        if !self.ctx.is_null() {
            unsafe { (self._free)(self.ctx) };
        }
    }
}
