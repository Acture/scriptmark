//! Entry-point adapters.
//!
//! Each adapter turns one source's material into the same [`crate::models::AssignmentInput`].
//! There is deliberately no `InputSource` trait: the ticket asks for a common *output*
//! interface, and the shared type plus its validator is that interface. A one-method trait
//! would additionally force the async Canvas loader (P-670) and the synchronous local scan
//! into one shape, which buys nothing.
//!
//! - Local directories: [`crate::discovery::load_local_input`]
//! - Canvas payloads: [`canvas::normalize`]

pub mod canvas;
