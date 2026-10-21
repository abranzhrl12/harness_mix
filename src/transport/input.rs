use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, ReadBuf};

pub(super) const MAX_MESSAGE_BYTES: usize = 128 * 1024;

/// Bound incoming JSON-RPC lines before the SDK's unbounded read_until buffer.
pub(super) struct BoundedLines<R> {
    inner: R,
    current: usize,
    max: usize,
}
impl<R> BoundedLines<R> {
    pub fn new(inner: R, max: usize) -> Self {
        Self {
            inner,
            current: 0,
            max,
        }
    }
}
impl<R: AsyncRead + Unpin> AsyncRead for BoundedLines<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        match Pin::new(&mut this.inner).poll_read(cx, buf) {
            Poll::Ready(Ok(())) => {
                for byte in &buf.filled()[before..] {
                    if *byte == b'\n' {
                        this.current = 0;
                    } else {
                        this.current += 1;
                        if this.current > this.max {
                            return Poll::Ready(Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "MCP message exceeds input limit",
                            )));
                        }
                    }
                }
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}

#[cfg(test)]
mod tests;
