pub mod audio;
pub mod augment;
#[cfg(not(target_arch = "wasm32"))]
pub mod config;
pub mod stream;
pub mod agent;
pub mod engine;

#[cfg(not(target_arch = "wasm32"))]
pub mod transport;
#[cfg(not(target_arch = "wasm32"))]
pub mod call;

#[cfg(all(not(target_arch = "wasm32"), feature = "mic-capture"))]
pub mod capture;

#[cfg(not(target_arch = "wasm32"))]
#[path = "whisper_host.rs"]
pub mod whisper;

#[cfg(target_arch = "wasm32")]
#[path = "whisper_stub.rs"]
pub mod whisper;

#[cfg(not(target_arch = "wasm32"))]
pub mod parakeet;

#[cfg(not(target_arch = "wasm32"))]
pub mod moonshine;

#[cfg(target_arch = "wasm32")]
pub mod wasm;
