use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use tokio::io::{AsyncRead, ReadBuf};

/// Async reader that bounds each newline-delimited MCP message.
pub struct LimitedLineReader<R> {
    inner: R,
    max_message_bytes: usize,
    current_line_bytes: usize,
}

impl<R> LimitedLineReader<R> {
    pub fn new(inner: R, max_message_bytes: usize) -> Self {
        Self {
            inner,
            max_message_bytes,
            current_line_bytes: 0,
        }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for LimitedLineReader<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let mut scratch = [0_u8; 8192];
        let read_limit = buffer.remaining().min(scratch.len());
        if read_limit == 0 {
            return Poll::Ready(Ok(()));
        }
        let mut temporary = ReadBuf::new(&mut scratch[..read_limit]);
        match Pin::new(&mut self.inner).poll_read(context, &mut temporary) {
            Poll::Ready(Ok(())) => {
                let mut current_line_bytes = self.current_line_bytes;
                for byte in temporary.filled() {
                    if *byte == b'\n' {
                        current_line_bytes = 0;
                    } else {
                        current_line_bytes = current_line_bytes.saturating_add(1);
                        if current_line_bytes > self.max_message_bytes {
                            return Poll::Ready(Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "STDIO MCP message exceeds configured transport limit",
                            )));
                        }
                    }
                }
                self.current_line_bytes = current_line_bytes;
                buffer.put_slice(temporary.filled());
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LimitedLineReader;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

    #[tokio::test]
    async fn bounds_each_line_and_allows_multiple_valid_messages() {
        let input = BufReader::new(LimitedLineReader::new(&b"1234\n12\n"[..], 4));
        let mut lines = input.lines();
        assert_eq!(lines.next_line().await.unwrap().as_deref(), Some("1234"));
        assert_eq!(lines.next_line().await.unwrap().as_deref(), Some("12"));
    }

    #[tokio::test]
    async fn rejects_a_line_over_the_configured_limit() {
        let mut input = LimitedLineReader::new(&b"12345\n"[..], 4);
        let mut output = Vec::new();
        let error = input.read_to_end(&mut output).await.unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }
}
