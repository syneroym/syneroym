//! Peer-initiated stop-sending detection, abstracted over transport.
//!
//! Iroh's underlying QUIC stream can be told by the peer to stop receiving
//! (`STOP_SENDING`) independently of the peer closing its own read/send
//! halves or the connection outright -- see `SendStream::stopped()`. This
//! is the one dead-subscriber signal `messaging/subscribe`'s
//! read-until-EOF detection can't see (task.md's "Dead-subscriber
//! cleanup"). WebRTC has no equivalent primitive, so its `stop_signal()`
//! never resolves and that transport relies solely on read-EOF/connection-
//! close detection, as it already did.

use std::{future::Future, pin::Pin};

pub trait StopSignal {
    /// A future that resolves once the peer signals it no longer wants
    /// data pushed on this stream's send side. Never resolves on
    /// transports with no such signal.
    fn stop_signal(&self) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(std::future::pending())
    }
}
