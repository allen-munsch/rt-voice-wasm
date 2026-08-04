pub mod audio;
pub mod stream;

#[cfg(not(target_arch = "wasm32"))]
#[path = "whisper_host.rs"]
pub mod whisper;

#[cfg(target_arch = "wasm32")]
#[path = "whisper_stub.rs"]
pub mod whisper;

#[cfg(target_arch = "wasm32")]
pub mod wasm;
