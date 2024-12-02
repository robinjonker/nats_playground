// src/messages.rs
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct KeyPress {
    #[prost(string, tag = "1")]
    pub key: String,
}

// STREAM_SUBJECTS
pub const KEY_PRESS: &str = "key-press-7";
