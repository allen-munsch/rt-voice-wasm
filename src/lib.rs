pub mod audio;
pub mod stream;
pub mod agent;

#[cfg(not(target_arch = "wasm32"))]
pub mod transport;
#[cfg(not(target_arch = "wasm32"))]
pub mod call;

#[cfg(not(target_arch = "wasm32"))]
#[path = "whisper_host.rs"]
pub mod whisper;

#[cfg(target_arch = "wasm32")]
#[path = "whisper_stub.rs"]
pub mod whisper;

#[cfg(target_arch = "wasm32")]
pub mod wasm;
