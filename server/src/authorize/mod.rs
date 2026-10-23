//! `lib/authorize/`: signing a desktop client in without ever showing it a
//! password (the RFC 8252 loopback flow; see [`loopback`]), plus the
//! server-side pickup next to it (not upstream; see [`pickup`]).

pub mod loopback;
pub mod pickup;
