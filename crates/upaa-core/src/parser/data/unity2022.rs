//! Experimental 2022.3 counted-layout adapter, pending real Editor comparison.
//! Shared primitives do NOT establish compatibility of the candidate suffix.
//! Unsupported counted layouts fail explicitly; no byte scanning or CPU fallback.
use super::unity6_structured::Decoder;
use std::sync::{atomic::AtomicBool, Arc};
pub fn adapter(cancel: Arc<AtomicBool>) -> Decoder {
    Decoder::configured(true, false, cancel)
}
