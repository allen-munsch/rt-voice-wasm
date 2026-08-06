//! Parakeet engine — wraps parakeet.cpp via its flat C API.
//!
//! parakeet.cpp is built as a shared library (libparakeet.so) with only
//! `parakeet_capi_*` symbols exported, so its ggml internals stay hidden.

use crate::engine::SttEngine;

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::Path;

#[repr(C)]
struct ParakeetCtx {
    _private: [u8; 0],
}

extern "C" {
    fn parakeet_capi_load(path: *const c_char) -> *mut ParakeetCtx;
    fn parakeet_capi_free(ctx: *mut ParakeetCtx);
    fn parakeet_capi_transcribe_pcm(
        ctx: *mut ParakeetCtx,
        samples: *const f32,
        n_samples: i32,
        sample_rate: i32,
        decoder: i32,
    ) -> *mut c_char;
    fn parakeet_capi_free_string(s: *mut c_char);
    fn parakeet_capi_last_error(ctx: *mut ParakeetCtx) -> *const c_char;
}

pub struct ParakeetEngine {
    ctx: *mut ParakeetCtx,
}

unsafe impl Send for ParakeetEngine {}
unsafe impl Sync for ParakeetEngine {}

impl ParakeetEngine {
    pub fn init_from_file(path: impl AsRef<Path>) -> Result<Self, String> {
        let c_path = CString::new(
            path.as_ref()
                .to_str()
                .ok_or("model path is not valid UTF-8")?,
        )
        .map_err(|_| "model path contains NUL byte")?;

        let ctx = unsafe { parakeet_capi_load(c_path.as_ptr()) };
        if ctx.is_null() {
            return Err("parakeet_capi_load returned NULL — check model path".into());
        }
        Ok(ParakeetEngine { ctx })
    }

    fn last_error(&self) -> String {
        unsafe {
            let ptr = parakeet_capi_last_error(self.ctx);
            if ptr.is_null() {
                return String::new();
            }
            CStr::from_ptr(ptr).to_string_lossy().into_owned()
        }
    }
}

impl SttEngine for ParakeetEngine {
    fn transcribe(&self, samples: &[i16]) -> Result<Vec<String>, String> {
        // Parakeet PCM API takes float samples
        let float_samples: Vec<f32> = samples.iter().map(|&s| s as f32 / 32768.0).collect();

        let text_ptr = unsafe {
            parakeet_capi_transcribe_pcm(
                self.ctx,
                float_samples.as_ptr(),
                float_samples.len() as i32,
                16000, // sample rate
                0,     // default decoder
            )
        };

        if text_ptr.is_null() {
            let err = self.last_error();
            return Err(if err.is_empty() {
                "transcription failed".into()
            } else {
                format!("parakeet: {err}")
            });
        }

        let text = unsafe { CStr::from_ptr(text_ptr).to_string_lossy().into_owned() };
        unsafe { parakeet_capi_free_string(text_ptr) };

        Ok(vec![text])
    }
}

impl Drop for ParakeetEngine {
    fn drop(&mut self) {
        if !self.ctx.is_null() {
            unsafe { parakeet_capi_free(self.ctx) };
        }
    }
}
