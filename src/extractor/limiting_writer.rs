// SPDX-License-Identifier: GPL-3.0-only
use std::io::{self, Write};

/// Writer that aborts once cumulative bytes exceed `max_bytes`.
pub struct LimitingWriter<W> {
    inner: W,
    written: u64,
    max_bytes: u64,
}

impl<W> LimitingWriter<W> {
    pub fn new(inner: W, max_bytes: u64) -> Self {
        Self {
            inner,
            written: 0,
            max_bytes,
        }
    }

    pub fn written(&self) -> u64 {
        self.written
    }

    pub fn into_inner(self) -> W {
        self.inner
    }
}

impl<W: Write> Write for LimitingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let next = self
            .written
            .checked_add(buf.len() as u64)
            .ok_or_else(|| io::Error::other("extraction size overflow"))?;
        if next > self.max_bytes {
            return Err(io::Error::other(format!(
                "extraction exceeded maximum size of {} bytes",
                self.max_bytes
            )));
        }
        let n = self.inner.write(buf)?;
        self.written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limiting_writer_rejects_over_cap() {
        let mut buf = Vec::new();
        let mut writer = LimitingWriter::new(&mut buf, 4);
        assert!(writer.write_all(b"abcd").is_ok());
        let err = writer.write_all(b"e").unwrap_err();
        assert!(err.to_string().contains("exceeded"));
        assert_eq!(buf, b"abcd");
    }
}
