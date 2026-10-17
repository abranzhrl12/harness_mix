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
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;
    #[tokio::test]
    async fn permits_separate_lines_but_bounds_a_line_across_reads() {
        let mut normal = BoundedLines::new(&b"abcd\nx\n"[..], 4);
        let mut output = Vec::new();
        normal.read_to_end(&mut output).await.unwrap();
        assert_eq!(output, b"abcd\nx\n");
        let mut oversized = BoundedLines::new(&b"abcdX\n"[..], 4);
        let mut pair = [0; 2];
        oversized.read_exact(&mut pair).await.unwrap();
        oversized.read_exact(&mut pair).await.unwrap();
        assert_eq!(
            oversized.read_exact(&mut pair).await.unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
